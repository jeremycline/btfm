use axum::{extract::Extension, Json};
use hyper::StatusCode;
use sqlx::SqlitePool;

use btfm_api_structs::Status;
use tracing::{error, instrument};

/// Reports on the health of the web server.
#[instrument(skip(db_pool))]
pub async fn get(Extension(db_pool): Extension<SqlitePool>) -> Result<Json<Status>, StatusCode> {
    match db_pool.acquire().await {
        Ok(_conn) => Ok(Status {
            db_connections: db_pool.size(),
        }
        .into()),
        Err(err) => {
            error!("Database is unavailable: {:?}", err);
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}
