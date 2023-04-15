// SPDX-License-Identifier: GPL-2.0-or-later
use once_cell::sync::OnceCell;
use serde::Serializer;
use sqlx::types::Uuid;
use thiserror::Error as ThisError;

pub static CONFIG: OnceCell<config::Config> = OnceCell::new();

/// An enumeration of errors BTFM library functions can encounter.
#[derive(ThisError, Debug)]
pub enum Error {
    #[error("A database error occurred: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Transcriber failed to respond to request")]
    TranscriberGone,
    #[error("A transcoding error occurred in GStreamer")]
    Trancode(#[from] gstreamer::glib::Error),
    #[error("Configuration file could not be read: {0}")]
    ConfigReadError(#[from] std::io::Error),
    #[error("Configuration file could not be parsed: {0}")]
    ConfigParseError(#[from] toml::de::Error),
    #[error("Configuration file contains invalid values: {0}")]
    ConfigValueError(String),
    #[error("The Discord client encountered an error: {0}")]
    Serenity(#[from] serenity::Error),
    #[error("Unable to generate voice Input: {0}")]
    SongbirdInput(#[from] songbird::input::error::Error),
    #[error("HTTP server encountered an error: {0}")]
    Server(std::io::Error),
    #[error("Tokio task failed: {0}")]
    TokioTask(#[from] tokio::task::JoinError),
    #[error("Client request is invalid")]
    BadRequest,
    #[error("File not found")]
    NotFound,
    #[error("An HTTP error occurred")]
    Http(#[from] reqwest::Error),
    #[error("A Url parsing error occurred")]
    ParseUrl(#[from] url::ParseError),
    #[error("An unexpected error occurred from the Python module: {0}")]
    Python(#[from] pyo3::PyErr),
}

/// Serializer for UUIDs
pub fn uuid_serializer<S>(uuid: &Uuid, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    s.serialize_str(&uuid.to_string())
}

pub mod cli;
pub mod config;
pub mod db;
pub mod discord;
pub(crate) mod mimic;
pub mod transcode;
pub mod transcribe;
pub mod web;
