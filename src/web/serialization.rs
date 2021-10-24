/// Defines public-facing structures used in the web API
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// A phrase used to trigger one or more clips
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Phrase {
    pub ulid: ulid::Ulid,
    pub phrase: String,
}

impl From<crate::db::Phrase> for Phrase {
    fn from(phrase: crate::db::Phrase) -> Self {
        Self {
            ulid: phrase.uuid.into(),
            phrase: phrase.phrase,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    /// The unique identifier for the clip and primary key for the table.
    pub ulid: ulid::Ulid,
    /// The time when the clip was added to the database.
    pub created_on: NaiveDateTime,
    /// The last time the clip was played; this is equal to `created_on` when created.
    pub last_played: NaiveDateTime,
    /// Number of times the clip has been played.
    pub plays: i64,
    /// The output of speech-to-text on the `audio_file`, optionally used as a matching phrase.
    pub speech_detected: String,
    /// A description of the clip for human consumption.
    pub description: String,
    /// Path to the audio file, relative to the BTFM_DATA_DIR.
    pub audio_file: String,
    pub phrases: Option<Vec<Phrase>>,
}

impl From<crate::db::Clip> for Clip {
    fn from(clip: crate::db::Clip) -> Self {
        Self {
            ulid: clip.uuid.into(),
            created_on: clip.created_on,
            last_played: clip.last_played,
            plays: clip.plays,
            speech_detected: clip.phrase,
            description: clip.description,
            audio_file: clip.audio_file,
            phrases: None,
        }
    }
}
