// SPDX-License-Identifier: GPL-2.0-or-later

use std::path::PathBuf;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "btfm", about = "Start the btfm service, add audio clips, etc.")]
pub struct Btfm {
    /// Path to the BTFM data directory where clips are stored
    #[structopt(long, parse(from_os_str), env = "BTFM_DATA_DIR")]
    pub btfm_data_dir: PathBuf,
    /// PostgreSQL database URL in the format postgres://<user>:<password>@hostname/dbname
    #[structopt(env = "DATABASE_URL")]
    pub db_url: String,
    #[structopt(subcommand)]
    pub command: Command,
}

#[derive(StructOpt, Debug)]
pub enum Command {
    /// Compare the clips in the database with audio clips on disk; this is useful if something
    /// goes terribly wrong with this program or your filesystem. It will list clips with files
    /// that don't exist, as well as files that don't belong to any clip.
    Tidy {
        /// Remove the dangling files and remove the clips without files from the database
        #[structopt(long)]
        clean: bool,
    },
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

    /// Set a clip to trigger on a given phrase; this will create a new phrase.
    /// To manage the association between existing phrases and a clip, use the phrase sub-command.
    Trigger {
        /// The clip ID (from "clip list")
        #[structopt()]
        clip_id: i64,
        /// The phrase that triggers the audio clip
        #[structopt()]
        phrase: String,
    },

    /// Manage audio clips for the bot
    Clip(Clip),
    /// Manage phrases that trigger audio clips
    Phrase(Phrase),
}

#[derive(StructOpt, Debug)]
pub enum Clip {
    /// Add a new clip to the database
    Add {
        /// A short description of the audio clip
        #[structopt()]
        description: String,
        /// The filename of the clip.
        #[structopt(parse(from_os_str))]
        file: PathBuf,
        /// Path to the DeepSpeech model directory.
        #[structopt(long, parse(from_os_str), env = "DEEPSPEECH_MODEL")]
        deepspeech_model: PathBuf,
        /// Path to the optional DeepSpeech scorer (increases accuracy)
        #[structopt(long, parse(from_os_str), env = "DEEPSPEECH_SCORER")]
        deepspeech_scorer: PathBuf,
    },
    /// Edit an existing clip in the database
    Edit {
        /// The clip ID (from "clip list")
        #[structopt()]
        clip_id: i64,
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
        clip_id: i64,
    },
}

#[derive(StructOpt, Debug)]
pub enum Phrase {
    /// Trigger a clip with an existing phrase
    Trigger {
        /// The clip ID (from "clip list")
        #[structopt()]
        clip_id: i64,
        /// The phrase ID (from "phrase list")
        #[structopt()]
        phrase_id: i64,
    },
    /// Remove a phrase as a trigger for a clip
    Untrigger {
        /// The clip ID (from "clip list")
        #[structopt()]
        clip_id: i64,
        /// The phrase ID (from "phrase list")
        #[structopt()]
        phrase_id: i64,
    },
    /// Add a new phrase to the database
    Add {
        /// A phrase that can be associated with clips to trigger them
        #[structopt()]
        phrase: String,
    },
    /// Edit an existing phrase in the database
    Edit {
        /// The phrase ID (from "phrase list")
        #[structopt()]
        phrase_id: i64,
        /// The new phrase
        #[structopt(short, long)]
        phrase: String,
    },
    /// List phrases in the database
    List {},
    /// Remove phrases from the database
    Remove {
        /// The phrase ID (from "phrase list")
        #[structopt()]
        phrase_id: i64,
    },
}
