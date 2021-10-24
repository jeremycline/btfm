use axum::{
    extract::{ContentLengthLimit, Extension, Multipart, Path},
    Json,
};
use chrono::NaiveDateTime;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgConnectionInfo, types::Uuid, PgPool};
use tracing::{error, info, instrument};
use ulid::Ulid;

use crate::db;

use super::serialization;

#[derive(Serialize)]
pub struct Status {
    db_version: Option<u32>,
    db_connections: u32,
}

#[derive(Debug, Serialize)]
pub struct Phrase {
    /// The unique identifier for the phrase
    pub uuid: String,
    /// The phrase that triggers any clips associated with this phrase.
    pub phrase: String,
}

#[derive(Debug, Serialize)]
pub struct Clip {
    /// The unique identifier for the clip.
    pub uuid: String,
    /// The time when the clip was added to the database.
    pub created_on: NaiveDateTime,
    /// The last time the clip was played; this is equal to `created_on` when created.
    pub last_played: NaiveDateTime,
    /// Number of times the clip has been played.
    pub plays: u64,
    /// The output of speech-to-text on the `audio_file`.
    pub clip_text: String,
    /// A description of the clip for human consumption.
    pub description: String,
    /// The phrases associated with the clip
    pub phrases: Vec<db::Phrase>,
}

#[derive(Debug, Serialize)]
pub struct Clips {
    items: u64,
    clips: Vec<Clip>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PhraseUpload {
    pub phrase: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ClipUpload {
    pub description: String,
    pub phrases: Option<Vec<String>>,
}

/// Reports on the health of the web server.
#[instrument(skip(db_pool))]
pub async fn status(Extension(db_pool): Extension<PgPool>) -> Result<Json<Status>, StatusCode> {
    match db_pool.acquire().await {
        Ok(conn) => Ok(Status {
            db_version: conn.server_version_num(),
            db_connections: db_pool.size(),
        }
        .into()),
        Err(err) => {
            error!("Database is unavailable: {:?}", err);
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

/// List clips known to BTFM
#[instrument(skip(db_pool))]
pub async fn clips(Extension(db_pool): Extension<PgPool>) -> Result<Json<Clips>, crate::Error> {
    let mut clips = vec![];
    for clip in db::Clip::list(&db_pool).await? {
        let mut conn = db_pool.acquire().await?;
        let phrases = db::phrases_for_clip(&mut conn, clip.uuid).await?;
        clips.push(Clip {
            uuid: clip.uuid.to_string(),
            created_on: clip.created_on,
            last_played: clip.last_played,
            plays: clip.plays as u64,
            clip_text: clip.phrase,
            description: clip.description,
            phrases,
        })
    }
    Ok(Clips {
        items: clips.len() as u64,
        clips,
    }
    .into())
}

/// Get a single clip by ID.
#[instrument(skip(db_pool))]
pub async fn clip(
    Extension(db_pool): Extension<PgPool>,
    Path(ulid): Path<Ulid>,
) -> Result<Json<serialization::Clip>, crate::Error> {
    let uuid = Uuid::from_u128(ulid.0);
    let mut conn = db_pool.begin().await?;
    let mut clip: serialization::Clip = db::get_clip(&mut conn, uuid)
        .await
        .map(|clip| clip.into())?;
    clip.phrases = Some(
        db::phrases_for_clip(&mut conn, uuid)
            .await?
            .into_iter()
            .map(|p| p.into())
            .collect::<Vec<serialization::Phrase>>(),
    );
    Ok(clip.into())
}

/// Create a new clip.
///
/// This accepts a multipart form consisting of two parts. The first part is a JSON object
/// with clip metadata and optional phrases to associate with it. The second part is the file
/// itself.
#[instrument(skip(db_pool))]
pub async fn create_clip(
    Extension(db_pool): Extension<PgPool>,
    ContentLengthLimit(mut form): ContentLengthLimit<Multipart, { 50 * 1024 * 1024 }>,
) -> Result<Json<serialization::Clip>, crate::Error> {
    let mut clip_metadata = None;
    let mut filename = None;
    let mut clip_data = None;
    while let Some(field) = form.next_field().await.unwrap() {
        match field.name().unwrap() {
            "clip_metadata" => {
                clip_metadata =
                    Some(serde_json::from_str::<ClipUpload>(&field.text().await.unwrap()).unwrap());
            }
            "clip" => {
                // Validate and write to filesystem
                filename = Some(field.file_name().unwrap().to_owned());
                clip_data = Some(field.bytes().await.unwrap());
            }
            _ => {
                info!(?field, "Ignoring unknown field");
            }
        }
    }
    match (clip_metadata, clip_data, filename) {
        (Some(metadata), Some(data), Some(filename)) => {
            let mut transaction = db_pool.begin().await?;
            let mut clip: serialization::Clip =
                db::add_clip(&mut transaction, data.to_vec(), metadata, &filename)
                    .await?
                    .into();
            clip.phrases = Some(
                db::phrases_for_clip(&mut transaction, Uuid::from_u128(clip.ulid.0))
                    .await?
                    .into_iter()
                    .map(|p| p.into())
                    .collect::<Vec<serialization::Phrase>>(),
            );
            transaction.commit().await?;
            Ok(clip.into())
        }
        _ => Err(crate::Error::BadRequest),
    }
}

#[derive(Serialize)]
pub struct Phrases {
    items: u64,
    phrases: Vec<serialization::Phrase>,
}

/// List clips known to BTFM
#[instrument(skip(db_pool))]
pub async fn phrases(Extension(db_pool): Extension<PgPool>) -> Result<Json<Phrases>, crate::Error> {
    let phrases = db::Phrase::list(&db_pool)
        .await?
        .into_iter()
        .map(|p| p.into())
        .collect::<Vec<serialization::Phrase>>();
    Ok(Phrases {
        items: phrases.len() as u64,
        phrases,
    }
    .into())
}
