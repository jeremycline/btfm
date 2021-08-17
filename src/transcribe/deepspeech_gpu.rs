use std::path::PathBuf;

use deepspeech::Model;
use log::info;
use tokio::{
    runtime::Handle,
    sync::{mpsc, oneshot},
};

use super::{async_trait, Config, Error, Transcribe};

struct TranscriberWorker {
    receiver: mpsc::Receiver<TranscriptionRequest>,
    thread_sender: mpsc::Sender<TranscriptionRequest>,
}

enum TranscriptionRequest {
    /// Provide a plain text transcription of the audio without any metadata
    PlainText {
        audio: Vec<i16>,
        respond_to: oneshot::Sender<String>,
    },
}

impl TranscriberWorker {
    fn new(
        receiver: mpsc::Receiver<TranscriptionRequest>,
        model_path: PathBuf,
        scorer_path: Option<PathBuf>,
    ) -> Self {
        let (thread_sender, mut thread_receiver) = mpsc::channel(64);
        let rt_handle = Handle::current();

        // TODO I should handle the case where the thread dies
        tokio::task::spawn_blocking(move || {
            let mut deepspeech_model =
                Model::load_from_files(&model_path).expect("Unable to load deepspeech model");
            if let Some(scorer) = scorer_path {
                deepspeech_model.enable_external_scorer(&scorer).unwrap();
            }
            info!("Successfully loaded voice recognition model");
            loop {
                match rt_handle.block_on(thread_receiver.recv()) {
                    Some(request) => match request {
                        TranscriptionRequest::PlainText { audio, respond_to } => {
                            let result = deepspeech_model.speech_to_text(&audio).unwrap();
                            info!("STT thinks someone said \"{}\"", result);
                            if respond_to.send(result).is_err() {
                                info!("The transcription requester is gone");
                            }
                        }
                    },
                    None => {
                        info!("Shutting down GPU transcription thread");
                        break;
                    }
                }
            }
        });
        TranscriberWorker {
            thread_sender,
            receiver,
        }
    }
}

async fn start_transcriber(mut transcriber: TranscriberWorker) {
    while let Some(request) = transcriber.receiver.recv().await {
        // TODO this obviously doesn't work if I want to generalize the interface
        let _ = transcriber.thread_sender.send(request).await;
    }
}

#[derive(Debug, Clone)]
pub struct Transcriber {
    sender: mpsc::Sender<TranscriptionRequest>,
}

#[async_trait]
impl Transcribe for Transcriber {
    fn new(config: &Config) -> Self {
        let (sender, receiver) = mpsc::channel(32);
        let transcriber = TranscriberWorker::new(
            receiver,
            config.deepspeech_model.clone(),
            config.deepspeech_scorer.clone(),
        );
        tokio::spawn(start_transcriber(transcriber));
        Self { sender }
    }

    async fn transcribe_plain_text(&self, audio: Vec<i16>) -> Result<String, Error> {
        let (sender, receiver) = oneshot::channel();
        let request = TranscriptionRequest::PlainText {
            audio,
            respond_to: sender,
        };

        let _ = self.sender.send(request).await;
        receiver.await.or(Err(Error::TranscriberGone))
    }
}
