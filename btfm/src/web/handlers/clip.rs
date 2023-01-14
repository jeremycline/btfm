use axum::{
    extract::{Extension, Multipart, Path},
    Json,
};
use btfm_api_structs::{Clip, ClipUpdated, ClipUpload, Clips};
use sqlx::{types::Uuid, PgPool};
use tracing::{info, instrument};
use ulid::Ulid;

use crate::db;
use crate::web::serialization::load_phrases;

/// List clips known to BTFM
#[instrument(skip(db_pool))]
pub async fn get_all(Extension(db_pool): Extension<PgPool>) -> Result<Json<Clips>, crate::Error> {
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
pub async fn get(
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
pub async fn create(
    Extension(db_pool): Extension<PgPool>,
    mut form: Multipart,
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
pub async fn edit(
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
pub async fn delete(
    Extension(db_pool): Extension<PgPool>,
    Path(ulid): Path<Ulid>,
) -> Result<Json<Clip>, crate::Error> {
    let uuid = Uuid::from_u128(ulid.0);
    let mut conn = db_pool.begin().await?;
    let clip: Clip = db::remove_clip(&mut conn, uuid).await?.into();
    Ok(clip.into())
}
