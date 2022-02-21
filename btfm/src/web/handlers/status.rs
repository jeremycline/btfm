use axum::{extract::Extension, Json};
use hyper::StatusCode;
use sqlx::{postgres::PgConnectionInfo, PgPool};

use btfm_api_structs::Status;
use tracing::{error, instrument};

/// Reports on the health of the web server.
#[instrument(skip(db_pool))]
pub async fn get(Extension(db_pool): Extension<PgPool>) -> Result<Json<Status>, StatusCode> {
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
