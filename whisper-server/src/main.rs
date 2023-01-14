use std::{io::Write, net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::Response,
    Router,
};
// SPDX-License-Identifier: GPL-2.0-or-later
use clap::Parser;
use pyo3::{types::PyModule, Python};
use tokio::sync::{mpsc, oneshot};
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    request_id::MakeRequestUuid,
    trace::{DefaultMakeSpan, DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer},
    ServiceBuilderExt,
};
use tracing::{instrument, Instrument, Level};

const WHISPER: &str = include_str!("transcribe.py");

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long, env = "WHISPER_MODEL")]
    model: PathBuf,
    #[arg(short, long, env = "WHISPER_LISTEN_ADDRESS")]
    listen_address: SocketAddr,
}

#[derive(Debug, Clone)]
struct AppState {
    transcriber: mpsc::Sender<(PathBuf, oneshot::Sender<String>)>,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    tracing_subscriber::fmt::init();

    let opts = Cli::parse();

    let (tx, rx) = tokio::sync::mpsc::channel(64);

    let transcriber = tokio::task::spawn_blocking(|| transcribe(opts.model, rx));
    let router = router(Arc::new(AppState { transcriber: tx }));
    let service = router.into_make_service();
    tracing::info!("Listening on {:?}", &opts.listen_address);
    axum::Server::bind(&opts.listen_address)
        .serve(service)
        .with_graceful_shutdown(shutdown_handler(transcriber))
        .await
        .unwrap();
}

async fn shutdown_handler(transcriber: tokio::task::JoinHandle<()>) {
    let _shutdown_signal = tokio::signal::ctrl_c().await;
    tracing::info!("Shutdown signal received; beginning shutdown");
    transcriber.abort();
}

fn transcribe(
    model: PathBuf,
    mut audio_receiver: mpsc::Receiver<(PathBuf, oneshot::Sender<String>)>,
) {
    Python::with_gil(|py| {
        let module = PyModule::from_code(py, WHISPER, "transcribe.py", "transcribe").unwrap();

        let load_model = module.getattr("load_model").unwrap();
        load_model.call1((model,)).unwrap();

        let transcriber = module.getattr("transcribe").unwrap();

        while let Some((audio, sender)) = audio_receiver.blocking_recv() {
            let result = transcriber
                .call1((audio,))
                .and_then(|r| r.extract())
                .unwrap_or_default();
            if sender.send(result).is_err() {
                tracing::error!("Failed to send STT result to the web handler");
            }
        }
    });
}

fn router(state: Arc<AppState>) -> Router {
    let app = Router::new().route("/v1/listen", axum::routing::get(listen));

    // Ordering matters here; requests pass through middleware top-to-bottom and responses bottom-to-top
    let middleware = ServiceBuilder::new()
        .set_x_request_id(MakeRequestUuid)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().include_headers(true))
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO))
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        )
        .propagate_x_request_id()
        .layer(CompressionLayer::new());

    app.layer(middleware).with_state(state)
}

#[instrument(skip_all)]
async fn listen(State(state): State<Arc<AppState>>, socket: WebSocketUpgrade) -> Response {
    let span = tracing::Span::current();
    socket.on_upgrade(|socket| handle_websocket(socket, state).instrument(span))
}

#[instrument(skip_all)]
async fn handle_websocket(mut socket: WebSocket, state: Arc<AppState>) {
    let tmpfile = tempfile::NamedTempFile::new();
    if tmpfile.is_err() {
        return;
    }
    let mut tmpfile = tmpfile.unwrap();
    let tmppath = tmpfile.path().to_path_buf();
    while let Some(message) = socket.recv().await {
        let message = if let Ok(message) = message {
            message
        } else {
            return;
        };

        match message {
            axum::extract::ws::Message::Binary(data) => {
                if data.is_empty() {
                    let (tx, rx) = oneshot::channel();
                    if state.transcriber.send((tmppath, tx)).await.is_err() {
                        tracing::error!("Transcriber channel is down");
                        return;
                    }
                    let result = rx.await.unwrap();
                    if socket.send(Message::Text(result)).await.is_err() {
                        return;
                    }
                    if socket.close().await.is_err() {
                        return;
                    }
                    break;
                } else if tmpfile.write_all(&data).is_err() {
                    tracing::error!("Failed to write to temp file");
                }
            }
            axum::extract::ws::Message::Text(_) => {
                let (tx, rx) = oneshot::channel();
                let _ = state.transcriber.send((tmppath, tx)).await;
                let result = rx.await.unwrap_or_default();
                let _ = socket.send(Message::Text(result)).await;
                let _ = socket.close().await;
                break;
            }
            _ => {}
        }
    }
}
