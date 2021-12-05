// SPDX-License-Identifier: GPL-2.0-or-later
use std::fs;
use std::sync::Arc;

use serenity::{client::Client, framework::StandardFramework, prelude::*};
use songbird::{driver::DecodeMode, SerenityInit, Songbird};
use sqlx::postgres::PgPoolOptions;
use structopt::StructOpt;
use tracing::{debug, error};

use btfm::discord::{
    text::{Handler, HttpClient},
    BtfmData,
};
use btfm::{cli, db, transcode, transcribe, Backend};

static MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/");

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    tracing_subscriber::fmt::init();

    let opts = cli::Btfm::from_args();
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

    match opts.command {
        cli::Command::Run { backend } => {
            let framework = StandardFramework::new();
            // Configure Songbird to decode audio to signed 16 bit-per-same stereo PCM.
            let songbird = Songbird::serenity();
            songbird.set_config(songbird::Config::default().decode_mode(DecodeMode::Decode));

            let mut client = Client::builder(&opts.config.discord_token)
                .event_handler(Handler)
                .framework(framework)
                .register_songbird_with(songbird)
                .await
                .expect("Failed to create client");
            {
                let mut data = client.data.write().await;

                data.insert::<HttpClient>(Arc::clone(&client.cache_and_http));
                data.insert::<BtfmData>(Arc::new(Mutex::new(BtfmData::new(backend).await)));
            }
            let discord_client_handle = tokio::spawn(async move {
                client
                    .start()
                    .await
                    .map_err(|e| error!("Discord client died: {:?}", e))
            });

            if let Some(socket_addr) = &opts.config.api_url {
                let app = btfm::web::create(db_pool);
                let server = axum::Server::bind(socket_addr).serve(app.into_make_service());
                let server_handle = tokio::spawn(server);
                let (_first, _second) = tokio::join!(discord_client_handle, server_handle);
            } else {
                discord_client_handle.await.unwrap().unwrap();
            }
        }

        cli::Command::Clip(clip_subcommand) => match clip_subcommand {
            cli::Clip::Add { description, file } => {
                fs::create_dir_all(opts.config.data_directory.join("clips"))
                    .expect("Unable to create clips directory");

                let transcriber = transcribe::Transcriber::new(&opts.config, &Backend::default());
                let audio = transcode::file_to_wav(&file, 16_000).await;
                let (audio_sender, audio_receiver) = tokio::sync::mpsc::channel(2048);
                let mut phrase_receiver = transcriber.stream(audio_receiver).await;
                audio_sender.send(audio).await.unwrap();
                drop(audio_sender);
                let phrase = phrase_receiver.blocking_recv().unwrap();

                db::Clip::insert(
                    &db_pool,
                    &opts.config.data_directory,
                    &file,
                    &description,
                    &phrase,
                )
                .await
                .unwrap();
            }

            cli::Clip::Edit {
                clip_id,
                description,
            } => {
                let mut clip = db::Clip::get(&db_pool, clip_id).await.unwrap();
                if let Some(desc) = description {
                    clip.description = desc;
                }
                clip.update(&db_pool).await.unwrap();
            }

            cli::Clip::List {} => {
                for clip in db::Clip::list(&db_pool).await.unwrap() {
                    println!("{}", clip);
                }
            }

            cli::Clip::Remove { clip_id } => {
                let clip = db::Clip::get(&db_pool, clip_id).await.unwrap();
                clip.remove(&db_pool, &opts.config.data_directory)
                    .await
                    .unwrap();
            }
        },

        cli::Command::Phrase(phrase_subcommand) => match phrase_subcommand {
            cli::Phrase::Add { phrase } => {
                db::Phrase::insert(&db_pool, &phrase)
                    .await
                    .expect("Failed to add phrase");
            }

            cli::Phrase::Edit { phrase_id, phrase } => {
                let db_phrase = db::Phrase::get(&db_pool, phrase_id)
                    .await
                    .expect("Unable to get phrase with that ID");
                db_phrase
                    .update(&db_pool, &phrase)
                    .await
                    .expect("Couldn't set the new phrase for the clip");
            }

            cli::Phrase::List {} => {
                for phrase in db::Phrase::list(&db_pool).await.unwrap() {
                    println!("{}", phrase);
                }
            }

            cli::Phrase::Remove { phrase_id } => {
                let db_phrase = db::Phrase::get(&db_pool, phrase_id)
                    .await
                    .expect("Unable to get phrase with that ID");
                db_phrase
                    .remove(&db_pool)
                    .await
                    .expect("Failed to remove phrase");
            }
            cli::Phrase::Trigger { clip_id, phrase_id } => {
                db::ClipPhrase::insert(&db_pool, clip_id, phrase_id)
                    .await
                    .unwrap();
            }
            cli::Phrase::Untrigger { clip_id, phrase_id } => {
                db::ClipPhrase::remove(&db_pool, clip_id, phrase_id)
                    .await
                    .unwrap();
            }
        },
        cli::Command::Trigger { clip_id, phrase } => {
            match db::Phrase::insert(&db_pool, &phrase).await {
                Ok(phrase) => {
                    db::ClipPhrase::insert(&db_pool, clip_id, phrase.uuid)
                        .await
                        .unwrap();
                }
                Err(e) => {
                    println!("Unable to add phrase: {:?}", e);
                }
            }
        }
        cli::Command::Tidy { clean } => {
            let clips = db::Clip::list(&db_pool)
                .await
                .expect("Failed to query the database for clips");
            println!("Clips without audio files:");
            for clip in clips.iter() {
                let file = opts.config.data_directory.join(&clip.audio_file);
                if !file.exists() {
                    println!("{}", clip);
                    if clean {
                        clip.remove(&db_pool, &opts.config.data_directory)
                            .await
                            .unwrap();
                    }
                }
            }

            let clip_dir = opts.config.data_directory.join("clips");
            match fs::read_dir(&clip_dir) {
                Ok(files) => {
                    println!("Audio files without clips:");
                    let clip_names: Vec<String> =
                        clips.iter().map(|clip| clip.audio_file.clone()).collect();
                    for file in files.flatten() {
                        let file_namish = "clips/".to_owned() + file.file_name().to_str().unwrap();
                        if !clip_names.iter().any(|p| p == &file_namish) {
                            let file_path = file.path();
                            println!("{}", &file_path.to_str().unwrap());
                            if clean {
                                if let Err(e) = tokio::fs::remove_file(file.path()).await {
                                    println!("Failed to remove file: {}", e)
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("Unable to read clips: {}", e)
                }
            }
        }
    }
}
