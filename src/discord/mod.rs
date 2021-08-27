//! Implements the Serenity event handlers for voice and text channels.
use crate::config::Config;
use crate::transcribe::{Transcribe, Transcriber};
use serenity::prelude::*;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::sync::Arc;

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
    pub async fn new(config: Config) -> BtfmData {
        let db = PgPoolOptions::new()
            .max_connections(10)
            .connect(&config.database_url)
            .await
            .expect("Unable to connect to database");
        let transcriber = Transcriber::new(&config);
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
    audio_buffer: Mutex<Vec<i16>>,
    speaking: bool,
}

impl User {
    pub fn new() -> User {
        User {
            audio_buffer: Mutex::new(Vec::new()),
            speaking: false,
        }
    }

    /// Add new audio to the user's buffer
    pub async fn push(&mut self, audio: &[i16]) {
        let mut buf = self.audio_buffer.lock().await;
        buf.extend(audio);
    }

    /// Empty and return the user buffer
    pub async fn reset(&mut self) -> Vec<i16> {
        let mut voice_data = self.audio_buffer.lock().await;
        let mut old_voice_data = Vec::new();
        old_voice_data.append(&mut voice_data);
        old_voice_data
    }
}

pub mod text;
pub mod voice;
