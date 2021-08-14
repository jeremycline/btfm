/// Handles the transcription of audio to text.
///
/// The behaviour of the transcription worker depends on what backend is
/// being used to transcribe the audio (DeepSpeech's CPU build, CUDA build, or some
/// third-party service).
#[cfg(feature = "deepspeech_cpu")]
mod deepspeech_cpu;
#[cfg(feature = "deepspeech_cpu")]
pub use deepspeech_cpu::Transcriber;
