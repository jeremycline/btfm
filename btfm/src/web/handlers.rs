use axum::{
    extract::{ContentLengthLimit, Extension, Multipart, Path},
    Json,
};
use hyper::StatusCode;
use sqlx::{postgres::PgConnectionInfo, types::Uuid, PgPool};
use tracing::{error, info, instrument};
use ulid::Ulid;

use crate::db;

use btfm_api_structs::{
    Clip, ClipUpdated, ClipUpload, Clips, CreatePhrase, Phrase, Phrases, Status,
};

use super::serialization::{db_phrases_to_api, load_phrases};

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
    let mut conn = db_pool.begin().await?;
    for clip in db::clips_list(&mut conn).await? {
        let mut conn = db_pool.acquire().await?;
        let mut clip: Clip = clip.into();
        load_phrases(&mut clip, &mut conn).await?;
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
    load_phrases(&mut clip, &mut conn).await?;
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
            load_phrases(&mut clip, &mut transaction).await?;
            transaction.commit().await?;
            Ok(clip.into())
        }
        _ => Err(crate::Error::BadRequest),
    }
}

#[instrument(skip(db_pool))]
pub async fn edit_clip(
    Extension(db_pool): Extension<PgPool>,
    Path(ulid): Path<Ulid>,
    Json(clip_metadata): Json<ClipUpload>,
) -> Result<Json<ClipUpdated>, crate::Error> {
    let uuid = Uuid::from_u128(ulid.0);
    let mut transaction = db_pool.begin().await?;

    let mut old_clip: Clip = db::get_clip(&mut transaction, uuid).await?.into();
    load_phrases(&mut old_clip, &mut transaction).await?;

    let description = match clip_metadata.description.is_empty() {
        true => &old_clip.description,
        false => &clip_metadata.description,
    };

    db::update_clip(
        &mut transaction,
        uuid,
        description,
        &clip_metadata.phrases.unwrap_or_default(),
    )
    .await?;

    let mut new_clip: Clip = db::get_clip(&mut transaction, uuid).await?.into();
    load_phrases(&mut new_clip, &mut transaction).await?;

    transaction.commit().await?;
    Ok(ClipUpdated { old_clip, new_clip }.into())
}

#[instrument(skip(db_pool))]
pub async fn delete_clip(
    Extension(db_pool): Extension<PgPool>,
    Path(ulid): Path<Ulid>,
) -> Result<Json<Clip>, crate::Error> {
    let uuid = Uuid::from_u128(ulid.0);
    let mut conn = db_pool.begin().await?;
    let clip: Clip = db::remove_clip(&mut conn, uuid).await?.into();
    Ok(clip.into())
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
    let mut conn = db_pool.begin().await?;
    let phrases: Phrases = db_phrases_to_api(db::list_phrases(&mut conn).await?);
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
    let phrase: Phrase = db::add_phrase(&mut conn, clip_uuid, &phrase_upload.phrase)
        .await?
        .into();
    Ok(phrase.into())
}

/// Show the phrase associated with a given Ulid.
#[instrument(skip(db_pool))]
pub async fn delete_phrase(
    Extension(db_pool): Extension<PgPool>,
    Path(ulid): Path<Ulid>,
) -> Result<Json<Phrase>, crate::Error> {
    let uuid = Uuid::from_u128(ulid.0);
    let mut conn = db_pool.begin().await?;
    let phrase: Phrase = db::remove_phrase(&mut conn, uuid).await?.into();
    Ok(phrase.into())
}
