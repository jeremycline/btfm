use axum::{
    body::BoxBody,
    handler::Handler,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
    routing::get,
    AddExtensionLayer, Json, Router,
};
use hyper::Body;
use serde_json::json;
use sqlx::PgPool;
use tower_http::{
    trace::{DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer},
    LatencyUnit,
};
use tracing::Level;
use ulid::Ulid;

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
        .route("/v1/phrases/:ulid/", get(handlers::phrase))
        .route(
            "/v1/phrases/",
            get(handlers::phrases).post(handlers::create_phrase),
        )
        .fallback(handle_404.into_service())
        .layer(AddExtensionLayer::new(db))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<Body>| {
                    tracing::info_span!("request",
                        id = %Ulid::new(),
                        method = %request.method(),
                        uri = %request.uri(),
                        version = ?request.version(),
                        header = ?request.headers(),
                    )
                })
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(
                    DefaultOnResponse::new()
                        .level(Level::INFO)
                        .latency_unit(LatencyUnit::Micros),
                )
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        )
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
