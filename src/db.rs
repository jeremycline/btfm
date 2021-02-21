// SPDX-License-Identifier: GPL-2.0-or-later
//
// Provides structures and functions for interacting with the database.
use std::{fs, path::Path};

use chrono::NaiveDateTime;
use deepspeech::Model;
use log::{error, info};
use rand::{distributions::Alphanumeric, prelude::*};
use sqlx::SqlitePool;

/// Representation of an audio clip in the database.
///
/// Administrators add these clips which are played when phrases associated with the clip match
/// the output of semi-accurate speech-to-text.
#[derive(Debug)]
pub struct Clip {
    /// Unique identifier for the clip; the primary key for the table.
    pub id: i64,
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
            self.id, self.phrase, self.description, self.audio_file
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
    pub async fn list(pool: &SqlitePool) -> Result<Vec<Clip>, crate::Error> {
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
    pub async fn insert(
        pool: &SqlitePool,
        btfm_data_dir: &Path,
        file: &Path,
        description: &str,
        deepspeech_model: &Path,
        deepspeech_external_scorer: &Path,
    ) -> Result<Clip, crate::Error> {
        let clips_dir = btfm_data_dir.join("clips");
        let file_prefix: String = thread_rng().sample_iter(&Alphanumeric).take(6).collect();
        let file_name = file_prefix
            + "-"
            + file
                .file_name()
                .expect("Path cannot terminate in ..")
                .to_str()
                .expect("File name is not valid UTF-8");
        let clip_destination = clips_dir.join(&file_name);
        fs::copy(&file, &clip_destination).expect("Unable to copy clip to data directory");

        let ds_model =
            Model::load_from_files(&deepspeech_model).expect("Unable to load deepspeech model");
        let audio = crate::voice::file_to_wav(&clip_destination, ds_model.get_sample_rate()).await;
        let phrase = crate::voice::voice_to_text(
            deepspeech_model.to_owned(),
            Some(deepspeech_external_scorer.to_owned()),
            audio,
        )
        .await;

        let insert_result = sqlx::query!(
            "
            INSERT INTO clips (phrase, description, audio_file)
            VALUES (?, ?, ?);
            ",
            phrase,
            description,
            file_name,
        )
        .execute(pool)
        .await;
        match insert_result {
            Ok(insert) => {
                info!("Added clip for {}", file_name.as_str());
                return Clip::get(pool, insert.last_insert_rowid()).await;
            }
            Err(e) => return Err(crate::Error::Database(e)),
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
    pub async fn remove(
        &self,
        pool: &SqlitePool,
        btfm_data_dir: &Path,
    ) -> Result<u64, crate::Error> {
        let clip_path = btfm_data_dir.join("clips").join(&self.audio_file);

        match tokio::fs::remove_file(&clip_path).await {
            Ok(_) => {
                info!("Removed audio file clips/{}", &self.audio_file)
            }
            Err(err) => {
                error!(
                    "Failed to remove audio file at clips/{}: {}",
                    &self.audio_file, err
                )
            }
        }

        sqlx::query!(
            "
            DELETE FROM clips
            WHERE id = ?
            ",
            self.id,
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
    pub async fn update(&self, pool: &SqlitePool) -> Result<u64, crate::Error> {
        sqlx::query!(
            "
            UPDATE clips
            SET description = ?, phrase = ?
            WHERE id = ?
            ",
            self.description,
            self.phrase,
            self.id,
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
    pub async fn get(pool: &SqlitePool, id: i64) -> Result<Clip, crate::Error> {
        sqlx::query_as!(
            Clip,
            "
            SELECT *
            FROM clips
            WHERE id = ?;
            ",
            id
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
    pub async fn mark_played(&mut self, pool: &SqlitePool) -> Result<u64, crate::Error> {
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
            SET plays = ?, last_played = ?
            WHERE id = ?
            ",
            self.plays,
            self.last_played,
            self.id,
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
#[derive(Debug)]
pub struct Phrase {
    pub id: i64,
    pub phrase: String,
}

impl std::fmt::Display for Phrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Phrase ID {}: \"{}\"", self.id, self.phrase)
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
    pub async fn list(pool: &SqlitePool) -> Result<Vec<Phrase>, crate::Error> {
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
    pub async fn insert(pool: &SqlitePool, phrase: &str) -> Result<i64, crate::Error> {
        let lowercase_phrase = phrase.to_lowercase();
        sqlx::query!(
            "
            INSERT INTO phrases (phrase)
            VALUES (?);
            ",
            lowercase_phrase
        )
        .execute(pool)
        .await
        .map_or_else(
            |e| Err(crate::Error::Database(e)),
            |insert| Ok(insert.last_insert_rowid()),
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
    pub async fn remove(&self, pool: &SqlitePool) -> Result<u64, crate::Error> {
        sqlx::query!(
            "
            DELETE FROM phrases
            WHERE id = ?
            ",
            self.id,
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
    pub async fn update(&self, pool: &SqlitePool, phrase: &str) -> Result<u64, crate::Error> {
        sqlx::query!(
            "
            UPDATE phrases
            SET phrase = ?
            WHERE id = ?
            ",
            phrase,
            self.id,
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
    pub async fn get(pool: &SqlitePool, phrase_id: i64) -> Result<Phrase, crate::Error> {
        sqlx::query_as!(
            Phrase,
            "
            SELECT *
            FROM phrases
            WHERE id = ?
            ",
            phrase_id,
        )
        .fetch_one(pool)
        .await
        .map_or_else(|e| Err(crate::Error::Database(e)), Ok)
    }
}

#[derive(Debug)]
pub struct ClipPhrase {
    pub clip_id: i64,
    pub phrase_id: i64,
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
    pub async fn insert(
        pool: &SqlitePool,
        clip_id: i64,
        phrase_id: i64,
    ) -> Result<(), crate::Error> {
        sqlx::query!(
            "
            INSERT INTO clips_phrases (clip_id, phrase_id)
            VALUES (?, ?);
            ",
            clip_id,
            phrase_id,
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
    pub async fn remove(
        pool: &SqlitePool,
        clip_id: i64,
        phrase_id: i64,
    ) -> Result<(), crate::Error> {
        sqlx::query!(
            "
            DELETE FROM clips_phrases
            WHERE clip_id = ? AND phrase_id = ?;
            ",
            clip_id,
            phrase_id,
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
pub async fn clips_for_phrase(
    pool: &SqlitePool,
    phrase_id: i64,
) -> Result<Vec<Clip>, crate::Error> {
    sqlx::query_as!(
        Clip,
        "
        SELECT clips.*
        FROM clips
        LEFT JOIN clips_phrases
        ON clips.id = clips_phrases.clip_id
        WHERE phrase_id = ?
        ",
        phrase_id
    )
    .fetch_all(pool)
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
pub async fn last_play_time(pool: &SqlitePool) -> NaiveDateTime {
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
        Ok(clip) => return clip.last_played,
        Err(_) => return NaiveDateTime::from_timestamp(0, 0),
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
pub async fn match_phrase(pool: &SqlitePool, phrase: &str) -> Result<Vec<Clip>, crate::Error> {
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
            matching_clips.append(&mut clips_for_phrase(pool, potential_phrase.id).await?);
            info!("Matched on '{}'", &potential_phrase);
        }
    }
    Ok(matching_clips)
}
