use std::path::PathBuf;

use deepspeech::Model;
use log::info;
use tokio::{runtime::Handle, sync::mpsc};

use super::TranscriptionRequest;
use crate::transcode::discord_to_wav;

pub struct TranscriberWorker {
    model_path: PathBuf,
    scorer_path: Option<PathBuf>,
    receiver: mpsc::Receiver<TranscriptionRequest>,
    gpu_sender: Option<mpsc::Sender<TranscriptionRequest>>,
}

impl TranscriberWorker {
    pub fn new(
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
                                TranscriptionRequest::Stream {
                                    mut audio,
                                    respond_to,
                                } => {
                                    let buffer = rt_handle.block_on(async move {
                                        let mut buffer = Vec::new();
                                        while let Some(mut snippet) = audio.recv().await {
                                            buffer.append(&mut snippet)
                                        }
                                        buffer
                                    });
                                    let result = deepspeech_model.speech_to_text(&buffer).unwrap();
                                    info!("STT thinks someone said \"{}\"", result);
                                    rt_handle.block_on(async move {
                                        if respond_to.send(result).await.is_err() {
                                            info!("The transcription requester is gone");
                                        }
                                    });
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

    pub fn transcribe(&mut self, request: TranscriptionRequest) {
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
            TranscriptionRequest::Stream {
                mut audio,
                respond_to,
            } => {
                // Deepspeech supports streaming, but we're going to fake it here because streaming with
                // GPU support gets wonky
                let model_path = self.model_path.clone();
                let scorer_path = self.scorer_path.clone();

                tokio::task::spawn(async move {
                    let mut buffer = Vec::new();
                    while let Some(mut snippet) = audio.recv().await {
                        buffer.append(&mut snippet)
                    }
                    let rt_handle = Handle::current();

                    let result = tokio::task::spawn_blocking(move || {
                        let mut deepspeech_model = Model::load_from_files(&model_path)
                            .expect("Unable to load deepspeech model");
                        if let Some(scorer) = scorer_path {
                            deepspeech_model.enable_external_scorer(&scorer).unwrap();
                        }
                        let sample_rate = deepspeech_model.get_sample_rate() as u32;

                        let buffer = rt_handle
                            .block_on(async move { discord_to_wav(buffer, sample_rate).await });

                        info!("Successfully loaded voice recognition model");
                        let result = deepspeech_model.speech_to_text(&buffer).unwrap();
                        info!("STT thinks someone said \"{}\"", result);
                        result
                    })
                    .await;
                    if let Ok(text) = result {
                        if respond_to.send(text).await.is_err() {
                            info!("The transcription requester is gone");
                        }
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
