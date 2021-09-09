/// Defines the configuration file format for BTFM.
use std::{fmt::Display, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    /// The data directory where clips and other application data is stored
    pub data_directory: PathBuf,
    /// The URL to the PostgreSQL database in the format "postgres://<user>:<pass>@host/database_name"
    pub database_url: String,
    /// The Discord API token.
    pub discord_token: String,
    /// Discord Channel ID to join.
    pub channel_id: u64,
    /// Discord Channel ID to log events to
    pub log_channel_id: Option<u64>,
    /// Discord Guild ID to join.
    pub guild_id: u64,
    /// How much to rate limit the bot. The odds of playing are 1 - e^-(x/rate_adjuster).
    pub rate_adjuster: f64,
    #[cfg(feature = "deepspeech-recognition")]
    /// DeepSpeech-specific configuration options
    pub deepspeech: DeepSpeech,
    #[cfg(feature = "deepgram-recognition")]
    /// Deepgram-specific configuration options
    pub deepgram: Deepgram,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DeepSpeech {
    /// Path to the DeepSpeech model (pbmm) file
    pub model: PathBuf,
    /// Path to the DeepSpeech scorer file
    pub scorer: Option<PathBuf>,
    /// Whether or not to use CUDA for DeepSpeech
    pub gpu: bool,
}

impl Default for DeepSpeech {
    fn default() -> Self {
        DeepSpeech {
            model: PathBuf::from(r"/var/lib/btfm/deepspeech.pbmm"),
            scorer: Some(PathBuf::from(r"/var/lib/btfm/deepspeech.scorer")),
            gpu: false,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Deepgram {
    /// The Deepgram API key to authenticate with
    pub api_key: String,
    /// The Deepgram streaming API endpoint; for example "wss://api.deepgram.com/v1/listen"
    pub api_endpoint: String,
}

impl Default for Deepgram {
    fn default() -> Self {
        Deepgram {
            api_key: "your-api-key".to_string(),
            api_endpoint: "api.deepgram.com".to_string(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            data_directory: PathBuf::from(r"/var/lib/btfm/"),
            database_url: "postgres:///btfm".to_string(),
            discord_token: "Go get a Discord API token".to_string(),
            channel_id: 0,
            log_channel_id: None,
            guild_id: 0,
            rate_adjuster: 120.0,
            #[cfg(feature = "deepspeech-recognition")]
            deepspeech: DeepSpeech::default(),
            #[cfg(feature = "deepgram-recognition")]
            deepgram: Deepgram::default(),
        }
    }
}

impl Display for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            toml::ser::to_string_pretty(&self).unwrap_or_default()
        )
    }
}

/// Load a [`Config`] instance from the given path.
pub fn load_config(path: &str) -> Result<Config, Error> {
    let path = PathBuf::from(path);
    let config_string = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&config_string).map_err(|err| {
        println!("Example config format:\n\n{}", Config::default());
        err
    })?;
    Ok(config)
}
