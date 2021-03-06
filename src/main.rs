// SPDX-License-Identifier: GPL-2.0-or-later
#[macro_use]
extern crate log;
extern crate stderrlog;

use std::fs;
use std::sync::Arc;

use serenity::{client::Client, framework::StandardFramework, prelude::*};
use songbird::{
    driver::{Config as DriverConfig, DecodeMode},
    SerenityInit, Songbird,
};
use structopt::StructOpt;

use btfm::voice::{BtfmData, Handler, HttpClient};
use btfm::{cli, db, DB_NAME};

static MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/");

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let opts = cli::Btfm::from_args();

    let db_pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect(opts.btfm_data_dir.join(DB_NAME).to_str().unwrap())
        .await
        .expect("Unable to connect to database");
    match MIGRATIONS.run(&db_pool).await {
        Ok(_) => {}
        Err(e) => {
            println!("Failed to migrate database: {}", e);
            return;
        }
    }

    match opts.command {
        cli::Command::Run {
            channel_id,
            log_channel_id,
            deepspeech_model,
            deepspeech_scorer,
            discord_token,
            guild_id,
            verbose,
            rate_adjuster,
        } => {
            stderrlog::new()
                .module(module_path!())
                .verbosity(verbose)
                .timestamp(stderrlog::Timestamp::Second)
                .init()
                .unwrap();

            let framework = StandardFramework::new();
            let songbird = Songbird::serenity();
            songbird.set_config(DriverConfig::default().decode_mode(DecodeMode::Decode));

            let mut client = Client::builder(&discord_token)
                .event_handler(Handler)
                .framework(framework)
                .register_songbird_with(songbird)
                .await
                .expect("Failed to create client");
            {
                let mut data = client.data.write().await;

                data.insert::<HttpClient>(Arc::clone(&client.cache_and_http));
                data.insert::<BtfmData>(Arc::new(Mutex::new(BtfmData::new(
                    opts.btfm_data_dir,
                    deepspeech_model,
                    deepspeech_scorer,
                    guild_id,
                    channel_id,
                    log_channel_id,
                    rate_adjuster,
                    db_pool,
                ))));
            }
            let _ = client
                .start()
                .await
                .map_err(|why| error!("Client ended: {:?}", why));
        }

        cli::Command::Clip(clip_subcommand) => match clip_subcommand {
            cli::Clip::Add {
                description,
                file,
                deepspeech_model,
                deepspeech_scorer,
            } => {
                fs::create_dir_all(opts.btfm_data_dir.join("clips"))
                    .expect("Unable to create clips directory");
                db::Clip::insert(
                    &db_pool,
                    &opts.btfm_data_dir,
                    &file,
                    &description,
                    &deepspeech_model,
                    &deepspeech_scorer,
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
                clip.remove(&db_pool, &opts.btfm_data_dir).await.unwrap();
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
                Ok(phrase_id) => {
                    db::ClipPhrase::insert(&db_pool, clip_id, phrase_id)
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
                let file = opts.btfm_data_dir.join(&clip.audio_file);
                if !file.exists() {
                    println!("{}", clip);
                    if clean {
                        clip.remove(&db_pool, &opts.btfm_data_dir).await.unwrap();
                    }
                }
            }

            let clip_dir = opts.btfm_data_dir.join("clips");
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
