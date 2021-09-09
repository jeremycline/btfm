// SPDX-License-Identifier: GPL-2.0-or-later
use clap::arg_enum;
use thiserror::Error as ThisError;

/// An enumeration of errors BTFM library functions can encounter.
#[derive(ThisError, Debug)]
pub enum Error {
    #[error("A database error occurred: {0}")]
    Database(sqlx::Error),
    #[error("Transcriber failed to respond to request")]
    TranscriberGone,
    #[error("Configuration file could not be read: {0}")]
    ConfigReadError(#[from] std::io::Error),
    #[error("Configuration file could not be parsed: {0}")]
    ConfigParseError(#[from] toml::de::Error),
    #[error("Invalid backend provided")]
    BackendParseError,
}

arg_enum! {
#[derive(Debug)]
pub enum Backend {
    DeepSpeechCpu,
    DeepSpeechGpu,
    Deepgram,
}
}

impl Default for Backend {
    fn default() -> Self {
        Backend::DeepSpeechCpu
    }
}

pub mod cli;
pub mod config;
pub mod db;
pub mod discord;
pub mod transcode;
pub mod transcribe;

pub const DB_NAME: &str = "btfm.sqlite3";
