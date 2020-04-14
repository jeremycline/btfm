#[macro_use]
extern crate log;
extern crate stderrlog;

use std::fs;
use std::sync::Arc;

use diesel::prelude::*;
use serenity::{client::Client, framework::StandardFramework, prelude::*};
use structopt::StructOpt;

use btfm::cli;
use btfm::voice::{BtfmData, Handler, VoiceManager};
use btfm::{models, schema, DB_NAME};

fn main() {
    let opts = cli::Btfm::from_args();

    match opts {
        cli::Btfm::Run {
            channel_id,
            btfm_data_dir,
            deepspeech_model_dir,
            discord_token,
            guild_id,
            verbose,
        } => {
            stderrlog::new()
                .module(module_path!())
                .verbosity(verbose)
                .timestamp(stderrlog::Timestamp::Second)
                .init()
                .unwrap();
            let mut client = Client::new(&discord_token, Handler).expect("Unable to create client");
            {
                let mut data = client.data.write();
                data.insert::<VoiceManager>(Arc::clone(&client.voice_manager));
                data.insert::<BtfmData>(Arc::new(Mutex::new(BtfmData::new(
                    btfm_data_dir,
                    deepspeech_model_dir,
                    guild_id,
                    channel_id,
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
                fs::create_dir_all(btfm_data_dir.join("clips"))
                    .expect("Unable to create clips directory");
                let clip_path = btfm_data_dir
                    .join("clips")
                    .join(&file.file_name().expect("Invalid file name"));
                fs::copy(&file, &clip_path).expect("Unable to copy clip to data directory");
                let clip = models::NewClip {
                    phrase: &phrase,
                    description: &description,
                    audio_file: &clip_path.to_str().unwrap(),
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
