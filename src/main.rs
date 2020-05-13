// SPDX-License-Identifier: GPL-2.0-or-later

#[macro_use]
extern crate diesel_migrations;
#[macro_use]
extern crate log;
extern crate stderrlog;

use std::fs;
use std::sync::Arc;

use diesel::prelude::*;
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use serenity::{client::Client, framework::StandardFramework, prelude::*};
use structopt::StructOpt;

use btfm::cli;
use btfm::voice::{BtfmData, Handler, VoiceManager};
use btfm::{models, schema, DB_NAME};

embed_migrations!("migrations/");

fn main() {
    let opts = cli::Btfm::from_args();

    match opts {
        cli::Btfm::Run {
            channel_id,
            btfm_data_dir,
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
            let conn = SqliteConnection::establish(btfm_data_dir.join(DB_NAME).to_str().unwrap())
                .expect("Unabled to connect to database");
            embedded_migrations::run(&conn).expect("Failed to run database migrations!");
            let mut client = Client::new(&discord_token, Handler).expect("Unable to create client");
            {
                let mut data = client.data.write();
                data.insert::<VoiceManager>(Arc::clone(&client.voice_manager));
                data.insert::<BtfmData>(Arc::new(Mutex::new(BtfmData::new(
                    btfm_data_dir,
                    deepspeech_model,
                    deepspeech_scorer,
                    guild_id,
                    channel_id,
                    rate_adjuster,
                ))));
            }
            client.with_framework(StandardFramework::new());
            let _ = client
                .start()
                .map_err(|why| error!("Client ended: {:?}", why));
        }
        cli::Btfm::Clip(clip_subcommand) => match clip_subcommand {
            cli::Clip::Add {
                btfm_data_dir,
                description,
                file,
                phrase,
            } => {
                let conn =
                    SqliteConnection::establish(btfm_data_dir.join(DB_NAME).to_str().unwrap())
                        .expect("Unabled to connect to database");
                embedded_migrations::run(&conn).expect("Failed to run database migrations!");
                fs::create_dir_all(btfm_data_dir.join("clips"))
                    .expect("Unable to create clips directory");
                let clips_path = btfm_data_dir.join("clips");
                let file_prefix: String = thread_rng().sample_iter(&Alphanumeric).take(6).collect();
                let file_name = file
                    .file_name()
                    .expect("Path cannot terminate in ..")
                    .to_str()
                    .expect("File name is not valid UTF-8");
                let clip_destination = clips_path.join(file_prefix + "-" + file_name);
                fs::copy(&file, &clip_destination).expect("Unable to copy clip to data directory");
                let clip = models::NewClip {
                    phrase: &phrase,
                    description: &description,
                    audio_file: &clip_destination.to_str().unwrap(),
                };

                diesel::insert_into(schema::clips::table)
                    .values(&clip)
                    .execute(&conn)
                    .expect("Failed to save clip");
                println!("Added clip successfully");
            }
            cli::Clip::List { btfm_data_dir } => {
                let conn =
                    SqliteConnection::establish(btfm_data_dir.join(DB_NAME).to_str().unwrap())
                        .expect("Unabled to connect to database");
                embedded_migrations::run(&conn).expect("Failed to run database migrations!");
                let clips = schema::clips::table
                    .load::<models::Clip>(&conn)
                    .expect("Database query failed");
                for clip in clips {
                    println!("{:?}", clip);
                }
            }
            cli::Clip::Remove {
                btfm_data_dir,
                clip_id,
            } => {
                let conn =
                    SqliteConnection::establish(btfm_data_dir.join(DB_NAME).to_str().unwrap())
                        .expect("Unabled to connect to database");
                embedded_migrations::run(&conn).expect("Failed to run database migrations!");
                match diesel::delete(schema::clips::table.filter(schema::clips::id.eq(clip_id)))
                    .execute(&conn)
                {
                    Ok(count) => {
                        if count == 0 {
                            println!("There's no clip with id {}", clip_id);
                        } else {
                            println!("Removed {} clips", count);
                        }
                    }
                    Err(e) => {
                        println!("Unable to remove clip: {:?}", e);
                    }
                }
            }
        },
    }
}
