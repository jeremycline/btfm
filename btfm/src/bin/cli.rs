use std::{path::PathBuf, time::Duration};

use btfm::web::api::{Clip, ClipUpdated, ClipUpload, Clips, CreatePhrase, Phrase, Phrases};
use chrono::SubsecRound;
use clap::{Parser, Subcommand};
use reqwest::{multipart, Body, Url};
use thiserror::Error as ThisError;
use tokio::fs::File;
use tokio_util::codec::{BytesCodec, FramedRead};
use ulid::Ulid;

const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

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
#[derive(clap::Parser, Debug)]
#[command(name = "btfm-cli")]
#[command(author = "Jeremy Cline <jeremy@jcline.org>")]
#[command(about = "Manage clips in the BTFM Discord bot", long_about = None)]
struct Cli {
    #[arg(long, env = "BTFM_URL")]
    url: Url,
    #[arg(long, short, env = "BTFM_USER")]
    user: String,
    #[arg(long, short, env = "BTFM_PASSWORD")]
    password: String,
    #[clap(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage audio clips for the bot
    #[command(subcommand)]
    Clip(ClipCommand),
    /// Manage phrases that trigger audio clips
    #[command(subcommand)]
    Phrase(PhraseCommand),
}

#[derive(Subcommand, Debug)]
pub enum ClipCommand {
    /// Add a new clip to the database
    Add {
        /// A phrase to trigger the new clip
        #[arg(short, long)]
        phrases: Option<Vec<String>>,
        /// A short description of the audio clip
        #[arg()]
        description: String,
        /// The filename of the clip.
        #[arg()]
        file: PathBuf,
    },
    /// Edit an existing clip in the database.
    ///
    /// Phrases are replaced rather than appended, so you must provide the complete list of phrases
    /// each time you edit the clip.
    Edit {
        /// The clip ID (from "clip list")
        #[arg()]
        clip_id: Ulid,
        /// A short description of the audio clip
        #[arg(short, long)]
        description: Option<String>,
        /// The phrase or phrases that cause the clip to be played.
        #[arg(short, long)]
        phrases: Option<Vec<String>>,
    },
    /// List clips in the database
    List {},
    /// Remove clips from the database
    Remove {
        /// The clip ID (from "clip list")
        #[clap()]
        clip_id: Ulid,
    },
}

#[derive(Subcommand, Debug)]
pub enum PhraseCommand {
    /// Add a trigger phrase to a clip
    Add {
        /// The clip ID (from "clip list")
        #[clap()]
        clip_id: Ulid,
        /// The phrase to associate with the clip
        #[clap()]
        phrase: String,
    },
    // /// Remove a phrase as a trigger for a clip
    // Remove {
    //     /// The clip ID (from "clip list")
    //     #[clap()]
    //     clip_id: Ulid,
    //     /// The phrase ID (from "phrase list")
    //     #[clap()]
    //     phrase_id: Ulid,
    // },
    // /// Edit an existing phrase in the database
    // Edit {
    //     /// The phrase ID (from "phrase list")
    //     #[clap()]
    //     phrase_id: Ulid,
    //     /// The new phrase
    //     #[clap(short, long)]
    //     phrase: String,
    // },
    /// List phrases in the database
    List {},
    // /// Remove phrases from the database
    // Remove {
    //     /// The phrase ID (from "phrase list")
    //     #[clap()]
    //     phrase_id: Ulid,
    // },
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let opts = Cli::parse();
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
            ClipCommand::Add {
                description,
                file,
                phrases,
            } => {
                let url = opts.url.join("/v1/clips/")?;
                let file_name = file
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap()
                    .to_owned();
                let clip = File::open(file).await?;
                let clip_len = clip.metadata().await?.len();
                let clip_stream = FramedRead::new(clip, BytesCodec::new());
                let clip_part =
                    multipart::Part::stream_with_length(Body::wrap_stream(clip_stream), clip_len)
                        .file_name(file_name);
                let clip_metadata = serde_json::to_string(&ClipUpload {
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
                let clip = response.json::<Clip>().await?;
                println!("{}", serde_json::to_string_pretty(&clip)?);
                Ok(())
            }
            ClipCommand::List {} => {
                let url = opts.url.join("/v1/clips/")?;
                let response = client
                    .get(url)
                    .basic_auth(opts.user, Some(opts.password))
                    .send()
                    .await
                    .map(|resp| resp.error_for_status())??;
                let clips = response.json::<Clips>().await?;
                display_clips(&clips);
                Ok(())
            }
            ClipCommand::Edit {
                clip_id,
                description,
                phrases,
            } => {
                let endpoint = format!("/v1/clips/{}", clip_id);
                let url = opts.url.join(&endpoint)?;
                let json = ClipUpload {
                    description: description.unwrap_or_default(),
                    phrases,
                };
                let response = client
                    .put(url)
                    .basic_auth(opts.user, Some(opts.password))
                    .json(&json)
                    .send()
                    .await
                    .map(|resp| resp.error_for_status())??;
                let response = response.json::<ClipUpdated>().await?;
                println!("{}", serde_json::to_string_pretty(&response)?);

                Ok(())
            }
            ClipCommand::Remove { clip_id } => {
                let endpoint = format!("/v1/clips/{}", clip_id);
                let url = opts.url.join(&endpoint)?;
                let response = client
                    .delete(url)
                    .basic_auth(opts.user, Some(opts.password))
                    .send()
                    .await
                    .map(|resp| resp.error_for_status())??;
                let response = response.json::<Clip>().await?;
                println!("{}", serde_json::to_string_pretty(&response)?);

                Ok(())
            }
        },
        Command::Phrase(subcommand) => match subcommand {
            PhraseCommand::List {} => {
                let url = opts.url.join("/v1/phrases/")?;
                let response = client
                    .get(url)
                    .basic_auth(opts.user, Some(opts.password))
                    .send()
                    .await
                    .map(|resp| resp.error_for_status())??;
                let phrases = response.json::<Phrases>().await?;
                println!("{}", serde_json::to_string_pretty(&phrases)?);
                Ok(())
            }
            PhraseCommand::Add { clip_id, phrase } => {
                let url = opts.url.join("/v1/phrases/")?;
                let response = client
                    .post(url)
                    .basic_auth(opts.user, Some(opts.password))
                    .json(&CreatePhrase {
                        clip: clip_id,
                        phrase,
                    })
                    .send()
                    .await
                    .map(|resp| resp.error_for_status())??;
                let phrase = response.json::<Phrase>().await?;
                println!("{}", serde_json::to_string_pretty(&phrase)?);
                Ok(())
            }
        },
    }
}

fn display_clips(clips: &Clips) {
    let mut table = prettytable::Table::new();
    table.add_row(prettytable::Row::new(vec![
        prettytable::Cell::new("ID").with_style(prettytable::Attr::Bold),
        prettytable::Cell::new("Created").with_style(prettytable::Attr::Bold),
        prettytable::Cell::new("Last Played").with_style(prettytable::Attr::Bold),
        prettytable::Cell::new("Plays").with_style(prettytable::Attr::Bold),
        prettytable::Cell::new("Description").with_style(prettytable::Attr::Bold),
        prettytable::Cell::new("Phrases").with_style(prettytable::Attr::Bold),
    ]));
    for clip in clips.clips.iter() {
        table.add_row(prettytable::Row::new(vec![
            prettytable::Cell::new(clip.ulid.to_string().as_str()),
            prettytable::Cell::new(clip.created_on.trunc_subsecs(0).to_string().as_str()),
            prettytable::Cell::new(clip.last_played.trunc_subsecs(0).to_string().as_str()),
            prettytable::Cell::new(clip.plays.to_string().as_str()),
            prettytable::Cell::new(
                clip.description
                    .chars()
                    .take(64)
                    .collect::<String>()
                    .as_str(),
            ),
            prettytable::Cell::new(
                clip.phrases
                    .as_ref()
                    .map(|p| p.items)
                    .unwrap_or(0)
                    .to_string()
                    .as_str(),
            ),
        ]));
    }

    table.printstd();
}
