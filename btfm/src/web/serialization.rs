/// Defines public-facing structures used in the web API
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;

use crate::db;

#[derive(Serialize)]
pub struct Status {
    pub db_version: Option<u32>,
    pub db_connections: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClipUpdated {
    /// The new clip.
    pub new_clip: Clip,
    /// The old clip.
    pub old_clip: Clip,
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
    /// Phrases associated with the clip.
    pub phrases: Option<Phrases>,
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

impl Clip {
    pub async fn load_phrases(
        &mut self,
        connection: &mut PgConnection,
    ) -> Result<(), crate::Error> {
        self.phrases = Some(
            db::phrases_for_clip(&mut *connection, self.ulid.into())
                .await?
                .into(),
        );
        Ok(())
    }
}
#[derive(Debug, Serialize)]
pub struct Clips {
    pub items: u64,
    pub clips: Vec<Clip>,
}

/// A phrase used to trigger one or more clips
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Phrase {
    pub ulid: ulid::Ulid,
    pub phrase: String,
}

impl From<db::Phrase> for Phrase {
    fn from(phrase: db::Phrase) -> Self {
        Self {
            ulid: phrase.uuid.into(),
            phrase: phrase.phrase,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct CreatePhrase {
    /// The phrase.
    pub phrase: String,
    /// The clip to associate the phrase to.
    pub clip: ulid::Ulid,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Phrases {
    pub items: u64,
    pub phrases: Vec<Phrase>,
}

impl From<Vec<db::Phrase>> for Phrases {
    fn from(phrases: Vec<db::Phrase>) -> Self {
        Self {
            items: phrases.len() as u64,
            phrases: phrases
                .into_iter()
                .map(|p| p.into())
                .collect::<Vec<Phrase>>(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ClipUpload {
    pub description: String,
    pub phrases: Option<Vec<String>>,
}
