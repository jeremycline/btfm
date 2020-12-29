// SPDX-License-Identifier: GPL-2.0-or-later
//
// TODO: return errors rather than just logging stuff
use std::{fs, path::Path};

use diesel::prelude::*;
use log::{debug, error, info};
use rand::{distributions::Alphanumeric, prelude::*};

use crate::models;
use crate::schema;

/// Get the naive datetime for the last clip played
pub fn last_play_time(conn: &SqliteConnection) -> chrono::NaiveDateTime {
    let clip = schema::clips::table
        .order(schema::clips::last_played.desc())
        .first::<models::Clip>(conn)
        .expect("Database query failed");
    clip.last_played
}

/// Find clips that include the given phrase.
pub fn choose_clip(
    conn: &SqliteConnection,
    rng: &mut dyn rand::RngCore,
    phrase: &str,
) -> Option<models::Clip> {
    let clips = schema::clips::table
        .load::<models::Clip>(conn)
        .expect("Database query failed");
    let mut potential_clips = Vec::new();
    for clip in clips {
        if phrase.contains(&clip.phrase) {
            info!("Matched on '{}'", &clip.phrase);
            potential_clips.push(clip);
        }
    }

    if let Some(mut clip) = potential_clips.into_iter().choose(rng) {
        clip.plays += 1;
        clip.last_played = chrono::NaiveDateTime::from_timestamp(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("Check your system clock")
                .as_secs() as i64,
            0,
        );
        let filter = schema::clips::table.filter(schema::clips::id.eq(clip.id));
        let update = diesel::update(filter).set(&clip).execute(conn);
        match update {
            Ok(rows_updated) => {
                if rows_updated != 1 {
                    error!(
                        "Update applied to {} rows which is not expected",
                        rows_updated
                    );
                } else {
                    debug!("Updated the play count and last_played time successfully");
                }
            }
            Err(e) => {
                error!("Updating the clip resulted in {:?}", e);
            }
        }
        return Some(clip);
    }

    None
}

pub fn all_clips(conn: &SqliteConnection) -> Vec<models::Clip> {
    schema::clips::table
        .load::<models::Clip>(conn)
        .expect("Database query failed")
}

/// Add a new clip to the database.
pub fn add_clip(
    conn: &SqliteConnection,
    btfm_data_dir: &Path,
    file: &Path,
    description: &str,
    phrase: &str,
) {
    let clips_dir = btfm_data_dir.join("clips");
    let file_prefix: String = thread_rng().sample_iter(&Alphanumeric).take(6).collect();
    let file_name = file_prefix
        + "-"
        + file
            .file_name()
            .expect("Path cannot terminate in ..")
            .to_str()
            .expect("File name is not valid UTF-8");
    let clip_destination = clips_dir.join(&file_name);
    fs::copy(&file, &clip_destination).expect("Unable to copy clip to data directory");
    let clip = models::NewClip {
        phrase,
        description,
        audio_file: file_name.as_str(),
    };

    diesel::insert_into(schema::clips::table)
        .values(&clip)
        .execute(conn)
        .expect("Failed to save clip");
    info!("Added clip {:?} successfully", &file_name);
}

/// Edit an existing clip.
pub fn edit_clip(
    conn: &SqliteConnection,
    clip_id: i32,
    description: Option<String>,
    phrase: Option<String>,
) {
    let filter = schema::clips::table.filter(schema::clips::id.eq(clip_id));
    let mut clip = filter
        .load::<models::Clip>(conn)
        .expect("Database query fails");
    if let Some(mut clip) = clip.pop() {
        if let Some(description) = description {
            clip.description = description;
        }
        if let Some(phrase) = phrase {
            clip.phrase = phrase;
        }
        let update = diesel::update(filter).set(clip).execute(conn);
        match update {
            Ok(rows_updated) => {
                if rows_updated != 1 {
                    error!(
                        "Update applied to {} rows which is not expected",
                        rows_updated
                    );
                } else {
                    info!("Updated the play count and last_played time successfully");
                }
            }
            Err(e) => {
                error!("Updating the clip resulted in {:?}", e);
            }
        }
    } else {
        error!("No clip with id {} exists", clip_id);
    }
}

/// Remove a clip from the database.
pub fn remove_clip(conn: &SqliteConnection, clip_id: i32) {
    // TODO delete the actual file
    match diesel::delete(schema::clips::table.filter(schema::clips::id.eq(clip_id))).execute(conn) {
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
