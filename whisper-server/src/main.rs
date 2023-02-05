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

mod error;

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
    match axum::Server::bind(&opts.listen_address)
        .serve(service)
        .with_graceful_shutdown(shutdown_handler(transcriber))
        .await
    {
        Ok(_) => {}
        Err(err) => {
            tracing::error!(err = ?err, "Server failed to shut down gracefully");
            std::process::exit(1);
        }
    };
}

async fn shutdown_handler(transcriber: tokio::task::JoinHandle<Result<(), error::Error>>) {
    let _shutdown_signal = tokio::signal::ctrl_c().await;
    tracing::info!("Shutdown signal received; beginning shutdown");
    transcriber.abort();
}

fn transcribe(
    model: PathBuf,
    mut audio_receiver: mpsc::Receiver<(PathBuf, oneshot::Sender<String>)>,
) -> Result<(), error::Error> {
    Python::with_gil(|py| {
        let module = PyModule::from_code(py, WHISPER, "transcribe.py", "transcribe")?;

        let load_model = module.getattr("load_model")?;
        load_model.call1((model,))?;

        let transcriber = module.getattr("transcribe")?;

        while let Some((audio, sender)) = audio_receiver.blocking_recv() {
            let result = transcriber
                .call1((audio,))
                .and_then(|r| r.extract())
                .unwrap_or_default();
            if sender.send(result).is_err() {
                tracing::error!("Failed to send STT result to the web handler");
            }
        }

        Ok(())
    })
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
async fn handle_websocket(socket: WebSocket, state: Arc<AppState>) {
    if let Err(err) = handle_websocket_with_result(socket, state).await {
        tracing::error!(err = ?err, "WebSocket failure");
    }
}

async fn handle_websocket_with_result(
    mut socket: WebSocket,
    state: Arc<AppState>,
) -> Result<(), error::Error> {
    let mut tmpfile = tempfile::NamedTempFile::new()?;
    let tmppath = tmpfile.path().to_path_buf();
    while let Some(message) = socket.recv().await {
        let message = message?;

        match message {
            axum::extract::ws::Message::Binary(data) if data.is_empty() => {
                tmpfile.flush()?;
                let (tx, rx) = oneshot::channel();
                if state.transcriber.send((tmppath, tx)).await.is_err() {
                    tracing::error!("The transcriber is gone!");
                    return Err(error::Error::TranscriberGone);
                }
                let result = match rx.await {
                    Ok(result) => result,
                    Err(_) => {
                        tracing::error!("The transcriber closed the channel without responding");
                        "".into()
                    }
                };
                socket.send(Message::Text(result)).await?;
                socket.close().await?;
                break;
            }
            axum::extract::ws::Message::Binary(data) => {
                tmpfile.write_all(&data)?;
            }
            axum::extract::ws::Message::Text(_) => {
                tmpfile.flush()?;
                let (tx, rx) = oneshot::channel();
                if state.transcriber.send((tmppath, tx)).await.is_err() {
                    tracing::error!("The transcriber is gone!");
                    return Err(error::Error::TranscriberGone);
                }
                let result = match rx.await {
                    Ok(result) => result,
                    Err(_) => {
                        tracing::error!("The transcriber closed the channel without responding");
                        "".into()
                    }
                };
                socket.send(Message::Text(result)).await?;
                socket.close().await?;
                break;
            }
            _ => {}
        }
    }

    Ok(())
}
