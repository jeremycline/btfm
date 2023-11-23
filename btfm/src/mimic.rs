//! Mimic3 client to perform text-to-speech, if configured.
use std::{
    io::Write,
    path::{Path, PathBuf},
};

use cached::proc_macro::cached;
use reqwest::Url;
use sha2::Digest;
use tracing::instrument;

use crate::Error;

#[instrument(skip(client))]
pub(crate) async fn tts(
    cache_dir: &Path,
    client: &reqwest::Client,
    endpoint: Url,
    text: String,
    voice: String,
) -> Result<songbird::input::File<PathBuf>, Error> {
    let mut hasher = sha2::Sha256::new();
    hasher.update(text.as_bytes());
    hasher.update(voice.as_bytes());
    let key = hex::encode(hasher.finalize());
    //let key = hex::encode(sha2::Sha256::digest(text.as_bytes()));
    let dest_path = cache_dir.join(key);
    if dest_path.is_file() {
        tracing::info!(file = ?dest_path, "Playing existing TTS file");
        Ok(songbird::input::File::new(dest_path))
    } else {
        let tts_endpoint = endpoint.join("tts")?;
        let response = client
            .post(tts_endpoint)
            .query(&[
                ("voice", voice.as_str()),
                ("noiseScale", "0.667"),
                ("noiseW", "0.8"),
                ("lengthScale", "1.0"),
                ("ssml", "false"),
            ])
            .body(text)
            .send()
            .await?
            .error_for_status()?;

        let body = response.bytes().await?;
        let mut dest_file = std::fs::File::create(&dest_path)?;
        dest_file.write_all(&body)?;
        dest_file.sync_all()?;
        drop(dest_file);
        tracing::info!(file = ?dest_path, "Caching new TTS file");
        Ok(songbird::input::File::new(dest_path))
    }
}

#[instrument(skip_all)]
#[cached(
    result = true,
    key = "String",
    convert = r#"{ format!("{}", endpoint) }"#
)]
pub(crate) async fn voices(client: &reqwest::Client, endpoint: Url) -> Result<Vec<String>, Error> {
    let voice_endpoint = endpoint.join("voices")?;
    tracing::info!(url = %voice_endpoint, "Querying Mimic for available voices");
    let response = client
        .get(voice_endpoint)
        .send()
        .await?
        .error_for_status()?;
    let voices = response
        .json::<Vec<serde_json::Value>>()
        .await?
        .into_iter()
        .filter_map(|value| {
            let key = &value["key"];
            if key.is_null() {
                None
            } else {
                let key = key.to_string().replace('"', "");
                if key.starts_with("en") {
                    tracing::info!(key = %key, "Adding TTS language key");
                    Some(key)
                } else {
                    tracing::debug!(key = %key, "Ignoring non-english key");
                    None
                }
            }
        });

    Ok(voices.collect())
}
