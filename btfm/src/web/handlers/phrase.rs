use axum::{
    extract::{Extension, Path},
    Json,
};
use sqlx::{types::Uuid, SqlitePool};
use tracing::instrument;

use crate::db;
use crate::web::serialization::db_phrases_to_api;

use btfm_api_structs::{CreatePhrase, Phrase, Phrases};

/// Show the phrase associated with a given Ulid.
#[instrument(skip(db_pool))]
pub async fn get(
    Extension(db_pool): Extension<SqlitePool>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Phrase>, crate::Error> {
    let uuid = uuid.to_string();
    let mut conn = db_pool.begin().await?;
    let phrase: Phrase = db::get_phrase(&mut conn, uuid).await?.into();
    Ok(phrase.into())
}

/// List phrases known to BTFM
#[instrument(skip(db_pool))]
pub async fn get_all(
    Extension(db_pool): Extension<SqlitePool>,
) -> Result<Json<Phrases>, crate::Error> {
    let mut conn = db_pool.begin().await?;
    let phrases = db_phrases_to_api(db::list_phrases(&mut conn).await?);
    Ok(phrases.into())
}

/// Get all phrases for a given clip.
#[instrument(skip(db_pool))]
pub async fn by_clip(
    Extension(db_pool): Extension<SqlitePool>,
    Path(clip_uuid): Path<Uuid>,
) -> Result<Json<Phrases>, crate::Error> {
    let clip_uuid = clip_uuid.to_string();
    let mut conn = db_pool.begin().await?;
    let phrases = db_phrases_to_api(db::phrases_for_clip(&mut conn, clip_uuid).await?);
    Ok(phrases.into())
}

/// Create a new trigger phrase for a clip.
#[instrument(skip(db_pool))]
pub async fn create(
    Extension(db_pool): Extension<SqlitePool>,
    Json(phrase_upload): Json<CreatePhrase>,
) -> Result<Json<Phrase>, crate::Error> {
    let clip_uuid = phrase_upload.clip.to_string();
    let mut conn = db_pool.begin().await?;
    let phrase: Phrase = db::add_phrase(&mut conn, clip_uuid, &phrase_upload.phrase)
        .await?
        .into();
    Ok(phrase.into())
}

/// Show the phrase associated with a given Ulid.
#[instrument(skip(db_pool))]
pub async fn delete(
    Extension(db_pool): Extension<SqlitePool>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Phrase>, crate::Error> {
    let uuid = uuid.to_string();
    let mut conn = db_pool.begin().await?;
    let phrase: Phrase = db::remove_phrase(&mut conn, uuid).await?.into();
    Ok(phrase.into())
}
