use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use crate::Phrases;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    /// The unique identifier for the clip and primary key for the table.
    pub uuid: String,
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
    /// Phrases associated with the clip.
    pub phrases: Option<Phrases>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Clips {
    pub items: u64,
    pub clips: Vec<Clip>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ClipUpload {
    pub title: String,
    pub description: String,
    pub phrases: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClipUpdated {
    /// The new clip.
    pub new_clip: Clip,
    /// The old clip.
    pub old_clip: Clip,
}
