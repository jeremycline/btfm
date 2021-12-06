use serde::Deserialize;
use tracing::{warn, Instrument};

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

#[derive(Deserialize)]
struct Response {
    results: Results,
}

#[derive(Deserialize)]
struct Results {
    channels: Vec<Channels>,
}

#[derive(Deserialize)]
struct Channels {
    alternatives: Vec<Alternatives>,
}

#[derive(Deserialize)]
struct Alternatives {
    transcript: String,
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
            "interum_results=false&encoding=linear16&sample_rate=48000&channels=2",
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
                mut audio,
                respond_to,
                span,
            } => {
                let api_key = self.api_key.clone().into_bytes();
                let ws_endpoint = self.websocket_endpoint.to_string();
                tokio::spawn(
                    async move {
                        let auth = httparse::Header {
                            name: "Authorization",
                            value: &api_key,
                        };
                        let mut headers = [auth];
                        let mut request = httparse::Request::new(&mut headers);
                        request.method = Some("GET");
                        request.path = Some(&ws_endpoint);
                        request.version = Some(2);

                        if let Ok((websocket, _)) = tokio_tungstenite::connect_async(request).await
                        {
                            let (mut writer, mut reader) = websocket.split();

                            // Text comes in
                            let handle = tokio::spawn(
                                async move {
                                    while let Some(message) = reader.next().await {
                                        match message {
                                            Ok(Message::Text(text)) => {
                                                if let Ok(transcript) =
                                                    serde_json::from_str::<Response>(&text)
                                                {
                                                    let _ = respond_to
                                                        .send(
                                                            transcript.results.channels[0]
                                                                .alternatives[0]
                                                                .transcript
                                                                .clone(),
                                                        )
                                                        .await
                                                        .expect("Unable to send transcribed audio");
                                                }
                                            }
                                            _ => warn!("{:?}", message),
                                        }
                                    }
                                }
                                .instrument(tracing::Span::current()),
                            );

                            // Audio goes out
                            while let Some(chunk) = audio.recv().await {
                                let mut bin = Vec::new();
                                for sample in chunk.into_iter() {
                                    bin.append(&mut sample.to_be_bytes().to_vec());
                                }
                                tracing::trace!("Sending chunk of {} bytes", bin.len());
                                writer
                                    .send(Message::binary(bin))
                                    .await
                                    .expect("Failed to transmit audio chunk");
                            }
                            writer
                                .send(Message::binary(vec![]))
                                .await
                                .expect("Failed to transmit empty frame");
                            handle.await.expect("reader failed");
                        } else {
                            warn!("Failed to connect to Deepgram endpoint");
                        }
                    }
                    .instrument(span),
                );
            }
        }
    }

    pub async fn run(&mut self) {
        while let Some(request) = self.receiver.recv().await {
            self.transcribe(request);
        }
    }
}
