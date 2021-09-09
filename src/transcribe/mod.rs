/// Handles the transcription of audio to text.
///
/// The behaviour of the transcription worker depends on what backend is
/// being used to transcribe the audio (DeepSpeech's CPU build, CUDA build, or some
/// third-party service).
use tokio::sync::{mpsc, oneshot};

use crate::config::Config;
use crate::{Backend, Error};

mod deepgram;
mod deepspeech;

#[derive(Debug)]
pub enum TranscriptionRequest {
    /// Provide a plain text transcription of the audio without any metadata
    PlainText {
        audio: Vec<i16>,
        respond_to: oneshot::Sender<String>,
    },
    Stream {
        audio: mpsc::Receiver<Vec<i16>>,
        respond_to: mpsc::Sender<String>,
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
                    config.deepgram.api_endpoint.clone(),
                );
                tokio::spawn(async move { worker.run().await });
            }
            Backend::DeepSpeech => {
                let mut worker = deepspeech::TranscriberWorker::new(
                    receiver,
                    config.deepspeech.model.clone(),
                    config.deepspeech.scorer.clone(),
                );
                tokio::spawn(async move { worker.run().await });
            }
        }

        Self { sender }
    }

    /// Convenience function to convert
    pub async fn plain_text(&self, audio: Vec<i16>) -> Result<String, Error> {
        let (sender, receiver) = oneshot::channel();
        let request = TranscriptionRequest::PlainText {
            audio,
            respond_to: sender,
        };

        let _ = self.sender.send(request).await;
        receiver.await.or(Err(Error::TranscriberGone))
    }

    /// Stream audio to the transcriber and receive a stream of text back
    ///
    /// Audio is expected to be stereo signed 16 bit PCM at 48khz
    pub async fn stream(&self, audio: mpsc::Receiver<Vec<i16>>) -> mpsc::Receiver<String> {
        let (respond_to, text_receiver) = mpsc::channel(256);

        let request = TranscriptionRequest::Stream { audio, respond_to };

        let _ = self.sender.send(request).await;

        text_receiver
    }
}
