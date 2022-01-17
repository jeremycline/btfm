use axum::{
    body::BoxBody,
    handler::Handler,
    http::{Request, Response, StatusCode},
    response::IntoResponse,
    routing::get,
    AddExtensionLayer, Json, Router,
};
use hyper::header;
use serde_json::json;
use sqlx::PgPool;
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
use ulid::Ulid;

use crate::{config::HttpApi, Error};

pub(crate) mod handlers;
pub(crate) mod serialization;

const SENSITIVE_HEADERS: [header::HeaderName; 1] = [header::AUTHORIZATION];

#[derive(Clone, Copy)]
struct MakeRequestUlid;

impl MakeRequestId for MakeRequestUlid {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        let request_id = Ulid::new().to_string().parse().unwrap();
        Some(RequestId::new(request_id))
    }
}

/// Create an Axum router configured with middleware.
pub fn create_router(config: &HttpApi, db: PgPool) -> Router {
    let app = Router::new()
        .route("/status/", get(handlers::status))
        .route(
            "/v1/clips/:ulid/",
            get(handlers::clip)
                .delete(handlers::delete_clip)
                .put(handlers::edit_clip),
        )
        .route(
            "/v1/clips/",
            get(handlers::clips).post(handlers::create_clip),
        )
        .route(
            "/v1/phrases/:ulid/",
            get(handlers::phrase).delete(handlers::delete_phrase),
        )
        .route(
            "/v1/phrases/",
            get(handlers::phrases).post(handlers::create_phrase),
        )
        .fallback(handle_404.into_service())
        .layer(AddExtensionLayer::new(db));

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
