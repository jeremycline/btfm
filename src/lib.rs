// SPDX-License-Identifier: GPL-2.0-or-later
use thiserror::Error as ThisError;

/// An enumeration of errors BTFM library functions can encounter.
#[derive(ThisError, Debug)]
pub enum Error {
    #[error("A database error occurred: {0}")]
    Database(sqlx::Error),
}

pub mod cli;
pub mod db;
pub mod voice;

pub const DB_NAME: &str = "btfm.sqlite3";
