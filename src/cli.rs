// SPDX-License-Identifier: GPL-2.0-or-later

use std::path::PathBuf;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "btfm", about = "Start the btfm service, add audio clips, etc.")]
pub struct Btfm {
    /// Path to the BTFM data directory where clips and the database is stored
    #[structopt(long, parse(from_os_str), env = "BTFM_DATA_DIR")]
    pub btfm_data_dir: PathBuf,
    #[structopt(subcommand)]
    pub command: Command,
}

#[derive(StructOpt, Debug)]
pub enum Command {
    /// Run the bot service
    Run {
        /// Log verbosity (-v for warn, -vv for info, etc)
        #[structopt(short, long, parse(from_occurrences))]
        verbose: usize,
        /// The Discord API token.
        #[structopt(long, env = "DISCORD_TOKEN", hide_env_values = true)]
        discord_token: String,
        /// Path to the DeepSpeech model directory.
        #[structopt(long, parse(from_os_str), env = "DEEPSPEECH_MODEL")]
        deepspeech_model: PathBuf,
        /// Path to the optional DeepSpeech scorer (increases accuracy)
        #[structopt(long, parse(from_os_str), env = "DEEPSPEECH_SCORER")]
        deepspeech_scorer: Option<PathBuf>,
        /// Discord Channel ID to join.
        #[structopt(long, env = "CHANNEL_ID")]
        channel_id: u64,
        /// Discord Channel ID to log events to
        #[structopt(long, env = "LOG_CHANNEL_ID")]
        log_channel_id: Option<u64>,
        /// Discord Guild ID to join.
        #[structopt(long, env = "GUILD_ID")]
        guild_id: u64,
        /// How much to rate limit the bot. The odds of playing are 1 - e^-(x/rate_adjuster).
        #[structopt(short, long, default_value = "256", env = "RATE_ADJUSTER")]
        rate_adjuster: f64,
    },
    /// Manage audio clips for the bot
    Clip(Clip),
}

#[derive(StructOpt, Debug)]
pub enum Clip {
    /// Add a new clip to the database
    Add {
        /// The phrase that triggers the audio clip
        #[structopt()]
        phrase: String,
        /// A short description of the audio clip
        #[structopt()]
        description: String,
        /// The filename of the clip.
        #[structopt(parse(from_os_str))]
        file: PathBuf,
    },
    /// Edit an existing clip in the database
    Edit {
        /// The clip ID (from "clip list")
        #[structopt()]
        clip_id: i32,
        /// The phrase that triggers the audio clip
        #[structopt(short, long)]
        phrase: Option<String>,
        /// A short description of the audio clip
        #[structopt(short, long)]
        description: Option<String>,
    },
    /// List clips in the database
    List {},
    /// Remove clips from the database
    Remove {
        /// The clip ID (from "clip list")
        #[structopt()]
        clip_id: i32,
    },
}
