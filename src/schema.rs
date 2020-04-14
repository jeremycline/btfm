// SPDX-License-Identifier: GPL-2.0-or-later

table! {
    clips (id) {
        id -> Integer,
        created_on -> Timestamp,
        last_played -> Timestamp,
        plays -> Integer,
        phrase -> Text,
        description -> Text,
        audio_file -> Text,
    }
}
