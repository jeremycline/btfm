use log::warn;
use serde::Deserialize;

use tokio::sync::mpsc;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::protocol::Message;

use super::TranscriptionRequest;

pub struct TranscriberWorker {
    api_key: String,
    receiver: mpsc::Receiver<TranscriptionRequest>,
    http_client: reqwest::Client,
    http_endpoint: String,
    ws_endpoint: String,
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
        api_endpoint: String,
    ) -> Self {
        let http_client = reqwest::Client::new();
        TranscriberWorker {
            api_key,
            receiver,
            http_client,
            http_endpoint: format!("https://{}/v1/listen", &api_endpoint),
            ws_endpoint: format!("wss://{}/v1/listen", &api_endpoint),
        }
    }

    fn transcribe(&mut self, request: TranscriptionRequest) {
        match request {
            TranscriptionRequest::PlainText { audio, respond_to } => {
                let endpoint = self.http_endpoint.clone();
                let client = self.http_client.clone();
                let api_key = self.api_key.clone();

                tokio::spawn(async move {
                    let response = client
                        .post(endpoint)
                        .header("Authorization", format!("Token {}", api_key))
                        .header("Content-Type", "audio/wav")
                        .body(crate::transcode::wrap_pcm(audio))
                        .send()
                        .await;
                    match response {
                        Ok(response) => {
                            if response.status() == reqwest::StatusCode::OK {
                                let transcript = response.json::<Response>().await.unwrap();
                                let _ = respond_to.send(
                                    transcript.results.channels[0].alternatives[0]
                                        .transcript
                                        .clone(),
                                );
                            } else {
                                warn!(
                                    "Reponse failed ({}): {}",
                                    response.status(),
                                    response.text().await.unwrap()
                                );
                            }
                        }
                        Err(e) => {
                            let _ = respond_to.send("".to_string());
                            warn!("failed to send request to Deepgram: {:?}", e)
                        }
                    }
                });
            }
            TranscriptionRequest::Stream {
                mut audio,
                respond_to,
            } => {
                let mut ws_endpoint = url::Url::parse(&self.ws_endpoint)
                    .expect("Deepgram websocket endpoint is invalid");
                let api_key = self.api_key.clone().into_bytes();
                // Songbird decodes the Opus audio to signed 16 bit, 48khz, stereo audio
                ws_endpoint.set_query(Some("encoding=linear16&sample_rate=48000&channels=2"));

                tokio::spawn(async move {
                    let auth = httparse::Header {
                        name: "Authorization",
                        value: &api_key,
                    };
                    let mut headers = [auth];
                    let mut request = httparse::Request::new(&mut headers);
                    request.method = Some("GET");
                    request.path = Some(ws_endpoint.as_str());
                    request.version = Some(2);

                    if let Ok((websocket, _)) = tokio_tungstenite::connect_async(request).await {
                        let (mut writer, mut reader) = websocket.split();

                        // Audio goes out
                        tokio::spawn(async move {
                            while let Some(chunk) = audio.recv().await {
                                let mut bin = Vec::new();
                                for sample in chunk.into_iter() {
                                    bin.append(&mut sample.to_be_bytes().to_vec());
                                }
                                writer
                                    .send(Message::binary(bin))
                                    .await
                                    .expect("Failed to transmit audio chunk");
                            }
                        });

                        // Text comes in
                        while let Some(message) = reader.next().await {
                            match message {
                                Ok(Message::Text(text)) => {
                                    if let Ok(transcript) = serde_json::from_str::<Response>(&text)
                                    {
                                        let _ = respond_to
                                            .send(
                                                transcript.results.channels[0].alternatives[0]
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
                    } else {
                        warn!("Failed to connect to Deepgram endpoint");
                    }
                });
            }
        }
    }

    pub async fn run(&mut self) {
        while let Some(request) = self.receiver.recv().await {
            self.transcribe(request);
        }
    }
}
