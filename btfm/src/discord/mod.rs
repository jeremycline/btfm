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
    pub status_report: Option<String>,
    pub http_client: reqwest::Client,
}
impl TypeMapKey for BtfmData {
    type Value = Arc<Mutex<BtfmData>>;
}
impl BtfmData {
    pub async fn new(backend: Backend) -> BtfmData {
        let config = crate::CONFIG
            .get()
            .expect("CONFIG needs to be initialized before starting the transcriber")
            .to_owned();
        let db = PgPoolOptions::new()
            .max_connections(10)
            .connect(&config.database_url)
            .await
            .expect("Unable to connect to database");

        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .expect("Unable to build a basic HTTP client");
        let transcriber = Transcriber::new(&config, &backend);
        BtfmData {
            config,
            transcriber,
            users: HashMap::new(),
            ssrc_map: HashMap::new(),
            user_history: HashMap::new(),
            db,
            status_report: None,
            http_client,
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
