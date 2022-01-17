// SPDX-License-Identifier: GPL-2.0-or-later
//
// Provides structures and functions for interacting with the database.
use std::{fs, path::Path};

use chrono::NaiveDateTime;
use rand::{distributions::Alphanumeric, prelude::*};
use serde::Serialize;
use sqlx::{types::Uuid, PgConnection, PgPool};
use tracing::{error, info, instrument};
use ulid::Ulid;

use crate::uuid_serializer;

/// Representation of an audio clip in the database.
///
/// Administrators add these clips which are played when phrases associated with the clip match
/// the output of semi-accurate speech-to-text.
#[derive(Debug)]
pub struct Clip {
    /// The unique identifier for the clip and primary key for the table.
    pub uuid: Uuid,
    /// The time when the clip was added to the database.
    pub created_on: NaiveDateTime,
    /// The last time the clip was played; this is equal to `created_on` when created.
    pub last_played: NaiveDateTime,
    /// Number of times the clip has been played.
    pub plays: i64,
    /// The output of speech-to-text on the `audio_file`, optionally used as a matching phrase.
    pub phrase: String,
    /// A description of the clip for human consumption.
    pub description: String,
    /// Path to the audio file, relative to the BTFM_DATA_DIR.
    pub audio_file: String,
}

impl std::fmt::Display for Clip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Clip ID {}\n\tPhrase: {}\n\tDescription: {}\n\tFile: {}\n",
            self.uuid, self.phrase, self.description, self.audio_file
        )
    }
}

impl Clip {
    /// List all clips in the database. No pagination is performed. Good luck.
    ///
    /// # Arguments
    ///
    /// `pool` - The SQLx database pool to use when issuing the query.
    ///
    /// # Returns
    ///
    /// A Result with all the clips in the database.
    #[instrument(skip_all)]
    pub async fn list(pool: &PgPool) -> Result<Vec<Clip>, crate::Error> {
        sqlx::query_as!(
            Clip,
            "
            SELECT *
            FROM clips;
            "
        )
        .fetch_all(pool)
        .await
        .map_err(crate::Error::Database)
    }

    /// Insert a new clip to the database.
    ///
    /// # Arguments
    ///
    /// `pool` - The SQLx database pool to use when issuing the query.
    ///
    /// `btfm_data_dir` - Path to the root of the BTFM data directory.
    ///
    /// `file` - Path to the audio file to add to the database; it will by copied to the clips
    ///          directory.
    ///
    /// `description` - The human-readable description for the clip.
    ///
    /// # Returns
    ///
    /// The Result is a newly-added Clip, assuming something terrible didn't happen.
    #[instrument(skip_all)]
    pub async fn insert(
        pool: &PgPool,
        btfm_data_dir: &Path,
        file: &Path,
        description: &str,
        phrase: &str,
    ) -> Result<Clip, crate::Error> {
        let mut file_prefix = "clips/".to_owned();
        let random_prefix: String = thread_rng()
            .sample_iter(&Alphanumeric)
            .take(6)
            .map(char::from)
            .collect();
        file_prefix.push_str(&random_prefix);
        let file_name = file_prefix
            + "-"
            + file
                .file_name()
                .expect("Path cannot terminate in ..")
                .to_str()
                .expect("File name is not valid UTF-8");
        let clip_destination = btfm_data_dir.join(&file_name);
        fs::copy(&file, &clip_destination).expect("Unable to copy clip to data directory");

        let uuid = Uuid::from_u128(Ulid::new().0);
        let insert_result = sqlx::query!(
            "
            INSERT INTO clips (uuid, phrase, description, audio_file)
            VALUES ($1, $2, $3, $4)
            RETURNING uuid, created_on, last_played, plays, phrase, description, audio_file
            ",
            uuid,
            phrase,
            description,
            file_name,
        )
        .fetch_one(pool)
        .await;
        match insert_result {
            Ok(insert) => {
                info!("Added clip for {}", &file_name);
                Ok(Clip {
                    uuid: insert.uuid,
                    created_on: insert.created_on,
                    last_played: insert.last_played,
                    plays: insert.plays,
                    phrase: insert.phrase,
                    description: insert.description,
                    audio_file: insert.audio_file,
                })
            }
            Err(e) => Err(crate::Error::Database(e)),
        }
    }

    /// Remove a clip from the database and remove the audio file associated with it.
    ///
    /// # Arguments
    ///
    /// `pool` - The SQLx database pool to use when issuing the query.
    ///
    /// `btfm_data_dir` - Path to the root of the BTFM data directory.
    ///
    /// # Returns
    ///
    /// The number of clips deleted, which should be either 1 or 0, or if things have
    /// gone very wrong, maybe many more. This isn't a very good API.
    #[instrument(skip_all)]
    pub async fn remove(&self, pool: &PgPool, btfm_data_dir: &Path) -> Result<u64, crate::Error> {
        let clip_path = btfm_data_dir.join(&self.audio_file);

        match tokio::fs::remove_file(&clip_path).await {
            Ok(_) => {
                info!("Removed audio file {}", &self.audio_file)
            }
            Err(err) => {
                error!(
                    "Failed to remove audio file at {}: {}",
                    &self.audio_file, err
                )
            }
        }

        sqlx::query!(
            "
            DELETE FROM clips
            WHERE uuid = $1
            ",
            self.uuid,
        )
        .execute(pool)
        .await
        .map_or_else(
            |e| Err(crate::Error::Database(e)),
            |delete| Ok(delete.rows_affected()),
        )
    }

    /// Update the phrase and description of a clip. Only the phrase and description
    /// fields can be updated; all other field changes are ignored.
    ///
    /// # Arguments
    ///
    /// `pool` - The SQLx database pool to use when issuing the query.
    ///
    /// # Returns
    ///
    /// A Result with the number of affected rows when issuing the update.
    #[instrument(skip_all)]
    pub async fn update(&self, pool: &PgPool) -> Result<u64, crate::Error> {
        sqlx::query!(
            "
            UPDATE clips
            SET description = $1, phrase = $2
            WHERE uuid = $3
            ",
            self.description,
            self.phrase,
            self.uuid,
        )
        .execute(pool)
        .await
        .map_or_else(
            |e| Err(crate::Error::Database(e)),
            |update| Ok(update.rows_affected()),
        )
    }

    /// Get a single clip by id. This is a terrible interface as it requires
    /// the user to already know the ID. It's only useful as the CLI is used
    /// by listing all clips and piping it to grep to find a phrase, which tells
    /// you something about how well the CLI meets the needs of the user.
    ///
    /// # Arguments
    ///
    /// `pool` - The SQLx database pool to use when issuing the query.
    ///
    /// `id` - The clip ID to get the full database row for.
    ///
    /// # Returns
    ///
    /// The Clip, or the database error you brought upon yourself.
    #[instrument(skip(pool))]
    pub async fn get(pool: &PgPool, uuid: Uuid) -> Result<Clip, crate::Error> {
        sqlx::query_as!(
            Clip,
            "
            SELECT *
            FROM clips
            WHERE uuid = $1;
            ",
            uuid
        )
        .fetch_one(pool)
        .await
        .map_err(crate::Error::Database)
    }

    /// Mark a clip as played; this will both increment the play counter and update the last played time.
    ///
    /// # Arguments
    ///
    /// `pool` - The SQLx database pool to use when issuing the query.
    ///
    /// `clip` - The clip to update.
    ///
    /// # Returns
    ///
    /// The number of rows updated, which should be 1 but maybe won't be, you should totally check that.
    #[instrument(skip_all)]
    pub async fn mark_played(&mut self, pool: &PgPool) -> Result<u64, crate::Error> {
        self.plays += 1;
        self.last_played = chrono::NaiveDateTime::from_timestamp(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("Check your system clock")
                .as_secs() as i64,
            0,
        );
        sqlx::query!(
            "
            UPDATE clips
            SET plays = $1, last_played = $2
            WHERE uuid = $3
            ",
            self.plays,
            self.last_played,
            self.uuid,
        )
        .execute(pool)
        .await
        .map_or_else(
            |e| Err(crate::Error::Database(e)),
            |update| Ok(update.rows_affected()),
        )
    }
}

/// Representation of a phrase in the database.
///
/// Speech-to-text is run on incoming audio and the result is compared to these phrases.
/// Phrases are associated with clips via `ClipPhrase` entries in a many-to-many relationship.
#[derive(Clone, Debug, Serialize)]
pub struct Phrase {
    #[serde(serialize_with = "uuid_serializer")]
    pub uuid: Uuid,
    pub phrase: String,
}

impl std::fmt::Display for Phrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Phrase ID {}: \"{}\"", self.uuid, self.phrase)
    }
}

impl Phrase {
    /// Get a list of all user-added phrases.
    ///
    /// # Arguments
    ///
    /// `pool` - The SQLx database pool to use when issuing the query.
    ///
    /// # Returns
    ///
    /// All Phrase objects in the database, or the unlying database error wrapped in an
    /// Error::Database.
    #[instrument(skip(pool))]
    pub async fn list(pool: &PgPool) -> Result<Vec<Phrase>, crate::Error> {
        sqlx::query_as!(
            Phrase,
            "
            SELECT *
            FROM phrases;
            "
        )
        .fetch_all(pool)
        .await
        .map_err(crate::Error::Database)
    }

    /// Add a new phrase to the database.
    ///
    /// # Arguments
    ///
    /// `pool` - The SQLx database pool to use when issuing the query.
    ///
    /// `phrase` - The new phrase to add; the phrase is case-insensitive.
    ///
    /// # Returns
    ///
    /// The Phrase ID.
    #[instrument(skip(pool))]
    pub async fn insert(pool: &PgPool, phrase: &str) -> Result<Phrase, crate::Error> {
        let lowercase_phrase = phrase.to_lowercase();
        let uuid = Uuid::from_u128(Ulid::new().0);
        sqlx::query!(
            "
            INSERT INTO phrases (phrase, uuid)
            VALUES ($1, $2)
            RETURNING uuid, phrase
            ",
            lowercase_phrase,
            uuid,
        )
        .fetch_one(pool)
        .await
        .map_or_else(
            |e| Err(crate::Error::Database(e)),
            |insert| {
                Ok(Phrase {
                    uuid,
                    phrase: insert.phrase,
                })
            },
        )
    }

    /// Remove the phrase.
    ///
    /// # Arguments
    ///
    /// `pool` - The SQLx database pool to use when issuing the query.
    ///
    /// # Returns
    ///
    /// The number of rows affected by the update.
    #[instrument(skip(pool))]
    pub async fn remove(&self, pool: &PgPool) -> Result<u64, crate::Error> {
        sqlx::query!(
            "
            DELETE FROM phrases
            WHERE uuid = $1
            ",
            self.uuid,
        )
        .execute(pool)
        .await
        .map_or_else(
            |e| Err(crate::Error::Database(e)),
            |delete| Ok(delete.rows_affected()),
        )
    }

    /// Update an existing phrase.
    ///
    /// # Arguments
    ///
    /// `pool` - The SQLx database pool to use when issuing the query.
    ///
    /// `phrase` - The new phrase to set.
    ///
    /// # Returns
    ///
    /// The number of rows affected by the update.
    #[instrument(skip(pool))]
    pub async fn update(&self, pool: &PgPool, phrase: &str) -> Result<u64, crate::Error> {
        sqlx::query!(
            "
            UPDATE phrases
            SET phrase = $1
            WHERE uuid = $2
            ",
            phrase,
            self.uuid,
        )
        .execute(pool)
        .await
        .map_or_else(
            |e| Err(crate::Error::Database(e)),
            |update| Ok(update.rows_affected()),
        )
    }

    /// Get a phrase by ID.
    ///
    /// # Arguments
    ///
    /// `pool` - The SQLx database pool to use when issuing the query.
    ///
    /// `phrase_id` - The phrase's primary key.
    ///
    /// # Returns
    ///
    /// The Phrase.
    #[instrument(skip(pool))]
    pub async fn get(pool: &PgPool, phrase_uuid: Uuid) -> Result<Phrase, crate::Error> {
        sqlx::query_as!(
            Phrase,
            "
            SELECT *
            FROM phrases
            WHERE uuid = $1
            ",
            phrase_uuid,
        )
        .fetch_one(pool)
        .await
        .map_or_else(|e| Err(crate::Error::Database(e)), Ok)
    }
}

#[derive(Debug)]
pub struct ClipPhrase {
    pub clip_uuid: Uuid,
    pub phrase_uuid: Uuid,
}

impl ClipPhrase {
    /// Associate a phrase with a clip; both the phrase and the clip must exist.
    ///
    /// # Arguments
    ///
    /// `pool` - The SQLx database pool to use when issuing the query.
    ///
    /// `clip_id` - The primary key of the clip to associate the phrase with.
    ///
    /// `phrase_id` - The primary key of the phrase to associate with the clip
    ///
    /// # Returns
    ///
    /// If an error occurs, it is returned, otherwise the clip_id and phrase_id provided are the composite
    /// primary key for the association table.
    #[instrument(skip(pool))]
    pub async fn insert(
        pool: &PgPool,
        clip_uuid: Uuid,
        phrase_uuid: Uuid,
    ) -> Result<(), crate::Error> {
        sqlx::query!(
            "
            INSERT INTO clips_to_phrases (clip_uuid, phrase_uuid)
            VALUES ($1, $2);
            ",
            clip_uuid,
            phrase_uuid,
        )
        .execute(pool)
        .await
        .map_or_else(|e| Err(crate::Error::Database(e)), |_| Ok(()))
    }

    /// Remove the association between a phrase and a clip.
    ///
    /// # Arguments
    ///
    /// `pool` - The SQLx database pool to use when issuing the query.
    ///
    /// `clip_id` - The primary key of the clip to remove the phrase from.
    ///
    /// `phrase_id` - The primary key of the phrase to remove from the clip
    ///
    /// # Returns
    ///
    /// If an error occurs, it is returned; this includes if the given ids are invalid.
    #[instrument(skip(pool))]
    pub async fn remove(
        pool: &PgPool,
        clip_uuid: Uuid,
        phrase_uuid: Uuid,
    ) -> Result<(), crate::Error> {
        sqlx::query!(
            "
            DELETE FROM clips_to_phrases
            WHERE clip_uuid = $1 AND phrase_uuid = $2;
            ",
            clip_uuid,
            phrase_uuid,
        )
        .execute(pool)
        .await
        .map_or_else(|e| Err(crate::Error::Database(e)), |_| Ok(()))
    }
}

/// Find clips associated with a given phrase.
///
/// # Arguments
///
/// `pool` - The SQLx database pool to use when issuing the query.
///
/// `phrase_id` - The primary key of the phrase for which you would
///               like to find the associated clips.
///
/// # Returns
///
/// All Clips associated with the given phrase ID.
#[instrument(skip(pool))]
pub async fn clips_for_phrase(pool: &PgPool, phrase_uuid: Uuid) -> Result<Vec<Clip>, crate::Error> {
    sqlx::query_as!(
        Clip,
        "
        SELECT clips.*
        FROM clips
        LEFT JOIN clips_to_phrases
        ON clips.uuid = clips_to_phrases.clip_uuid
        WHERE phrase_uuid = $1
        ",
        phrase_uuid
    )
    .fetch_all(pool)
    .await
    .map_err(crate::Error::Database)
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
        SELECT phrases.*
        FROM phrases
        LEFT JOIN clips_to_phrases
        ON phrases.uuid = clips_to_phrases.phrase_uuid
        WHERE clip_uuid = $1
        ",
        clip_uuid
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(crate::Error::Database)
}

/// Get the last time a clip was played.

/// # Arguments
///
/// `pool` - The SQLx database pool to use when issuing the query.
///
/// # Returns
///
/// The last time a clip was played by the bot. In the event that an error
/// occurs, the epoch is returned. I did this to make some sort of epoch
/// failure joke, in the docs, but phrasing it smoothly is difficult.
#[instrument(skip_all)]
pub async fn last_play_time(pool: &PgPool) -> NaiveDateTime {
    let clip_query = sqlx::query!(
        "
        SELECT last_played
        FROM clips
        ORDER BY last_played DESC
        LIMIT 1"
    )
    .fetch_one(pool)
    .await;
    match clip_query {
        Ok(clip) => clip.last_played,
        Err(_) => NaiveDateTime::from_timestamp(0, 0),
    }
}

/// Find all clips that match the given phrase.
///
/// # Arguments
///
/// `pool` - The SQLx database pool to use when issuing the query.
///
/// `phrase` - Arbitrary text to search for matching phrases.
///
/// # Returns
///
/// Clips that match the given phrase, if any. Clips match the phrase
/// if they contain (according to deepspeech) the given phrase, or if a
/// user-provided phrase is associated with it.
#[instrument(skip(pool))]
pub async fn match_phrase(pool: &PgPool, phrase: &str) -> Result<Vec<Clip>, crate::Error> {
    let clips = Clip::list(pool).await?;
    let phrases = Phrase::list(pool).await?;

    let mut matching_clips = Vec::new();
    for clip in clips {
        if phrase.contains(&clip.phrase) && clip.phrase.split_whitespace().count() > 2_usize {
            info!("Matched on '{}' based on the in-clip audio", &clip.phrase);
            matching_clips.push(clip);
        }
    }
    for potential_phrase in phrases {
        if phrase.contains(&potential_phrase.phrase) {
            matching_clips.append(&mut clips_for_phrase(pool, potential_phrase.uuid).await?);
            info!("Matched on '{}'", &potential_phrase);
        }
    }
    Ok(matching_clips)
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
            FROM phrases
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
    phrase: &str,
    clip_uuid: Uuid,
) -> Result<Phrase, crate::Error> {
    let lowercase_phrase = phrase.to_lowercase();
    let uuid = Uuid::from_u128(Ulid::new().0);
    let phrase = sqlx::query!(
        "
            INSERT INTO phrases (phrase, uuid)
            VALUES ($1, $2)
            RETURNING uuid, phrase
            ",
        lowercase_phrase,
        uuid,
    )
    .fetch_one(&mut *connection)
    .await
    .map(|record| Phrase {
        uuid: record.uuid,
        phrase: record.phrase,
    })?;
    sqlx::query!(
        "
            INSERT INTO clips_to_phrases (clip_uuid, phrase_uuid)
            VALUES ($1, $2);
            ",
        clip_uuid,
        phrase.uuid,
    )
    .execute(&mut *connection)
    .await?;

    Ok(phrase)
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
            FROM phrases
            WHERE uuid = $1
            ",
        uuid,
    )
    .fetch_one(&mut *connection)
    .await?;

    sqlx::query!(
        "
            DELETE FROM phrases
            WHERE uuid = $1
            ",
        uuid,
    )
    .execute(&mut *connection)
    .await?;

    Ok(phrase)
}

#[instrument(skip(connection))]
pub async fn get_clip(connection: &mut PgConnection, uuid: Uuid) -> Result<Clip, crate::Error> {
    Ok(sqlx::query_as!(
        Clip,
        "
            SELECT *
            FROM clips
            WHERE clips.uuid = $1;
        ",
        uuid
    )
    .fetch_one(&mut *connection)
    .await?)
}

/// Add a clip and any phrases included in the [`ClipUpload`] metadata.
#[instrument(skip_all)]
pub async fn add_clip(
    connection: &mut PgConnection,
    data: Vec<u8>,
    metadata: crate::web::serialization::ClipUpload,
    filename: &str,
) -> Result<Clip, crate::Error> {
    let config = crate::CONFIG.get().expect("Initialize the config");
    let mut file_prefix = "clips/".to_owned();
    let random_prefix: String = thread_rng()
        .sample_iter(&Alphanumeric)
        .take(6)
        .map(char::from)
        .collect();
    file_prefix.push_str(&random_prefix);
    let prefixed_filename = file_prefix + "-" + filename;
    let clip_destination = config.data_directory.join(&prefixed_filename);
    fs::write(&clip_destination, data).expect("woops or something");

    let uuid = Uuid::from_u128(Ulid::new().0);
    let clip = sqlx::query!(
        "
            INSERT INTO clips (uuid, phrase, description, audio_file)
            VALUES ($1, $2, $3, $4)
            RETURNING uuid, created_on, last_played, plays, phrase, description, audio_file
            ",
        uuid,
        "",
        metadata.description,
        prefixed_filename,
    )
    .fetch_one(&mut *connection)
    .await
    .map(|record| Clip {
        uuid: record.uuid,
        created_on: record.created_on,
        last_played: record.last_played,
        plays: record.plays,
        phrase: record.phrase,
        description: record.description,
        audio_file: record.audio_file,
    })?;

    for phrase in metadata.phrases.unwrap_or_else(Vec::new) {
        add_phrase(&mut *connection, &phrase, clip.uuid).await?;
    }
    Ok(clip)
}

/// Update a clip's metadata and phrases.
///
/// # Arguments
///
/// `uuid` - The primary key of the clip to update.
/// `description` - The new human-readable description of the clip
///
/// # Returns
///
/// A Result with the number of affected rows when issuing the update.
#[instrument(skip(connection), fields(phrases_deleted, clip_updated, phrases_added))]
pub async fn update_clip<S>(
    connection: &mut PgConnection,
    uuid: Uuid,
    description: &str,
    phrases: &[S],
) -> Result<(), crate::Error>
where
    S: AsRef<str> + std::fmt::Debug,
{
    let clip_updated = sqlx::query!(
        "
        UPDATE clips
        SET description = $1
        WHERE uuid = $2
        ",
        description,
        uuid,
    )
    .execute(&mut *connection)
    .await
    .map(|update| update.rows_affected())?;
    tracing::Span::current().record("clip_updated", &clip_updated);

    let phrases_deleted = sqlx::query!(
        "
        DELETE FROM phrases
        WHERE uuid IN (
            SELECT phrases.uuid
            FROM phrases
            LEFT JOIN clips_to_phrases
            ON phrases.uuid = clips_to_phrases.phrase_uuid
            WHERE clip_uuid = $1
        )
        ",
        uuid
    )
    .execute(&mut *connection)
    .await
    .map(|deleted| deleted.rows_affected())?;
    tracing::Span::current().record("phrases_deleted", &phrases_deleted);

    for phrase in phrases {
        add_phrase(connection, phrase.as_ref(), uuid).await?;
    }
    tracing::Span::current().record("phrases_added", &phrases.len());

    Ok(())
}

/// Remove a clip from the database and remove the audio file associated with it.
#[instrument(skip(connection))]
pub async fn remove_clip(connection: &mut PgConnection, uuid: Uuid) -> Result<Clip, crate::Error> {
    let clip = sqlx::query_as!(
        Clip,
        "
            SELECT *
            FROM clips
            WHERE clips.uuid = $1;
        ",
        uuid
    )
    .fetch_one(&mut *connection)
    .await?;

    let config = crate::CONFIG.get().expect("Initialize the config");
    let clip_path = config.data_directory.join(&clip.audio_file);

    match tokio::fs::remove_file(&clip_path).await {
        Ok(_) => {
            info!("Removed audio file {}", &clip.audio_file)
        }
        Err(err) => {
            error!(
                "Failed to remove audio file at {}: {}",
                &clip.audio_file, err
            )
        }
    }

    sqlx::query!(
        "
            DELETE FROM clips
            WHERE uuid = $1
            ",
        uuid,
    )
    .execute(&mut *connection)
    .await?;

    Ok(clip)
}
