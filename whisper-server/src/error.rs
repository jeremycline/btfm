use thiserror::Error as ThisError;

#[derive(ThisError, Debug)]
pub enum Error {
    #[error("Tokio task failed: {0}")]
    TokioTask(#[from] tokio::task::JoinError),
    #[error("An unexpected error occurred from the Python module: {0}")]
    Python(#[from] pyo3::PyErr),
    #[error("An IO error occurred: {0}")]
    Io(#[from] std::io::Error),
    #[error("An unhandled Axum error occurred: {0}")]
    Axum(#[from] axum::Error),
    #[error("The transcriber worker is gone")]
    TranscriberGone,
}
