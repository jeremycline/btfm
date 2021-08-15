/// Handles the transcription of audio to text.
///
/// The behaviour of the transcription worker depends on what backend is
/// being used to transcribe the audio (DeepSpeech's CPU build, CUDA build, or some
/// third-party service).
use std::path::PathBuf;

use async_trait::async_trait;

#[cfg(feature = "deepspeech_cpu")]
mod deepspeech_cpu;
use crate::Error;
#[cfg(feature = "deepspeech_cpu")]
pub use deepspeech_cpu::Transcriber;

pub struct Config {
    pub deepspeech_model: PathBuf,
    pub deepspeech_scorer: Option<PathBuf>,
}

impl Config {
    pub fn new(deepspeech_model: PathBuf, deepspeech_scorer: Option<PathBuf>) -> Self {
        Config {
            deepspeech_model,
            deepspeech_scorer,
        }
    }
}

#[async_trait]
pub trait Transcribe {
    /// Construct a new Transcriber
    fn new(config: &Config) -> Self;
    /// Transcribe PCM audio to plain, unannotated text
    async fn transcribe_plain_text(&self, audio: Vec<i16>) -> Result<String, Error>;
}
