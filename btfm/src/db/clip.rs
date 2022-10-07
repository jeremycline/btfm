use std::fs;

use btfm_api_structs::Clip as ApiClip;
use btfm_api_structs::ClipUpload;
use chrono::NaiveDateTime;
use rand::{distributions::Alphanumeric, prelude::*};
use sqlx::{types::Uuid, PgConnection};
use tracing::{error, info, instrument};
use ulid::Ulid;

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

impl From<Clip> for ApiClip {
    fn from(clip: Clip) -> Self {
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
pub async fn mark_played(
    connection: &mut PgConnection,
    clip: &mut Clip,
) -> Result<u64, crate::Error> {
    clip.plays += 1;
    clip.last_played = chrono::NaiveDateTime::from_timestamp(
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
        clip.plays,
        clip.last_played,
        clip.uuid,
    )
    .execute(&mut *connection)
    .await
    .map_or_else(
        |e| Err(crate::Error::Database(e)),
        |update| Ok(update.rows_affected()),
    )
}

/// Get the last time a clip was played.
///
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
pub async fn last_play_time(connection: &mut PgConnection) -> NaiveDateTime {
    let clip_query = sqlx::query!(
        "
        SELECT last_played
        FROM clips
        ORDER BY last_played DESC
        LIMIT 1"
    )
    .fetch_one(&mut *connection)
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
#[instrument(skip(connection))]
pub async fn match_phrase(
    connection: &mut PgConnection,
    phrase: &str,
) -> Result<Vec<Clip>, crate::Error> {
    let clips = clips_list(connection).await?;
    let phrases = super::list_phrases(connection).await?;

    // And As It Is Such, So Also As Such Is It Unto You
    if phrase.contains("random") {
        return Ok(clips);
    }

    let mut matching_clips = Vec::new();
    for clip in clips {
        if phrase.contains(&clip.phrase) && clip.phrase.split_whitespace().count() > 2_usize {
            info!("Matched on '{}' based on the in-clip audio", &clip.phrase);
            matching_clips.push(clip);
        }
    }
    let synonym = synonym(phrase);
    info!("Considering {} as a synonym", synonym);
    for potential_phrase in phrases {
        if phrase.contains(&potential_phrase.phrase) || synonym.contains(&potential_phrase.phrase) {
            let clip = get_clip(connection, potential_phrase.clip).await?;
            matching_clips.push(clip);
            info!("Matched on '{}'", &potential_phrase);
        }
    }
    Ok(matching_clips)
}

fn synonym(phrase: &str) -> String {
    let mut new_phrase = String::new();
    for word in phrase.split_whitespace() {
        let synonyms = thesaurus::synonyms(word);

        new_phrase.push_str(
            &synonyms
                .into_iter()
                .choose(&mut rand::thread_rng())
                .map(|mut w| {
                    w.push(' ');
                    w
                })
                .unwrap_or_else(String::new),
        );
    }
    new_phrase
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
    metadata: ClipUpload,
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

    for phrase in metadata.phrases.unwrap_or_default() {
        super::add_phrase(&mut *connection, clip.uuid, &phrase).await?;
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
        DELETE FROM clip_phrases
        WHERE clip = $1
        ",
        uuid
    )
    .execute(&mut *connection)
    .await
    .map(|deleted| deleted.rows_affected())?;
    tracing::Span::current().record("phrases_deleted", &phrases_deleted);

    for phrase in phrases {
        super::add_phrase(connection, uuid, phrase.as_ref()).await?;
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
pub async fn clips_list(connection: &mut PgConnection) -> Result<Vec<Clip>, crate::Error> {
    sqlx::query_as!(
        Clip,
        "
        SELECT *
        FROM clips;
        "
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(crate::Error::Database)
}
