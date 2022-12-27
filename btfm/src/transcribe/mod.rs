/// Handles the transcription of audio to text.
///
/// The behaviour of the transcription worker depends on what backend is
/// being used to transcribe the audio (DeepSpeech's CPU build, CUDA build, or some
/// third-party service).
use tokio::sync::mpsc;

use crate::config::Config;
use crate::Backend;

mod deepgram;
mod whisper;

#[derive(Debug)]
pub enum TranscriptionRequest {
    Stream {
        audio: mpsc::Receiver<Vec<i16>>,
        respond_to: mpsc::Sender<String>,
        span: tracing::Span,
    },
}

#[derive(Debug, Clone)]
pub struct Transcriber {
    sender: mpsc::Sender<TranscriptionRequest>,
}

impl Transcriber {
    /// Construct a new Transcriber
    pub fn new(config: &Config, backend: &Backend) -> Self {
        let (sender, receiver) = mpsc::channel(32);

        match backend {
            Backend::Deepgram => {
                let mut worker = deepgram::TranscriberWorker::new(
                    receiver,
                    config.deepgram.api_key.clone(),
                    config.deepgram.websocket_url.clone(),
                );
                tokio::spawn(async move { worker.run().await });
            }
            Backend::Whisper => {
                let mut worker =
                    whisper::TranscriberWorker::new(receiver, config.whisper.websocket_url.clone());
                tokio::spawn(async move { worker.run().await });
            }
        }

        Self { sender }
    }

    /// Stream audio to the transcriber and receive a stream of text back
    ///
    /// Audio is expected to be stereo signed 16 bit PCM at 48khz
    pub async fn stream(&self, audio: mpsc::Receiver<Vec<i16>>) -> mpsc::Receiver<String> {
        let (respond_to, text_receiver) = mpsc::channel(256);

        let request = TranscriptionRequest::Stream {
            audio,
            respond_to,
            span: tracing::Span::current(),
        };

        let _ = self.sender.send(request).await;

        text_receiver
    }
}
