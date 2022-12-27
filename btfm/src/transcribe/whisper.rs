use futures::stream::{SplitSink, SplitStream};
use tracing::{error, info, instrument, warn, Instrument};

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

use crate::transcode::whisper_transcode;

use super::TranscriptionRequest;

pub struct TranscriberWorker {
    receiver: mpsc::Receiver<TranscriptionRequest>,
    websocket_endpoint: Url,
}

impl TranscriberWorker {
    pub fn new(receiver: mpsc::Receiver<TranscriptionRequest>, websocket_endpoint: Url) -> Self {
        TranscriberWorker {
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
                let ws_endpoint = self.websocket_endpoint.clone();
                let handler = handle_websocket(ws_endpoint, audio, respond_to).instrument(span);
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
    audio: mpsc::Receiver<Vec<i16>>,
    respond_to: mpsc::Sender<String>,
) {
    let request = http::Request::builder()
        .method("GET")
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
        let (writer, reader) = websocket.split();
        tokio::spawn(ws_reader(reader, respond_to).in_current_span());
        tokio::spawn(ws_writer(writer, audio).in_current_span());
    } else {
        warn!("Failed to connect to Whisper endpoint");
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
    let mut bin = Vec::new();
    while let Some(chunk) = audio.recv().await {
        for sample in chunk.into_iter() {
            bin.append(&mut sample.to_le_bytes().to_vec());
        }
    }

    let bin = whisper_transcode(bin).await;

    tracing::trace!("Sending chunk of {} bytes", bin.len());
    if let Err(e) = writer.send(Message::binary(bin)).await {
        warn!(error = ?e, "Failed to send audio data to Whisper API");
        return;
    }

    if let Err(e) = writer.send(Message::binary(vec![])).await {
        warn!(error = ?e, "Failed to send final empty frame to Whisper API");
    }
    if let Err(e) = writer.flush().await {
        warn!(error = ?e, "Failed to flush the write side of the WebSocket");
    }
}
