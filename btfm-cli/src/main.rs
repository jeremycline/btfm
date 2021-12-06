use std::{path::PathBuf, time::Duration};

use reqwest::{multipart, Body, Url};
use structopt::StructOpt;
use thiserror::Error as ThisError;
use tokio::fs::File;
use tokio_util::codec::{BytesCodec, FramedRead};
use ulid::Ulid;

const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

// I need to factor these out and move the From impls around I think
mod gross_hack {
    /// Defines public-facing structures used in the web API
    use chrono::NaiveDateTime;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize)]
    pub struct Status {
        pub db_version: Option<u32>,
        pub db_connections: u32,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct Clip {
        /// The unique identifier for the clip and primary key for the table.
        pub ulid: ulid::Ulid,
        /// The time when the clip was added to the database.
        pub created_on: NaiveDateTime,
        /// The last time the clip was played; this is equal to `created_on` when created.
        pub last_played: NaiveDateTime,
        /// Number of times the clip has been played.
        pub plays: i64,
        /// The output of speech-to-text on the `audio_file`, optionally used as a matching phrase.
        pub speech_detected: String,
        /// A description of the clip for human consumption.
        pub description: String,
        /// Path to the audio file, relative to the BTFM_DATA_DIR.
        pub audio_file: String,
        /// Phrases associated with the clip.
        pub phrases: Option<Phrases>,
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct Clips {
        pub items: u64,
        pub clips: Vec<Clip>,
    }

    /// A phrase used to trigger one or more clips
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct Phrase {
        pub ulid: ulid::Ulid,
        pub phrase: String,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct CreatePhrase {
        /// The phrase.
        pub phrase: String,
        /// The clip to associate the phrase to.
        pub clip: ulid::Ulid,
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct Phrases {
        pub items: u64,
        pub phrases: Vec<Phrase>,
    }

    #[derive(Debug, Deserialize, Serialize)]
    pub struct ClipUpload {
        pub description: String,
        pub phrases: Option<Vec<String>>,
    }
}

#[derive(ThisError, Debug)]
enum Error {
    #[error("An HTTP error occurred: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("Unable to parse BTFM server URL: {0}")]
    Url(#[from] url::ParseError),
    #[error("Unable to read from the filesystem: {0}")]
    Io(#[from] std::io::Error),
    #[error("Unable to serialize request to valid JSON: {0}")]
    Json(#[from] serde_json::error::Error),
}

/// Command-line interface for the btfm service
///
/// This supports uploading, editing, and removing clips or trigger phrases.
#[derive(StructOpt, Debug)]
#[structopt(name = "btfm-cli")]
struct Cli {
    #[structopt(long, env = "BTFM_URL")]
    url: Url,
    #[structopt(long, short, env = "BTFM_USER")]
    user: String,
    #[structopt(long, short, env = "BTFM_PASSWORD")]
    password: String,
    #[structopt(subcommand)]
    command: Command,
}

#[derive(StructOpt, Debug)]
pub enum Command {
    /// Manage audio clips for the bot
    Clip(Clip),
    /// Manage phrases that trigger audio clips
    Phrase(Phrase),
}

#[derive(StructOpt, Debug)]
pub enum Clip {
    /// Add a new clip to the database
    Add {
        /// A phrase to trigger the new clip
        #[structopt(short, long)]
        phrases: Option<Vec<String>>,
        /// A short description of the audio clip
        #[structopt()]
        description: String,
        /// The filename of the clip.
        #[structopt(parse(from_os_str))]
        file: PathBuf,
    },
    // /// Edit an existing clip in the database
    // Edit {
    //     /// The clip ID (from "clip list")
    //     #[structopt()]
    //     clip_id: Ulid,
    //     /// A short description of the audio clip
    //     #[structopt(short, long)]
    //     description: Option<String>,
    // },
    /// List clips in the database
    List {},
    // /// Remove clips from the database
    // Remove {
    //     /// The clip ID (from "clip list")
    //     #[structopt()]
    //     clip_id: Ulid,
    // },
}

#[derive(StructOpt, Debug)]
pub enum Phrase {
    /// Add a trigger phrase to a clip
    Add {
        /// The clip ID (from "clip list")
        #[structopt()]
        clip_id: Ulid,
        /// The phrase to associate with the clip
        #[structopt()]
        phrase: String,
    },
    // /// Remove a phrase as a trigger for a clip
    // Remove {
    //     /// The clip ID (from "clip list")
    //     #[structopt()]
    //     clip_id: Ulid,
    //     /// The phrase ID (from "phrase list")
    //     #[structopt()]
    //     phrase_id: Ulid,
    // },
    // /// Edit an existing phrase in the database
    // Edit {
    //     /// The phrase ID (from "phrase list")
    //     #[structopt()]
    //     phrase_id: Ulid,
    //     /// The new phrase
    //     #[structopt(short, long)]
    //     phrase: String,
    // },
    /// List phrases in the database
    List {},
    // /// Remove phrases from the database
    // Remove {
    //     /// The phrase ID (from "phrase list")
    //     #[structopt()]
    //     phrase_id: Ulid,
    // },
}

#[tokio::main]
async fn main() {
    let opts = Cli::from_args();
    match process_command(opts).await {
        Ok(_) => {}
        Err(e) => eprintln!("Error: {}", e),
    };
}

async fn process_command(opts: Cli) -> Result<(), Error> {
    let client = reqwest::ClientBuilder::new()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Unable to create HTTP client");

    match opts.command {
        Command::Clip(subcommand) => match subcommand {
            Clip::Add {
                description,
                file,
                phrases,
            } => {
                let url = opts.url.join("/v1/clips/")?;
                let clip = File::open(file).await?;
                let clip_len = clip.metadata().await?.len();
                let clip_stream = FramedRead::new(clip, BytesCodec::new());
                let clip_part =
                    multipart::Part::stream_with_length(Body::wrap_stream(clip_stream), clip_len);
                let clip_metadata = serde_json::to_string(&gross_hack::ClipUpload {
                    description,
                    phrases,
                })?;
                let clip_metadata_part =
                    multipart::Part::text(clip_metadata).mime_str("application/json")?;
                let form = multipart::Form::new()
                    .part("clip_metadata", clip_metadata_part)
                    .part("clip", clip_part);
                let response = client
                    .post(url)
                    .basic_auth(opts.user, Some(opts.password))
                    .multipart(form)
                    .send()
                    .await
                    .map(|resp| resp.error_for_status())??;
                let clip = response.json::<gross_hack::Clip>().await?;
                println!("{}", serde_json::to_string_pretty(&clip)?);
                Ok(())
            }
            Clip::List {} => {
                let url = opts.url.join("/v1/clips/")?;
                let response = client
                    .get(url)
                    .basic_auth(opts.user, Some(opts.password))
                    .send()
                    .await
                    .map(|resp| resp.error_for_status())??;
                let clips = response.json::<gross_hack::Clips>().await?;
                println!("{}", serde_json::to_string_pretty(&clips)?);
                Ok(())
            }
        },
        Command::Phrase(subcommand) => match subcommand {
            Phrase::List {} => {
                let url = opts.url.join("/v1/phrases/")?;
                let response = client
                    .get(url)
                    .basic_auth(opts.user, Some(opts.password))
                    .send()
                    .await
                    .map(|resp| resp.error_for_status())??;
                let phrases = response.json::<gross_hack::Phrases>().await?;
                println!("{}", serde_json::to_string_pretty(&phrases)?);
                Ok(())
            }
            Phrase::Add { clip_id, phrase } => {
                let url = opts.url.join("/v1/phrases/")?;
                let response = client
                    .post(url)
                    .basic_auth(opts.user, Some(opts.password))
                    .json(&gross_hack::CreatePhrase {
                        clip: clip_id,
                        phrase,
                    })
                    .send()
                    .await
                    .map(|resp| resp.error_for_status())??;
                let phrase = response.json::<gross_hack::Phrase>().await?;
                println!("{}", serde_json::to_string_pretty(&phrase)?);
                Ok(())
            }
        },
    }
}