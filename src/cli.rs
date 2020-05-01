// SPDX-License-Identifier: GPL-2.0-or-later

use std::path::PathBuf;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "btfm", about = "Start the btfm service, add audio clips, etc.")]
pub enum Btfm {
    /// Run the bot service
    Run {
        /// Log verbosity (-v for warn, -vv for info, etc)
        #[structopt(short, long, parse(from_occurrences))]
        verbose: usize,
        /// The Discord API token.
        #[structopt(long, env = "DISCORD_TOKEN")]
        discord_token: String,
        /// Path to the DeepSpeech model directory.
        #[structopt(long, parse(from_os_str), env = "DEEPSPEECH_MODEL")]
        deepspeech_model: PathBuf,
        /// Path to the optional DeepSpeech scorer (increases accuracy)
        #[structopt(long, parse(from_os_str), env = "DEEPSPEECH_SCORER")]
        deepspeech_scorer: Option<PathBuf>,
        /// Path to the BTFM data directory where clips and the database is stored
        #[structopt(long, parse(from_os_str), env = "BTFM_DATA_DIR")]
        btfm_data_dir: PathBuf,
        /// Discord Channel ID to join.
        #[structopt(long, env = "CHANNEL_ID")]
        channel_id: u64,
        /// Discord Guild ID to join.
        #[structopt(long, env = "GUILD_ID")]
        guild_id: u64,
    },
    /// Manage audio clips for the bot
    Clip(Clip),
}

#[derive(StructOpt, Debug)]
pub enum Clip {
    /// Add a new clip to the database
    Add {
        /// Path to the BTFM data directory where clips and the database is stored
        #[structopt(long, parse(from_os_str), env = "BTFM_DATA_DIR")]
        btfm_data_dir: PathBuf,
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
    /// List clips in the database
    List {
        /// Path to the BTFM data directory where clips and the database is stored
        #[structopt(long, parse(from_os_str), env = "BTFM_DATA_DIR")]
        btfm_data_dir: PathBuf,
    },
    /// Remove clips from the database
    Remove {
        /// Path to the BTFM data directory where clips and the database is stored
        #[structopt(long, parse(from_os_str), env = "BTFM_DATA_DIR")]
        btfm_data_dir: PathBuf,
        /// The clip ID (from "clip list")
        #[structopt()]
        clip_id: i32,
    },
}
