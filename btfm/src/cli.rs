// SPDX-License-Identifier: GPL-2.0-or-later
use clap::{Parser, Subcommand, ValueEnum};

use crate::config::{load_config, Config};

/// CLI to start the btfm service, manage audio clips, and more.
///
/// # Logging
///
/// When running the service, log levels and filtering are controlled by tracing_subscriber's
/// EnvFilter using the RUST_LOG environment variable. Refer to the documentation at
/// https://docs.rs/tracing-subscriber/0.3.1/tracing_subscriber/filter/struct.EnvFilter.html
/// for complete details.
///
/// The most basic form is one of "trace", "debug", "info", "warn", or "error". For example:
///
/// RUST_LOG=warn
///
/// # Configuration
///
/// The configuration file is expected to be in TOML format.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Btfm {
    /// Path to the BTFM configuration file; see btfm.toml.example for details
    #[arg(value_parser = load_config, env = "BTFM_CONFIG")]
    pub config: Config,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Compare the clips in the database with audio clips on disk; this is useful if something
    /// goes terribly wrong with this program or your filesystem. It will list clips with files
    /// that don't exist, as well as files that don't belong to any clip.
    Tidy {
        /// Remove the dangling files and remove the clips without files from the database
        #[arg(long)]
        clean: bool,
    },
    /// Run the bot service
    Run {
        #[arg(
            short,
            long,
            value_enum,
            ignore_case = true,
            default_value_t,
            env = "BTFM_BACKEND"
        )]
        backend: crate::Backend,
    },
}

#[derive(ValueEnum, Clone, Debug)]
pub enum Backend {
    Deepgram,
    Whisper,
}

impl Default for Backend {
    fn default() -> Self {
        Backend::Whisper
    }
}
