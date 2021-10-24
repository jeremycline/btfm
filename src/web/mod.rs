use axum::{
    body::BoxBody,
    handler::Handler,
    http::{Response, StatusCode},
    response::IntoResponse,
    routing::get,
    AddExtensionLayer, Json, Router,
};
use serde_json::json;
use sqlx::PgPool;

use crate::Error;

pub(crate) mod handlers;
pub(crate) mod serialization;

pub fn create(db: PgPool) -> Router {
    Router::new()
        .route("/status/", get(handlers::status))
        .route("/v1/clips/:ulid/", get(handlers::clip))
        .route(
            "/v1/clips/",
            get(handlers::clips).post(handlers::create_clip),
        )
        .route("/v1/phrases/", get(handlers::phrases))
        .fallback(handle_404.into_service())
        .layer(AddExtensionLayer::new(db))
}

async fn handle_404() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        "This isn't the endpoint you're looking for",
    )
}

impl IntoResponse for Error {
    fn into_response(self) -> Response<BoxBody> {
        let (status, error_message) = match self {
            Error::Database(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "The database is unavailable",
            ),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "Something went oopsies"),
        };

        let body = Json(json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}
