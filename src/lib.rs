// SPDX-License-Identifier: GPL-2.0-or-later
#[macro_use]
extern crate diesel;

pub mod cli;
pub mod db;
pub mod models;
pub mod schema;
pub mod voice;

pub const DB_NAME: &str = "btfm.sqlite3";
