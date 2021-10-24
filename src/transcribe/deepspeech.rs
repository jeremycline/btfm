use std::path::{Path, PathBuf};

use deepspeech::Model;
use tokio::{runtime::Handle, sync::mpsc};
use tracing::{info, Instrument};

use super::TranscriptionRequest;
use crate::transcode::discord_to_wav;

pub struct TranscriberWorker {
    model_path: PathBuf,
    scorer_path: Option<PathBuf>,
    receiver: mpsc::Receiver<TranscriptionRequest>,
}

impl TranscriberWorker {
    pub fn new(
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

    pub fn transcribe(&mut self, request: TranscriptionRequest) {
        match request {
            TranscriptionRequest::Stream {
                mut audio,
                respond_to,
                span,
            } => {
                // Deepspeech supports streaming, but we're going to fake it here to get started with.
                let model_path = self.model_path.clone();
                let scorer_path = self.scorer_path.clone();

                tokio::task::spawn(
                    async move {
                        let mut buffer = Vec::new();
                        while let Some(mut snippet) = audio.recv().await {
                            buffer.append(&mut snippet)
                        }
                        let rt_handle = Handle::current();

                        let result = tokio::task::spawn_blocking(move || {
                            blocking_transcribe(
                                &model_path,
                                scorer_path.as_ref(),
                                rt_handle,
                                buffer,
                            )
                        })
                        .await;
                        if let Ok(text) = result {
                            if respond_to.send(text).await.is_err() {
                                info!("The transcription requester is gone");
                            }
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

fn blocking_transcribe(
    model_path: &Path,
    scorer_path: Option<&PathBuf>,
    rt_handle: Handle,
    audio: Vec<i16>,
) -> String {
    let mut deepspeech_model =
        Model::load_from_files(model_path).expect("Unable to load deepspeech model");
    if let Some(scorer) = scorer_path {
        deepspeech_model.enable_external_scorer(scorer).unwrap();
    }
    let sample_rate = deepspeech_model.get_sample_rate() as u32;

    let buffer = rt_handle.block_on(async move { discord_to_wav(audio, sample_rate).await });

    info!("Successfully loaded voice recognition model");
    let result = deepspeech_model.speech_to_text(&buffer).unwrap();
    info!("STT thinks someone said \"{}\"", result);
    result
}
