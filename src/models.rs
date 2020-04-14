// SPDX-License-Identifier: GPL-2.0-or-later

use crate::schema::clips;

#[derive(Queryable, Debug)]
pub struct Clip {
    pub id: i32,
    pub created_on: String,
    pub last_played: String,
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
