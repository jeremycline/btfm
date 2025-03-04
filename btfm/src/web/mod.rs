use axum::{
    body::Body,
    extract::Extension,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use hyper::header;
use serde_json::json;
use sqlx::SqlitePool;
use tower::ServiceBuilder;
use tower_http::{
    auth::AddAuthorizationLayer,
    compression::CompressionLayer,
    request_id::{MakeRequestId, RequestId},
    sensitive_headers::SetSensitiveHeadersLayer,
    trace::{DefaultMakeSpan, DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer},
    ServiceBuilderExt,
};
use tracing::Level;
use uuid::Uuid;

use crate::{config::HttpApi, transcribe::Transcriber, Error};

pub(crate) mod handlers;
pub(crate) mod serialization;

const SENSITIVE_HEADERS: [header::HeaderName; 1] = [header::AUTHORIZATION];

#[derive(Clone, Copy)]
struct MakeRequestUlid;

impl MakeRequestId for MakeRequestUlid {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        let request_id = Uuid::new_v4().to_string().parse().unwrap();
        Some(RequestId::new(request_id))
    }
}

/// Create an Axum router configured with middleware.
pub fn create_router(config: &HttpApi, db: SqlitePool, transcriber: Transcriber) -> Router {
    let app = Router::new()
        .route("/status/", get(handlers::status::get))
        .route("/v1/clips/{uuid}/phrases/", get(handlers::phrase::by_clip))
        .route(
            "/v1/clips/{uuid}",
            get(handlers::clip::get)
                .delete(handlers::clip::delete)
                .put(handlers::clip::edit),
        )
        .route("/v1/clips/{uuid}/audio", get(handlers::clip::download_clip))
        .route(
            "/v1/clips/",
            get(handlers::clip::get_all).post(handlers::clip::create),
        )
        .route(
            "/v1/phrases/{uuid}",
            get(handlers::phrase::get).delete(handlers::phrase::delete),
        )
        .route(
            "/v1/phrases/",
            get(handlers::phrase::get_all).post(handlers::phrase::create),
        )
        .fallback(handle_404)
        .layer(Extension(db))
        .layer(Extension(transcriber));

    // Ordering matters here; requests pass through middleware top-to-bottom and responses bottom-to-top
    let middleware = ServiceBuilder::new()
        .layer(SetSensitiveHeadersLayer::new(SENSITIVE_HEADERS))
        .set_x_request_id(MakeRequestUlid)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().include_headers(true))
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO))
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        )
        .propagate_x_request_id()
        .layer(AddAuthorizationLayer::basic(&config.user, &config.password))
        .layer(CompressionLayer::new());

    app.layer(middleware)
}

async fn handle_404() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        "This isn't the endpoint you're looking for",
    )
}

impl IntoResponse for Error {
    fn into_response(self) -> Response<Body> {
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
