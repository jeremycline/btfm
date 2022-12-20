// SPDX-License-Identifier: GPL-2.0-or-later
use std::fs;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use serenity::{client::Client, framework::StandardFramework, prelude::*};
use songbird::{driver::DecodeMode, SerenityInit, Songbird};
use sqlx::{postgres::PgPoolOptions, Pool, Postgres};
use tracing::{debug, error, info};

use btfm::config::{load_config, Config};
use btfm::discord::{
    text::{Handler, HttpClient},
    BtfmData,
};
use btfm::{db, Backend, Error};

static MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/");

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
        backend: Backend,
    },
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    tracing_subscriber::fmt::init();

    let opts = Btfm::parse();
    btfm::CONFIG
        .set(opts.config.clone())
        .expect("Failed to set config global");

    debug!("Starting database connection to perform the database migration");
    let db_pool = match PgPoolOptions::new()
        .max_connections(32)
        .connect(&opts.config.database_url)
        .await
    {
        Ok(pool) => match MIGRATIONS.run(&pool).await {
            Ok(_) => pool,
            Err(e) => {
                error!("Failed to migrate the database: {:?}", e);
                return;
            }
        },
        Err(e) => {
            error!("Unable to connect to the database: {}", e);
            return;
        }
    };

    match process_command(opts, db_pool).await {
        Ok(_) => {}
        Err(e) => eprintln!("Error: {}", e),
    }
}

async fn process_command(opts: Btfm, db_pool: Pool<Postgres>) -> Result<(), Error> {
    match opts.command {
        Command::Run { backend } => {
            let framework = StandardFramework::new();
            // Configure Songbird to decode audio to signed 16 bit-per-same stereo PCM.
            let songbird = Songbird::serenity();
            songbird.set_config(songbird::Config::default().decode_mode(DecodeMode::Decode));

            let intents = GatewayIntents::non_privileged();
            let mut client = Client::builder(&opts.config.discord_token, intents)
                .event_handler(Handler)
                .framework(framework)
                .register_songbird_with(songbird)
                .await?;
            {
                let mut data = client.data.write().await;

                data.insert::<HttpClient>(Arc::clone(&client.cache_and_http));
                data.insert::<BtfmData>(Arc::new(Mutex::new(BtfmData::new(backend).await)));
            }
            let discord_client_handle = tokio::spawn(async move { client.start().await });

            let http_api = opts.config.http_api.clone();
            let router = btfm::web::create_router(&http_api, db_pool);
            let server_handle = match (http_api.tls_certificate, http_api.tls_key) {
                (None, None) => {
                    info!("Starting HTTP server on {:?}", &http_api.url);
                    tokio::spawn(async move {
                        axum_server::bind(http_api.url)
                            .serve(router.into_make_service())
                            .await
                            .map_err(Error::Server)
                    })
                }
                (Some(cert), Some(key)) => tokio::spawn(async move {
                    info!("Starting HTTPS server on {:?}", &http_api.url);
                    let tls_config =
                        axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?;
                    axum_server::bind_rustls(http_api.url, tls_config)
                        .serve(router.into_make_service())
                        .await
                        .map_err(Error::Server)
                }),
                _ => return Err(Error::ConfigValueError(
                    "'tls_certificate' and 'tls_key' must both be set or neither should be set."
                        .into(),
                )),
            };

            let (discord, http_server) = tokio::join!(discord_client_handle, server_handle);
            discord??;
            http_server??;

            Ok(())
        }
        Command::Tidy { clean } => {
            let mut conn = db_pool.acquire().await.unwrap();
            let clips = db::clips_list(&mut conn).await?;
            println!("Clips without audio files:");
            for clip in clips.iter() {
                let file = opts.config.data_directory.join(&clip.audio_file);
                if !file.exists() {
                    println!("{}", clip);
                    if clean {
                        db::remove_clip(&mut conn, clip.uuid).await?;
                    }
                }
            }

            let clip_dir = opts.config.data_directory.join("clips");
            let files = fs::read_dir(&clip_dir)?;
            println!("Audio files without clips:");
            let clip_names: Vec<String> =
                clips.iter().map(|clip| clip.audio_file.clone()).collect();
            for file in files.flatten() {
                let file_namish = "clips/".to_owned() + file.file_name().to_str().unwrap();
                if !clip_names.iter().any(|p| p == &file_namish) {
                    let file_path = file.path();
                    if let Some(p) = file_path.to_str() {
                        println!("{}", p);
                        if clean {
                            if let Err(e) = tokio::fs::remove_file(file.path()).await {
                                println!("Failed to remove file: {}", e)
                            }
                        }
                    }
                }
            }

            Ok(())
        }
    }
}
