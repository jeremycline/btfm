use axum::{
    extract::{Extension, Multipart, Path},
    response::IntoResponse,
    Json,
};
use btfm_api_structs::{Clip, ClipUpdated, ClipUpload, Clips};
use sqlx::{types::Uuid, SqlitePool};
use tokio_util::io::ReaderStream;
use tracing::{info, instrument};

use crate::web::serialization::load_phrases;
use crate::{db, transcribe::Transcriber};

/// List clips known to BTFM
#[instrument(skip(db_pool))]
pub async fn get_all(
    Extension(db_pool): Extension<SqlitePool>,
) -> Result<Json<Clips>, crate::Error> {
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
    Extension(db_pool): Extension<SqlitePool>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Clip>, crate::Error> {
    let uuid = uuid.to_string();
    let mut conn = db_pool.begin().await?;
    let mut clip: Clip = db::get_clip(&mut conn, uuid).await?.into();
    load_phrases(&mut clip, &mut conn).await?;
    Ok(clip.into())
}

pub async fn download_clip(
    Extension(db_pool): Extension<SqlitePool>,
    Path(uuid): Path<Uuid>,
) -> Result<impl IntoResponse, crate::Error> {
    let uuid = uuid.to_string();
    let mut conn = db_pool.begin().await?;
    let clip: Clip = db::get_clip(&mut conn, uuid).await.unwrap().into();
    let config = crate::CONFIG.get().expect("Initialize the config");
    let clip_path = config.data_directory.join(&clip.audio_file);

    let clip_file = tokio::fs::File::open(&clip_path)
        .await
        .map_err(|_| crate::Error::NotFound)?;
    let body = axum::body::Body::from_stream(ReaderStream::new(clip_file));
    let filename = clip_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let headers = [
        (
            axum::http::header::CONTENT_TYPE,
            "application/octet-stream".to_string(),
        ),
        (
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", filename),
        ),
    ];

    Ok((headers, body))
}

/// Create a new clip.
///
/// This accepts a multipart form consisting of two parts. The first part is a JSON object
/// with clip metadata and optional phrases to associate with it. The second part is the file
/// itself.
#[instrument(skip(db_pool, transcriber))]
pub async fn create(
    Extension(db_pool): Extension<SqlitePool>,
    Extension(transcriber): Extension<Transcriber>,
    mut form: Multipart,
) -> Result<Json<Clip>, crate::Error> {
    let mut clip_metadata = None;
    let mut filename = None;
    let mut clip_data = None;
    while let Some(field) = form.next_field().await? {
        match field.name().ok_or(crate::Error::BadRequest)? {
            "clip_metadata" => {
                clip_metadata = Some(serde_json::from_str::<ClipUpload>(&field.text().await?)?);
            }
            "clip" => {
                // Validate and write to filesystem
                filename = Some(
                    field
                        .file_name()
                        .ok_or(crate::Error::BadRequest)?
                        .to_owned(),
                );
                clip_data = Some(field.bytes().await?);
            }
            _ => {
                info!(?field, "Ignoring unknown field");
            }
        }
    }
    match (clip_metadata, clip_data, filename) {
        (Some(metadata), Some(data), Some(filename)) => {
            let mut transaction = db_pool.begin().await?;
            let mut clip: Clip = db::add_clip(
                &mut transaction,
                data.to_vec(),
                metadata,
                &filename,
                transcriber,
            )
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
    Extension(db_pool): Extension<SqlitePool>,
    Path(uuid): Path<Uuid>,
    Json(clip_metadata): Json<ClipUpload>,
) -> Result<Json<ClipUpdated>, crate::Error> {
    let uuid = uuid.to_string();
    let mut transaction = db_pool.begin().await?;

    let mut old_clip: Clip = db::get_clip(&mut transaction, uuid.clone()).await?.into();
    load_phrases(&mut old_clip, &mut transaction).await?;

    let description = match clip_metadata.description.is_empty() {
        true => &old_clip.description,
        false => &clip_metadata.description,
    };

    db::update_clip(
        &mut transaction,
        uuid.clone(),
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
    Extension(db_pool): Extension<SqlitePool>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Clip>, crate::Error> {
    let uuid = uuid.to_string();
    let mut transaction = db_pool.begin().await?;
    let clip: Clip = db::remove_clip(&mut transaction, uuid).await?.into();
    transaction.commit().await?;
    Ok(clip.into())
}
