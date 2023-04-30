// SPDX-License-Identifier: GPL-2.0-or-later

/// Handles the transcription of audio to text.
///
/// The behaviour of the transcription worker depends on what backend is
/// being used to transcribe the audio (DeepSpeech's CPU build, CUDA build, or some
/// third-party service).
use std::path::PathBuf;
use std::thread::JoinHandle;

use numpy::IntoPyArray;
use pyo3::{types::PyModule, Python};
use tokio::sync::{mpsc, oneshot};
use tracing::Instrument;

use crate::config::Config;
use crate::transcode::{discord_to_whisper, whisper_transcode};

const WHISPER: &str = include_str!("transcribe.py");

#[derive(Debug)]
pub enum TranscriptionRequest {
    Stream {
        audio: mpsc::Receiver<Vec<i16>>,
        respond_to: oneshot::Sender<String>,
        span: tracing::Span,
    },
    File {
        path: PathBuf,
        respond_to: oneshot::Sender<String>,
    },
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct Transcriber {
    sender: mpsc::Sender<TranscriptionRequest>,
}

impl Transcriber {
    /// Construct a new Transcriber
    pub fn new(config: &Config) -> Result<Self, crate::Error> {
        let (sender, receiver) = mpsc::channel(32);

        let worker = TranscriberWorker::new(receiver, config.whisper.model.clone())?;
        tokio::spawn(async move { worker.run().await });

        Ok(Self { sender })
    }

    pub async fn shutdown(&self) {
        let _ = self.sender.send(TranscriptionRequest::Shutdown).await;
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

    pub async fn file(&self, path: PathBuf) -> oneshot::Receiver<String> {
        let (respond_to, text_receiver) = oneshot::channel();

        let request = TranscriptionRequest::File { path, respond_to };

        let _ = self.sender.send(request).await;

        text_receiver
    }
}

enum Request {
    File(PathBuf, oneshot::Sender<String>),
    Raw(Vec<f32>, oneshot::Sender<String>),
    Shutdown,
}

struct TranscriberWorker {
    receiver: mpsc::Receiver<TranscriptionRequest>,
    transcriber: Option<JoinHandle<Result<(), crate::Error>>>,
    transcribe_channel: mpsc::Sender<Request>,
}

impl TranscriberWorker {
    fn new(
        receiver: mpsc::Receiver<TranscriptionRequest>,
        model: PathBuf,
    ) -> Result<Self, crate::Error> {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let transcriber = Some(
            std::thread::Builder::new()
                .name("whisper-transcriber".into())
                .spawn(|| Self::transcribe(model, rx))?,
        );
        Ok(TranscriberWorker {
            receiver,
            transcriber,
            transcribe_channel: tx,
        })
    }

    /// Processes transcription requests until the given receiver closes.
    ///
    /// This is intended to be run in a dedicated thread.
    fn transcribe(
        model: PathBuf,
        mut audio_receiver: mpsc::Receiver<Request>,
    ) -> Result<(), crate::Error> {
        let result = Python::with_gil(|py| {
            let module = PyModule::from_code(py, WHISPER, "transcribe.py", "transcribe")?;

            let load_model = module.getattr("load_model")?;
            load_model.call1((model,))?;

            let transcriber = module.getattr("transcribe")?;

            while let Some(request) = audio_receiver.blocking_recv() {
                match request {
                    Request::Raw(audio, sender) => {
                        tracing::debug!("Processing new transcription request");
                        let audio = audio.into_pyarray(py);
                        let result = transcriber
                            .call1((audio,))
                            .and_then(|r| r.extract())
                            .unwrap_or_default();
                        if sender.send(result).is_err() {
                            tracing::error!("Failed to send STT result back to the caller.");
                        }
                    }
                    Request::File(audio, sender) => {
                        tracing::debug!("Processing new transcription request");
                        let result = transcriber
                            .call1((audio,))
                            .and_then(|r| r.extract())
                            .unwrap_or_default();
                        if sender.send(result).is_err() {
                            tracing::error!("Failed to send STT result back to the caller.");
                        }
                    }
                    Request::Shutdown => {
                        tracing::info!("Shutting down the transcriber");
                        break;
                    }
                }
            }

            Ok(())
        });

        if result.is_err() {
            tracing::error!(err = ?result, "Transcribe thread failed!");
        }

        result
    }

    async fn run(mut self) {
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

                            let bin = whisper_transcode(discord_to_whisper(), bin).await;

                            if transcriber
                                .send(Request::Raw(bin, respond_to))
                                .await
                                .is_err()
                            {
                                tracing::error!("The transcriber thread is gone?");
                            }
                        }
                        .instrument(span),
                    );
                }
                TranscriptionRequest::File { path, respond_to } => {
                    let transcriber = self.transcribe_channel.clone();
                    tokio::spawn(async move {
                        //let mut file = File::open(path).await.unwrap();
                        //let mut bin = Vec::new();
                        //file.read_to_end(&mut bin).await.unwrap();

                        //let bin = whisper_transcode(container_to_whisper(), bin).await;

                        //if transcriber
                        //    .send(Request::Raw(bin, respond_to))
                        //    .await
                        //    .is_err()
                        //{
                        //    tracing::error!("The transcriber thread is gone?");
                        //}
                        if transcriber
                            .send(Request::File(path, respond_to))
                            .await
                            .is_err()
                        {
                            tracing::error!("The transcriber thread is gone?");
                        }
                    });
                }
                TranscriptionRequest::Shutdown => {
                    if self
                        .transcribe_channel
                        .send(Request::Shutdown)
                        .await
                        .is_err()
                    {
                        panic!("Unable to shut down the transcriber thread gracefully")
                    }

                    if let Some(thread) = self.transcriber.take() {
                        let result = thread.join();
                        match result {
                            Err(_) => tracing::error!("Failed to join the transcriber thread"),
                            Ok(Ok(_)) => tracing::info!("Shut down transcriber thread"),
                            Ok(Err(e)) => tracing::error!(error=?e, "Transcriber thread crashed"),
                        };
                    }
                }
            }
        }
    }
}
