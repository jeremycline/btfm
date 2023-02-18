/// Defines the configuration file format for BTFM.
use std::{
    fmt::Display,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::Error;

#[derive(Serialize, Deserialize, Debug, Clone)]
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
    /// Whisper configuration options
    pub whisper: Whisper,
    /// Deepgram-specific configuration options
    pub deepgram: Deepgram,
    /// The HTTP server configution options
    pub http_api: HttpApi,
    /// The time between random clip plays, in seconds.
    pub random_clip_interval: u64,

    pub mimic_endpoint: Option<Url>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Whisper {
    /// Path to the whisper-server endpoint; for example "ws://localhost:8000/v1/listen"
    pub model: PathBuf,
}

impl Default for Whisper {
    fn default() -> Self {
        Whisper {
            model: PathBuf::from("/var/lib/btfm/whisper/base.en.pt"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Deepgram {
    /// The Deepgram API key to authenticate with
    pub api_key: String,
    /// The Deepgram streaming API endpoint; for example "wss://api.deepgram.com/v1/listen"
    pub websocket_url: Url,
}

impl Default for Deepgram {
    fn default() -> Self {
        Deepgram {
            api_key: "your-api-key".to_string(),
            websocket_url: Url::parse("wss://api.deepgram.com/v1/listen").unwrap(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HttpApi {
    /// The URL of an HTTP API used to manage the bot.
    pub url: SocketAddr,
    /// The username to use for API Basic Authentication; this is used by btfm-cli.
    pub user: String,
    /// The password to use for API Basic Authentication; this is used by btfm-cli.
    pub password: String,
    /// The path to an x509 certificate the server should use for HTTPS.
    pub tls_certificate: Option<PathBuf>,
    /// The path to the key for the given certificate.
    pub tls_key: Option<PathBuf>,
}

impl Default for HttpApi {
    fn default() -> Self {
        HttpApi {
            url: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080),
            user: "admin".to_string(),
            password: "admin".to_string(),
            tls_certificate: None,
            tls_key: None,
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
            deepgram: Default::default(),
            whisper: Default::default(),
            http_api: Default::default(),
            random_clip_interval: 60 * 15,
            mimic_endpoint: None,
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
