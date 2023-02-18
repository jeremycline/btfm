// SPDX-License-Identifier: GPL-2.0-or-later
use std::{path::PathBuf, thread::JoinHandle};

use numpy::IntoPyArray;
use pyo3::{types::PyModule, Python};
use tokio::sync::{mpsc, oneshot};
use tracing::Instrument;

use super::TranscriptionRequest;
use crate::transcode::whisper_transcode;

const WHISPER: &str = include_str!("../transcribe.py");

pub struct TranscriberWorker {
    receiver: mpsc::Receiver<TranscriptionRequest>,
    _transcriber: JoinHandle<Result<(), crate::Error>>,
    transcribe_channel: mpsc::Sender<(Vec<f32>, oneshot::Sender<String>)>,
}

impl TranscriberWorker {
    pub fn new(receiver: mpsc::Receiver<TranscriptionRequest>, model: PathBuf) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let transcriber = std::thread::spawn(|| transcribe(model, rx));
        TranscriberWorker {
            receiver,
            _transcriber: transcriber,
            transcribe_channel: tx,
        }
    }

    fn transcribe(&mut self, request: TranscriptionRequest) {
        match request {
            TranscriptionRequest::Stream {
                audio,
                respond_to,
                span,
            } => {
                let handler = handle_request(self.transcribe_channel.clone(), audio, respond_to)
                    .instrument(span);
                tokio::spawn(handler);
            }
        }
    }

    pub async fn run(&mut self) {
        while let Some(request) = self.receiver.recv().await {
            self.transcribe(request);
        }
    }
}

async fn handle_request(
    transcribe_channel: mpsc::Sender<(Vec<f32>, oneshot::Sender<String>)>,
    mut audio: mpsc::Receiver<Vec<i16>>,
    respond_to: mpsc::Sender<String>,
) {
    let mut bin = Vec::new();
    while let Some(chunk) = audio.recv().await {
        for sample in chunk.into_iter() {
            bin.append(&mut sample.to_le_bytes().to_vec());
        }
    }

    let bin = whisper_transcode(bin).await;
    let (tx, rx) = oneshot::channel();

    if transcribe_channel.send((bin, tx)).await.is_err() {
        tracing::error!("The transcriber thread is gone?");
        let _ = respond_to.send("".to_string()).await;
    } else if respond_to.send(rx.await.unwrap_or_default()).await.is_err() {
        tracing::error!("Failed to ferry response back to caller");
    }
}

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
