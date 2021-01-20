// SPDX-License-Identifier: GPL-2.0-or-later

#[macro_use]
extern crate diesel_migrations;
#[macro_use]
extern crate log;
extern crate stderrlog;

use std::fs;
use std::sync::Arc;

use diesel::prelude::*;
use serenity::{client::Client, framework::StandardFramework, prelude::*};
use songbird::{
    driver::{Config as DriverConfig, DecodeMode},
    SerenityInit, Songbird,
};
use structopt::StructOpt;

use btfm::voice::{BtfmData, Handler, HttpClient};
use btfm::{cli, db, schema, DB_NAME};

embed_migrations!("migrations/");

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() {
    let opts = cli::Btfm::from_args();
    let conn = SqliteConnection::establish(opts.btfm_data_dir.join(DB_NAME).to_str().unwrap())
        .expect("Unabled to connect to database");
    embedded_migrations::run(&conn).expect("Failed to run database migrations!");

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
                phrase,
            } => {
                fs::create_dir_all(opts.btfm_data_dir.join("clips"))
                    .expect("Unable to create clips directory");
                db::add_clip(&conn, &opts.btfm_data_dir, &file, &description, &phrase)
            }

            cli::Clip::Edit {
                clip_id,
                description,
                phrase,
            } => {
                db::edit_clip(&conn, clip_id, description, phrase);
            }

            cli::Clip::List {} => {
                for clip in db::all_clips(&conn) {
                    println!("{:?}", clip);
                }
            }

            cli::Clip::Remove { clip_id } => {
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
