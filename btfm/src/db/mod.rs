// SPDX-License-Identifier: GPL-2.0-or-later
//
// Provides structures and functions for interacting with the database.

mod clip;
mod phrase;

pub use clip::{
    add_clip, clips_list, get_clip, last_play_time, mark_played, match_phrase, remove_clip,
    update_clip,
};
pub use phrase::{add_phrase, get_phrase, list_phrases, phrases_for_clip, remove_phrase, Phrase};
