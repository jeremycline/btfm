// SPDX-License-Identifier: GPL-2.0-or-later
use std::fs;
use std::sync::Arc;

use clap::Parser;
use serenity::{client::Client, framework::StandardFramework, prelude::*};
use songbird::{driver::DecodeMode, SerenityInit, Songbird};
use sqlx::{postgres::PgPoolOptions, Pool, Postgres};
use tracing::{debug, error, info};

use btfm::discord::{
    text::{Handler, HttpClient},
    BtfmData,
};
use btfm::{cli, db, Error};

static MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/");

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    tracing_subscriber::fmt::init();

    let opts = cli::Btfm::parse();
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
        Err(e) => eprintln!("Error: {e}"),
    }
}

async fn process_command(opts: cli::Btfm, db_pool: Pool<Postgres>) -> Result<(), Error> {
    match opts.command {
        cli::Command::Run { backend } => {
            gstreamer::init()?;

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

            let http_handle = axum_server::Handle::new();
            {
                let mut data = client.data.write().await;
                let btfm_data = BtfmData::new(backend).await;
                let transcriber = btfm_data.transcriber.clone();
                let handle = http_handle.clone();
                data.insert::<HttpClient>(Arc::clone(&client.cache_and_http));
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
            }
            let discord_client_handle = tokio::spawn(async move { client.start().await });

            let http_api = opts.config.http_api.clone();
            let router = btfm::web::create_router(&http_api, db_pool);
            let server_handle = match (http_api.tls_certificate, http_api.tls_key) {
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
                        axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?;
                    axum_server::bind_rustls(http_api.url, tls_config)
                        .handle(http_handle)
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
        cli::Command::Tidy { clean } => {
            let mut conn = db_pool.acquire().await.unwrap();
            let clips = db::clips_list(&mut conn).await?;
            println!("Clips without audio files:");
            for clip in clips.iter() {
                let file = opts.config.data_directory.join(&clip.audio_file);
                if !file.exists() {
                    println!("{clip}");
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
    }
}
