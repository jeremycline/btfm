use tracing::{debug, info, info_span, warn, Instrument};

use tokio::sync::mpsc;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::protocol::Message;
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
                let ws_endpoint = self.websocket_endpoint.to_string();
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
    ws_endpoint: String,
    api_key: Vec<u8>,
    mut audio: mpsc::Receiver<Vec<i16>>,
    respond_to: mpsc::Sender<String>,
) {
    let auth = httparse::Header {
        name: "Authorization",
        value: &api_key,
    };
    let mut headers = [auth];
    let mut request = httparse::Request::new(&mut headers);
    request.method = Some("GET");
    request.path = Some(&ws_endpoint);
    request.version = Some(2);

    if let Ok((websocket, _)) = tokio_tungstenite::connect_async(request).await {
        let (mut writer, mut reader) = websocket.split();

        // Audio goes out
        let writer_span = info_span!("websocket_writer");
        tokio::spawn(
            async move {
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
                info!("")
            }
            .instrument(writer_span),
        );

        // Text comes in
        let reader_span = info_span!("websocket_reader");
        tokio::spawn(
            async move {
                while let Some(message) = reader.next().await {
                    match message {
                        Ok(Message::Text(text)) => {
                            if let Ok(response) = serde_json::from_str::<serde_json::Value>(&text) {
                                let transcript =
                                    &response["channel"]["alternatives"][0]["transcript"];
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
                            return reason;
                        }
                        _ => {
                            warn!("Unhandled WebSocket message: {:?}", message);
                            break;
                        }
                    }
                }
                None
            }
            .instrument(reader_span),
        );
    } else {
        warn!("Failed to connect to Deepgram endpoint");
    }
}
