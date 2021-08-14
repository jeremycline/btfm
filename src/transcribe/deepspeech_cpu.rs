use std::path::PathBuf;

use deepspeech::Model;
use log::info;
use tokio::sync::{mpsc, oneshot};

use crate::Error;

struct TranscriberWorker {
    model_path: PathBuf,
    scorer_path: Option<PathBuf>,
    receiver: mpsc::Receiver<TranscriptionRequest>,
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
        TranscriberWorker {
            model_path,
            scorer_path,
            receiver,
        }
    }

    fn transcribe(&mut self, request: TranscriptionRequest) {
        match request {
            TranscriptionRequest::PlainText { audio, respond_to } => {
                let model_path = self.model_path.clone();
                let scorer_path = self.scorer_path.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let mut deepspeech_model = Model::load_from_files(&model_path)
                        .expect("Unable to load deepspeech model");
                    if let Some(scorer) = scorer_path {
                        deepspeech_model.enable_external_scorer(&scorer).unwrap();
                    }
                    info!("Successfully loaded voice recognition model");
                    let result = deepspeech_model.speech_to_text(&audio).unwrap();
                    info!("STT thinks someone said \"{}\"", result);
                    if let Err(_) = respond_to.send(result) {
                        info!("The transcription requester is gone");
                    }
                });
            }
        }
    }
}

async fn start_transcriber(mut transcriber: TranscriberWorker) {
    while let Some(request) = transcriber.receiver.recv().await {
        transcriber.transcribe(request);
    }
}

#[derive(Debug, Clone)]
pub struct Transcriber {
    sender: mpsc::Sender<TranscriptionRequest>,
}

impl Transcriber {
    pub fn new(model_path: PathBuf, scorer_path: Option<PathBuf>) -> Self {
        let (sender, receiver) = mpsc::channel(32);
        let transcriber = TranscriberWorker::new(receiver, model_path, scorer_path);
        tokio::spawn(start_transcriber(transcriber));
        Self { sender }
    }

    pub async fn transcribe_plain_text(&self, audio: Vec<i16>) -> Result<String, Error> {
        let (sender, receiver) = oneshot::channel();
        let request = TranscriptionRequest::PlainText {
            audio,
            respond_to: sender,
        };

        let _ = self.sender.send(request).await;
        receiver.await.or(Err(Error::TranscriberGone))
    }
}
