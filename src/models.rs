// SPDX-License-Identifier: GPL-2.0-or-later
use chrono::NaiveDateTime;

use crate::schema::clips;

#[derive(Queryable, Debug, Identifiable, AsChangeset)]
pub struct Clip {
    pub id: i32,
    pub created_on: NaiveDateTime,
    pub last_played: NaiveDateTime,
    pub plays: i32,
    pub phrase: String,
    pub description: String,
    pub audio_file: String,
}

#[derive(Insertable)]
#[table_name = "clips"]
pub struct NewClip<'a> {
    pub phrase: &'a str,
    pub description: &'a str,
    pub audio_file: &'a str,
}
