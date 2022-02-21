// SPDX-License-Identifier: GPL-2.0-or-later
//
// Provides structures and functions for the phrases in the database
use serde::Serialize;
use sqlx::{types::Uuid, PgConnection};
use tracing::instrument;
use ulid::Ulid;

use crate::uuid_serializer;

/// Representation of a phrase in the database.
///
/// Speech-to-text is run on incoming audio and the result is compared to these phrases.
/// Phrases are associated with clips via `ClipPhrase` entries in a many-to-many relationship.
#[derive(Clone, Debug, Serialize)]
pub struct Phrase {
    #[serde(serialize_with = "uuid_serializer")]
    pub uuid: Uuid,
    #[serde(serialize_with = "uuid_serializer")]
    pub clip: Uuid,
    pub phrase: String,
}

impl std::fmt::Display for Phrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Phrase ID {}: \"{}\"", self.uuid, self.phrase)
    }
}

/// Find phrases associated with a given clip.
///
/// # Arguments
///
/// `pool` - The SQLx database pool to use when issuing the query.
///
/// `clip_id` - The primary key of the clip for which you would
///               like to find the associated phrases.
///
/// # Returns
///
/// All Phrases associated with the given clip ID.
#[instrument(skip(connection))]
pub async fn phrases_for_clip(
    connection: &mut PgConnection,
    clip_uuid: Uuid,
) -> Result<Vec<Phrase>, crate::Error> {
    sqlx::query_as!(
        Phrase,
        "
        SELECT *
        FROM clip_phrases
        WHERE clip = $1
        ",
        clip_uuid
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(crate::Error::Database)
}

/// Get a single phrase by Uuid.
#[instrument(skip(connection))]
pub async fn get_phrase(
    connection: &mut PgConnection,
    phrase_uuid: Uuid,
) -> Result<Phrase, crate::Error> {
    Ok(sqlx::query_as!(
        Phrase,
        "
            SELECT *
            FROM clip_phrases
            WHERE uuid = $1
            ",
        phrase_uuid,
    )
    .fetch_one(&mut *connection)
    .await?)
}

/// Add a phrase to a clip.
#[instrument(skip(connection))]
pub async fn add_phrase(
    connection: &mut PgConnection,
    clip: Uuid,
    phrase: &str,
) -> Result<Phrase, crate::Error> {
    let phrase = phrase.to_lowercase();
    let uuid = Uuid::from_u128(Ulid::new().0);
    sqlx::query!(
        "
        INSERT INTO clip_phrases (uuid, clip, phrase)
        VALUES ($1, $2, $3)
        ",
        uuid,
        clip,
        phrase,
    )
    .execute(&mut *connection)
    .await?;

    Ok(Phrase { uuid, clip, phrase })
}

/// List all known phrases in the database.
#[instrument(skip(connection))]
pub async fn list_phrases(connection: &mut PgConnection) -> Result<Vec<Phrase>, crate::Error> {
    sqlx::query_as!(
        Phrase,
        "
        SELECT *
        FROM clip_phrases;
        "
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(crate::Error::Database)
}

/// Remove the phrase.
///
/// # Returns
///
/// The phrase that was deleted.
#[instrument(skip(connection))]
pub async fn remove_phrase(
    connection: &mut PgConnection,
    uuid: Uuid,
) -> Result<Phrase, crate::Error> {
    let phrase = sqlx::query_as!(
        Phrase,
        "
            SELECT *
            FROM clip_phrases
            WHERE uuid = $1
            ",
        uuid,
    )
    .fetch_one(&mut *connection)
    .await?;

    sqlx::query!(
        "
            DELETE FROM clip_phrases
            WHERE uuid = $1
            ",
        uuid,
    )
    .execute(&mut *connection)
    .await?;

    Ok(phrase)
}
