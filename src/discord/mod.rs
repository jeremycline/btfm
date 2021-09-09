//! Implements the Serenity event handlers for voice and text channels.
use std::collections::HashMap;
use std::sync::Arc;

use serenity::prelude::*;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::transcribe::Transcriber;
use crate::Backend;

pub struct BtfmData {
    /// Application configuration
    pub config: Config,
    /// Service to handle transcription requests
    transcriber: Transcriber,
    /// Map ssrcs to Users
    users: HashMap<u32, User>,
    // Map user IDs to ssrc
    ssrc_map: HashMap<u64, u32>,
    // How many times the given user has joined the channel so we can give them rejoin messages.
    pub user_history: HashMap<u64, u32>,
    db: sqlx::PgPool,
}
impl TypeMapKey for BtfmData {
    type Value = Arc<Mutex<BtfmData>>;
}
impl BtfmData {
    pub async fn new(config: Config, backend: Backend) -> BtfmData {
        let db = PgPoolOptions::new()
            .max_connections(10)
            .connect(&config.database_url)
            .await
            .expect("Unable to connect to database");
        let transcriber = Transcriber::new(&config, &backend);
        BtfmData {
            config,
            transcriber,
            users: HashMap::new(),
            ssrc_map: HashMap::new(),
            user_history: HashMap::new(),
            db,
        }
    }
}

/// Represents an active user in a voice channel.
struct User {
    transcriber: Option<mpsc::Sender<Vec<i16>>>,
    speaking: bool,
}

impl User {
    pub fn new() -> User {
        User {
            transcriber: None,
            speaking: false,
        }
    }
}

pub mod text;
pub mod voice;