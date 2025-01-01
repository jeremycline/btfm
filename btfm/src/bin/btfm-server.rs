// SPDX-License-Identifier: GPL-2.0-or-later
use std::sync::Arc;
use std::{fs, str::FromStr};

use anyhow::Context;
use clap::Parser;
use serenity::{client::Client, prelude::*};
use songbird::{driver::DecodeMode, SerenityInit, Songbird};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    ConnectOptions, Pool, Sqlite,
};
use tracing::{info, instrument, Instrument};

use btfm::discord::{text::Handler, BtfmData};
use btfm::{cli, db, Error};

static MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/");

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let opts = cli::Btfm::parse();
    btfm::CONFIG
        .set(opts.config.clone())
        .expect("Failed to set config global");

    if !opts.config.data_directory.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .create(&opts.config.data_directory)
            .context("Data directory doesn't exist and couldn't be created.")?;
    }

    let db_pool = migrate(&opts).await?;
    match process_command(opts, db_pool).await {
        Ok(_) => {}
        Err(e) => eprintln!("Error: {e}"),
    };

    Ok(())
}

#[instrument(skip_all)]
async fn migrate(opts: &cli::Btfm) -> anyhow::Result<Pool<Sqlite>> {
    // Create the database if it doesn't exist; there's no option to create
    // if missing on the PoolOptions.
    let _ = SqliteConnectOptions::from_str(&opts.config.database_url())?
        .create_if_missing(true)
        .connect()
        .await?;
    let db_pool = SqlitePoolOptions::new()
        .connect(&opts.config.database_url())
        .await?;
    MIGRATIONS
        .run(&db_pool)
        .await
        .context("Failed to migrate database to latest versions!")?;

    Ok(db_pool)
}

async fn process_command(opts: cli::Btfm, db_pool: Pool<Sqlite>) -> Result<(), Error> {
    match opts.command {
        cli::Command::Discord {} => {
            gstreamer::init()?;

            // Configure Songbird to decode audio to signed 16 bit-per-same stereo PCM.
            let songbird = Songbird::serenity();
            songbird.set_config(songbird::Config::default().decode_mode(DecodeMode::Decode));

            let intents = GatewayIntents::non_privileged();
            let mut client = Client::builder(&opts.config.discord_token, intents)
                .event_handler(Handler)
                .register_songbird_with(songbird)
                .await?;

            let http_handle = axum_server::Handle::new();
            let transcriber = {
                let mut data = client.data.write().await;
                let btfm_data = BtfmData::new().await;
                let transcriber = btfm_data.transcriber.clone();
                let web_transcriber = btfm_data.transcriber.clone();
                let handle = http_handle.clone();
                data.insert::<BtfmData>(Arc::new(Mutex::new(btfm_data)));

                tokio::spawn(async move {
                    let _shutdown_signal = tokio::signal::ctrl_c().await;
                    tracing::info!("Shutdown signal received; beginning graceful shutdown.");
                    transcriber.shutdown().await;
                    handle.graceful_shutdown(Some(std::time::Duration::from_secs(15)));

                    // TODO figure out why the transcriber still blocks shutdown.
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    std::process::exit(1);
                });
                web_transcriber
            };
            let discord_span = tracing::info_span!("discord");
            let discord_client_handle =
                tokio::spawn(async move { client.start().await }.instrument(discord_span));

            let http_api = opts.config.http_api.clone();
            let router = btfm::web::create_router(&http_api, db_pool, transcriber);
            let http_span = tracing::info_span!("http_server");
            let server_handle = match (http_api.tls_certificate, http_api.tls_key) {
                (None, None) => {
                    info!("Starting HTTP server on {:?}", &http_api.url);
                    tokio::spawn(
                        async move {
                            axum_server::bind(http_api.url)
                                .handle(http_handle)
                                .serve(router.into_make_service())
                                .await
                                .map_err(Error::Server)
                        }
                        .instrument(http_span),
                    )
                }
                (Some(cert), Some(key)) => tokio::spawn(
                    async move {
                        info!("Starting HTTPS server on {:?}", &http_api.url);
                        let tls_config =
                            axum_server::tls_openssl::OpenSSLConfig::from_pem_file(cert, key)
                                .unwrap();
                        axum_server::bind_openssl(http_api.url, tls_config)
                            .handle(http_handle)
                            .serve(router.into_make_service())
                            .await
                            .map_err(Error::Server)
                    }
                    .instrument(http_span),
                ),
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
        cli::Command::Tidy { clean } => {
            let mut conn = db_pool.acquire().await.unwrap();
            let clips = db::clips_list(&mut conn).await?;
            println!("Clips without audio files:");
            for clip in clips.iter() {
                let file = opts.config.data_directory.join(&clip.audio_file);
                if !file.exists() {
                    println!("{clip}");
                    if clean {
                        db::remove_clip(&mut conn, clip.uuid.clone()).await?;
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
                        println!("{p}");
                        if clean {
                            if let Err(e) = tokio::fs::remove_file(file.path()).await {
                                println!("Failed to remove file: {e}")
                            }
                        }
                    }
                }
            }

            Ok(())
        }
        cli::Command::Web {} => {
            gstreamer::init()?;

            let http_handle = axum_server::Handle::new();
            let transcriber = {
                let handle = http_handle.clone();
                let btfm_data = BtfmData::new().await;
                let transcriber = btfm_data.transcriber.clone();
                let web_transcriber = btfm_data.transcriber;

                tokio::spawn(async move {
                    let _shutdown_signal = tokio::signal::ctrl_c().await;
                    tracing::info!("Shutdown signal received; beginning graceful shutdown.");
                    transcriber.shutdown().await;
                    handle.graceful_shutdown(Some(std::time::Duration::from_secs(15)));
                    // TODO figure out why the transcriber still blocks shutdown.
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    std::process::exit(1);
                });
                web_transcriber
            };
            let http_api = opts.config.http_api.clone();
            let router = btfm::web::create_router(&http_api, db_pool, transcriber);
            match (http_api.tls_certificate, http_api.tls_key) {
                (None, None) => {
                    info!("Starting HTTP server on {:?}", &http_api.url);
                    tokio::spawn(async move {
                        axum_server::bind(http_api.url)
                            .handle(http_handle)
                            .serve(router.into_make_service())
                            .await
                            .map_err(Error::Server)
                    })
                }
                (Some(cert), Some(key)) => tokio::spawn(async move {
                    info!("Starting HTTPS server on {:?}", &http_api.url);
                    let tls_config =
                        axum_server::tls_openssl::OpenSSLConfig::from_pem_file(cert, key).unwrap();
                    axum_server::bind_openssl(http_api.url, tls_config)
                        .handle(http_handle)
                        .serve(router.into_make_service())
                        .await
                        .map_err(Error::Server)
                }),
                _ => return Err(Error::ConfigValueError(
                    "'tls_certificate' and 'tls_key' must both be set or neither should be set."
                        .into(),
                )),
            }
            .await??;

            Ok(())
        }
    }
}
