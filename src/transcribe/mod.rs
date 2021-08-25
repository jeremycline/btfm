/// Handles the transcription of audio to text.
///
/// The behaviour of the transcription worker depends on what backend is
/// being used to transcribe the audio (DeepSpeech's CPU build, CUDA build, or some
/// third-party service).
use async_trait::async_trait;

#[cfg(feature = "deepspeech-recognition")]
mod deepspeech;
#[cfg(feature = "deepspeech-recognition")]
pub use crate::transcribe::deepspeech::Transcriber;

use crate::config::Config;
use crate::Error;

#[async_trait]
pub trait Transcribe {
    /// Construct a new Transcriber
    fn new(config: &Config) -> Self;
    /// Transcribe PCM audio to plain, unannotated text
    async fn transcribe_plain_text(&self, audio: Vec<i16>) -> Result<String, Error>;
}
