use std::path::PathBuf;

use deepspeech::Model;
use log::info;
use tokio::{
    runtime::Handle,
    sync::{mpsc, oneshot},
};

use super::{async_trait, Config, Error, Transcribe};

struct TranscriberWorker {
    model_path: PathBuf,
    scorer_path: Option<PathBuf>,
    receiver: mpsc::Receiver<TranscriptionRequest>,
    gpu_sender: Option<mpsc::Sender<TranscriptionRequest>>,
}

#[derive(Debug)]
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
        gpu: bool,
    ) -> Self {
        let gpu_sender = {
            if gpu {
                let (gpu_sender, mut gpu_receiver) = mpsc::channel(64);
                let rt_handle = Handle::current();
                let model_path = model_path.clone();
                let scorer_path = scorer_path.clone();

                // TODO I should handle the case where the thread dies
                tokio::task::spawn_blocking(move || {
                    let mut deepspeech_model = Model::load_from_files(&model_path)
                        .expect("Unable to load deepspeech model");
                    if let Some(scorer) = scorer_path {
                        deepspeech_model.enable_external_scorer(&scorer).unwrap();
                    }
                    info!("Successfully loaded voice recognition model");
                    loop {
                        match rt_handle.block_on(gpu_receiver.recv()) {
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
                Some(gpu_sender)
            } else {
                None
            }
        };
        TranscriberWorker {
            model_path,
            scorer_path,
            receiver,
            gpu_sender,
        }
    }

    fn transcribe(&mut self, request: TranscriptionRequest) {
        if let Some(gpu_sender) = &self.gpu_sender {
            let sender = gpu_sender.clone();
            tokio::task::spawn(async move {
                let _ = sender.send(request).await;
            });
            return;
        }

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
                    if respond_to.send(result).is_err() {
                        info!("The transcription requester is gone");
                    }
                });
            }
        }
    }

    async fn run(&mut self) {
        while let Some(request) = self.receiver.recv().await {
            self.transcribe(request);
        }
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
        let mut transcriber = TranscriberWorker::new(
            receiver,
            config.deepspeech.model.clone(),
            config.deepspeech.scorer.clone(),
            config.deepspeech.gpu,
        );
        tokio::spawn(async move { transcriber.run().await });
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
