// SPDX-License-Identifier: GPL-2.0-or-later

/// Handles the transcription of audio to text.
///
/// The behaviour of the transcription worker depends on what backend is
/// being used to transcribe the audio (DeepSpeech's CPU build, CUDA build, or some
/// third-party service).
use std::{path::PathBuf, thread::JoinHandle};

use numpy::IntoPyArray;
use pyo3::{types::PyModule, Python};
use tokio::sync::{mpsc, oneshot};
use tracing::Instrument;

use crate::config::Config;
use crate::transcode::whisper_transcode;
use crate::Backend;

const WHISPER: &str = include_str!("transcribe.py");

#[derive(Debug)]
pub enum TranscriptionRequest {
    Stream {
        audio: mpsc::Receiver<Vec<i16>>,
        respond_to: oneshot::Sender<String>,
        span: tracing::Span,
    },
}

#[derive(Debug, Clone)]
pub struct Transcriber {
    sender: mpsc::Sender<TranscriptionRequest>,
}

impl Transcriber {
    /// Construct a new Transcriber
    pub fn new(config: &Config, backend: &Backend) -> Result<Self, crate::Error> {
        let (sender, receiver) = mpsc::channel(32);

        match backend {
            Backend::Whisper => {
                let mut worker = TranscriberWorker::new(receiver, config.whisper.model.clone())?;
                tokio::spawn(async move { worker.run().await });
            }
        }

        Ok(Self { sender })
    }

    /// Stream audio to the transcriber and receive a stream of text back
    ///
    /// Audio is expected to be stereo signed 16 bit PCM at 48khz
    pub async fn stream(&self, audio: mpsc::Receiver<Vec<i16>>) -> oneshot::Receiver<String> {
        let (respond_to, text_receiver) = oneshot::channel();

        let request = TranscriptionRequest::Stream {
            audio,
            respond_to,
            span: tracing::Span::current(),
        };

        let _ = self.sender.send(request).await;

        text_receiver
    }
}

struct TranscriberWorker {
    receiver: mpsc::Receiver<TranscriptionRequest>,
    _transcriber: JoinHandle<Result<(), crate::Error>>,
    transcribe_channel: mpsc::Sender<(Vec<f32>, oneshot::Sender<String>)>,
}

impl TranscriberWorker {
    fn new(
        receiver: mpsc::Receiver<TranscriptionRequest>,
        model: PathBuf,
    ) -> Result<Self, crate::Error> {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let transcriber = std::thread::Builder::new()
            .name("whisper-transcriber".into())
            .spawn(|| Self::transcribe(model, rx))?;
        Ok(TranscriberWorker {
            receiver,
            _transcriber: transcriber,
            transcribe_channel: tx,
        })
    }

    /// Processes transcription requests until the given receiver closes.
    ///
    /// This is intended to be run in a dedicated thread.
    fn transcribe(
        model: PathBuf,
        mut audio_receiver: mpsc::Receiver<(Vec<f32>, oneshot::Sender<String>)>,
    ) -> Result<(), crate::Error> {
        Python::with_gil(|py| {
            let module = PyModule::from_code(py, WHISPER, "transcribe.py", "transcribe")?;

            let load_model = module.getattr("load_model")?;
            load_model.call1((model,))?;

            let transcriber = module.getattr("transcribe")?;

            while let Some((audio, sender)) = audio_receiver.blocking_recv() {
                let audio = audio.into_pyarray(py);
                let result = transcriber
                    .call1((audio,))
                    .and_then(|r| r.extract())
                    .unwrap_or_default();
                if sender.send(result).is_err() {
                    tracing::error!("Failed to send STT result back to the caller.");
                }
            }

            Ok(())
        })
    }

    async fn run(&mut self) {
        while let Some(request) = self.receiver.recv().await {
            match request {
                TranscriptionRequest::Stream {
                    mut audio,
                    respond_to,
                    span,
                } => {
                    let transcriber = self.transcribe_channel.clone();
                    tokio::spawn(
                        async move {
                            let mut bin = Vec::new();
                            while let Some(chunk) = audio.recv().await {
                                for sample in chunk.into_iter() {
                                    bin.append(&mut sample.to_le_bytes().to_vec());
                                }
                            }

                            let bin = whisper_transcode(bin).await;

                            if transcriber.send((bin, respond_to)).await.is_err() {
                                tracing::error!("The transcriber thread is gone?");
                            }
                        }
                        .instrument(span),
                    );
                }
            }
        }
    }
}
