/// Defines public-facing structures used in the web API
use sqlx::PgConnection;

use crate::db;
use btfm_api_structs::{Clip, Phrase, Phrases};

pub async fn load_phrases(
    clip: &mut Clip,
    connection: &mut PgConnection,
) -> Result<(), crate::Error> {
    let db_phrases = db::phrases_for_clip(&mut *connection, clip.ulid.into()).await?;
    clip.phrases = Some(db_phrases_to_api(db_phrases));
    Ok(())
}

impl From<db::Phrase> for Phrase {
    fn from(phrase: db::Phrase) -> Self {
        Self {
            ulid: phrase.uuid.into(),
            phrase: phrase.phrase,
        }
    }
}

// TODO From
pub fn db_phrases_to_api(phrases: Vec<db::Phrase>) -> Phrases {
    Phrases {
        items: phrases.len() as u64,
        phrases: phrases
            .into_iter()
            .map(|p| p.into())
            .collect::<Vec<Phrase>>(),
    }
}
