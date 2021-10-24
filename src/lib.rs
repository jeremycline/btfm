// SPDX-License-Identifier: GPL-2.0-or-later
use clap::arg_enum;
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
    #[error("Configuration file could not be read: {0}")]
    ConfigReadError(#[from] std::io::Error),
    #[error("Configuration file could not be parsed: {0}")]
    ConfigParseError(#[from] toml::de::Error),
    #[error("Invalid backend provided")]
    BackendParseError,
    #[error("Client request is invalid")]
    BadRequest,
}

arg_enum! {
#[derive(Debug)]
pub enum Backend {
    DeepSpeech,
    Deepgram,
}
}

impl Default for Backend {
    fn default() -> Self {
        Backend::DeepSpeech
    }
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
pub mod transcode;
pub mod transcribe;
pub mod web;
