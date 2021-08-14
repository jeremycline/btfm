// SPDX-License-Identifier: GPL-2.0-or-later
use thiserror::Error as ThisError;

/// An enumeration of errors BTFM library functions can encounter.
#[derive(ThisError, Debug)]
pub enum Error {
    #[error("A database error occurred: {0}")]
    Database(sqlx::Error),
    #[error("Transcriber failed to respond to request")]
    TranscriberGone,
}

pub mod cli;
pub mod db;
mod transcode;
mod transcribe;
pub mod voice;

pub const DB_NAME: &str = "btfm.sqlite3";
