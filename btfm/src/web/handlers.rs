use axum::{
    extract::{ContentLengthLimit, Extension, Multipart, Path},
    Json,
};
use hyper::StatusCode;
use sqlx::{postgres::PgConnectionInfo, types::Uuid, PgPool};
use tracing::{error, info, instrument};
use ulid::Ulid;

use crate::db;

use super::serialization::{Clip, ClipUpload, Clips, CreatePhrase, Phrase, Phrases, Status};

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
        let mut clip: Clip = clip.into();
        clip.load_phrases(&mut conn).await?;
        clips.push(clip);
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
) -> Result<Json<Clip>, crate::Error> {
    let uuid = Uuid::from_u128(ulid.0);
    let mut conn = db_pool.begin().await?;
    let mut clip: Clip = db::get_clip(&mut conn, uuid).await?.into();
    clip.load_phrases(&mut conn).await?;
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
) -> Result<Json<Clip>, crate::Error> {
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
            let mut clip: Clip = db::add_clip(&mut transaction, data.to_vec(), metadata, &filename)
                .await?
                .into();
            clip.load_phrases(&mut transaction).await?;
            transaction.commit().await?;
            Ok(clip.into())
        }
        _ => Err(crate::Error::BadRequest),
    }
}

/// Show the phrase associated with a given Ulid.
#[instrument(skip(db_pool))]
pub async fn phrase(
    Extension(db_pool): Extension<PgPool>,
    Path(ulid): Path<Ulid>,
) -> Result<Json<Phrase>, crate::Error> {
    let uuid = Uuid::from_u128(ulid.0);
    let mut conn = db_pool.begin().await?;
    let phrase: Phrase = db::get_phrase(&mut conn, uuid).await?.into();
    Ok(phrase.into())
}

/// List phrases known to BTFM
#[instrument(skip(db_pool))]
pub async fn phrases(Extension(db_pool): Extension<PgPool>) -> Result<Json<Phrases>, crate::Error> {
    let phrases: Phrases = db::Phrase::list(&db_pool).await?.into();
    Ok(phrases.into())
}

/// Create a new trigger phrase for a clip.
#[instrument(skip(db_pool))]
pub async fn create_phrase(
    Extension(db_pool): Extension<PgPool>,
    Json(phrase_upload): Json<CreatePhrase>,
) -> Result<Json<Phrase>, crate::Error> {
    let clip_uuid = Uuid::from_u128(phrase_upload.clip.0);
    let mut conn = db_pool.begin().await?;
    let phrase: Phrase = db::add_phrase(&mut conn, &phrase_upload.phrase, clip_uuid)
        .await?
        .into();
    Ok(phrase.into())
}
