/// Handles the transcription of audio to text.
///
/// The behaviour of the transcription worker depends on what backend is
/// being used to transcribe the audio (DeepSpeech's CPU build, CUDA build, or some
/// third-party service).
use tokio::sync::{mpsc, oneshot};

#[cfg(feature = "deepspeech-recognition")]
mod deepspeech;

#[cfg(feature = "deepgram-recognition")]
mod deepgram;

use crate::config::Config;
use crate::{Backend, Error};

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
            #[cfg(feature = "deepgram-recognition")]
            Backend::Deepgram => {
                let mut worker = deepgram::TranscriberWorker::new(
                    receiver,
                    config.deepgram.api_key.clone(),
                    config.deepgram.api_endpoint.clone(),
                );
                tokio::spawn(async move { worker.run().await });
            }
            #[cfg(feature = "deepspeech-recognition")]
            Backend::DeepSpeechCpu | Backend::DeepSpeechGpu => {
                let mut worker = deepspeech::TranscriberWorker::new(
                    receiver,
                    config.deepspeech.model.clone(),
                    config.deepspeech.scorer.clone(),
                    config.deepspeech.gpu,
                );
                tokio::spawn(async move { worker.run().await });
            }
        }

        Self { sender }
    }

    /// Transcribe PCM audio to plain, unannotated text
    pub async fn transcribe_plain_text(&self, audio: Vec<i16>) -> Result<String, Error> {
        let (sender, receiver) = oneshot::channel();
        let request = TranscriptionRequest::PlainText {
            audio,
            respond_to: sender,
        };

        let _ = self.sender.send(request).await;
        receiver.await.or(Err(Error::TranscriberGone))
    }

    pub async fn stream_plain_text(
        &self,
        audio: mpsc::Receiver<Vec<i16>>,
    ) -> mpsc::Receiver<String> {
        let (respond_to, text_receiver) = mpsc::channel(256);

        let request = TranscriptionRequest::Stream { audio, respond_to };

        let _ = self.sender.send(request).await;

        text_receiver
    }
}
