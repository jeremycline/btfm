use futures::stream::{SplitSink, SplitStream};
use tracing::{debug, error, info, instrument, warn, Instrument};

use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc,
};

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{
    tungstenite::{handshake::client::generate_key, protocol::Message},
    WebSocketStream,
};
use url::Url;

use super::TranscriptionRequest;

pub struct TranscriberWorker {
    api_key: String,
    receiver: mpsc::Receiver<TranscriptionRequest>,
    websocket_endpoint: Url,
}

impl TranscriberWorker {
    pub fn new(
        receiver: mpsc::Receiver<TranscriptionRequest>,
        api_key: String,
        websocket_endpoint: Url,
    ) -> Self {
        // Songbird decodes the Opus audio to signed 16 bit, 48khz, stereo audio
        let mut websocket_endpoint = websocket_endpoint;
        websocket_endpoint.set_query(Some(
            "interim_results=false&encoding=linear16&sample_rate=48000&channels=2",
        ));
        TranscriberWorker {
            api_key,
            receiver,
            websocket_endpoint,
        }
    }

    fn transcribe(&mut self, request: TranscriptionRequest) {
        match request {
            TranscriptionRequest::Stream {
                audio,
                respond_to,
                span,
            } => {
                let api_key = self.api_key.clone().into_bytes();
                let ws_endpoint = self.websocket_endpoint.clone();
                let handler =
                    handle_websocket(ws_endpoint, api_key, audio, respond_to).instrument(span);
                tokio::spawn(handler);
            }
        }
    }

    pub async fn run(&mut self) {
        while let Some(request) = self.receiver.recv().await {
            self.transcribe(request);
        }
    }
}

async fn handle_websocket(
    ws_endpoint: Url,
    api_key: Vec<u8>,
    audio: mpsc::Receiver<Vec<i16>>,
    respond_to: mpsc::Sender<String>,
) {
    let request = http::Request::builder()
        .method("GET")
        .header(hyper::http::header::AUTHORIZATION, api_key)
        .header(
            "Host",
            ws_endpoint
                .host()
                .expect("WebSocket endpoint must contain a host.")
                .to_string(),
        )
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_key())
        .uri(ws_endpoint.to_string())
        .body(())
        .expect("Invalid WebSocket endpoint");

    if let Ok((websocket, _)) = tokio_tungstenite::connect_async(request).await {
        info!("Successfully opened WebSocket to the Deepgram API");
        let (writer, reader) = websocket.split();
        tokio::spawn(ws_reader(reader, respond_to).in_current_span());
        tokio::spawn(ws_writer(writer, audio).in_current_span());
    } else {
        warn!("Failed to connect to Deepgram endpoint");
    }
}

#[instrument(skip_all)]
async fn ws_reader<S>(mut reader: SplitStream<WebSocketStream<S>>, respond_to: mpsc::Sender<String>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    while let Some(message) = reader.next().await {
        match message {
            Ok(Message::Text(text)) => {
                if let Ok(response) = serde_json::from_str::<serde_json::Value>(&text) {
                    let transcript = &response["channel"]["alternatives"][0]["transcript"];
                    if transcript.is_null() {
                        continue;
                    }

                    let text = transcript.to_string();
                    debug!("Parsed Deepgram response: {}", &text);
                    if respond_to.send(text).await.is_err() {
                        warn!("Unable to send transcribed audio; sender closed");
                    }
                }
            }
            Ok(Message::Close(reason)) => {
                info!("Server closed the websocket: {:?}", &reason);
            }
            _ => {
                error!("Unhandled WebSocket message: {:?}", message);
                break;
            }
        }
    }
}

#[instrument(skip_all)]
async fn ws_writer<S>(
    mut writer: SplitSink<WebSocketStream<S>, Message>,
    mut audio: mpsc::Receiver<Vec<i16>>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    while let Some(chunk) = audio.recv().await {
        let mut bin = Vec::new();
        for sample in chunk.into_iter() {
            bin.append(&mut sample.to_le_bytes().to_vec());
        }
        tracing::trace!("Sending chunk of {} bytes", bin.len());
        if let Err(e) = writer.send(Message::binary(bin)).await {
            warn!(error = ?e, "Failed to send audio data to Deepgram API");
            return;
        }
    }
    if let Err(e) = writer.send(Message::binary(vec![])).await {
        warn!(error = ?e, "Failed to send final empty frame to Deepgram API");
    }
    if let Err(e) = writer.flush().await {
        warn!(error = ?e, "Failed to flush the write side of the WebSocket");
    }
}
