//! Handlers for Discord voice channels.
//!
//! Serenity does not ship with direct support for voice channels. Instead,
//! support is provided via Songbird.

use std::path::PathBuf;
use std::sync::Arc;

use bytes::{BufMut, BytesMut};
use rand::prelude::*;
use regex::Regex;
use serenity::{async_trait, model::id::ChannelId, prelude::*};
use songbird::{
    model::payload::{ClientDisconnect, Speaking},
    Call, Event, EventContext, EventHandler as VoiceEventHandler,
};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, warn, Instrument};
use uuid::Uuid;

use super::{BtfmData, User};
use crate::db;

/// Return an AudioSource to greet a new user (or the channel at large).
pub async fn hello_there(event_name: &str) -> Option<songbird::input::File<PathBuf>> {
    crate::CONFIG
        .get()
        .map(|config| config.data_directory.join(event_name))
        .and_then(|f| if f.exists() { Some(f) } else { None })
        .map(songbird::input::File::new)
}

pub struct Receiver {
    btfm_data: Arc<Mutex<BtfmData>>,
    http: Arc<serenity::http::Http>,
    call: Arc<Mutex<Call>>,
}

impl Receiver {
    pub fn new(
        btfm_data: Arc<Mutex<BtfmData>>,
        http: Arc<serenity::http::Http>,
        call: Arc<Mutex<Call>>,
    ) -> Receiver {
        Receiver {
            btfm_data,
            http,
            call,
        }
    }
}

#[async_trait]
impl VoiceEventHandler for Receiver {
    async fn act(&self, context: &EventContext<'_>) -> Option<Event> {
        match context {
            EventContext::SpeakingStateUpdate(Speaking {
                speaking,
                ssrc,
                user_id,
                ..
            }) => {
                debug!("Got speaking update ({:?}) for {:?}", speaking, user_id);
                if let Some(user_id) = user_id {
                    self.btfm_data
                        .lock()
                        .await
                        .ssrc_map
                        .entry(user_id.0)
                        .or_insert(*ssrc);
                }
            }
            EventContext::VoiceTick(voice_tick) => {
                // Update all current speakers
                let mut btfm_data = self.btfm_data.lock().await;
                for (ssrc, voice_data) in voice_tick.speaking.iter() {
                    if let Some(user) = btfm_data.users.get_mut(ssrc) {
                        user.speaking = true;
                    }

                    let transcriber = &btfm_data.transcriber.clone();
                    let user = btfm_data.users.entry(*ssrc).or_insert_with(User::new);

                    // The user just started talking.
                    if user.transcriber.is_none() {
                        let (audio_sender, audio_receiver) = mpsc::channel(2048);
                        let span =
                            tracing::info_span!("stream", id = %Uuid::new_v4(), ssrc = %ssrc);
                        info!(parent: &span, "Beginning new transcription stream");
                        let text_receiver =
                            transcriber.stream(audio_receiver).instrument(span).await;
                        tokio::task::spawn(handle_text(
                            self.btfm_data.clone(),
                            self.http.clone(),
                            self.call.clone(),
                            text_receiver,
                        ));
                        user.transcriber = Some(audio_sender);
                    }

                    // Add the voice data we just got to the per-user transciption channel
                    let transcriber = user.transcriber.take();
                    if let Some(handle) = transcriber {
                        let audio = voice_data
                            .decoded_voice
                            .as_ref()
                            .expect("Error: Configure songbird to decode audio");
                        let mut buffer = BytesMut::with_capacity(audio.len() * 2);
                        for sample in audio.iter() {
                            buffer.put(sample.to_le_bytes().as_ref())
                        }
                        let buffer = buffer.freeze();
                        if handle.send(buffer).await.is_err() {
                            warn!("Failed to send audio to transcriber");
                        } else {
                            user.transcriber.replace(handle);
                        }
                    }
                }

                // All other users in the call who aren't currently talking.
                // If they were speaking the previous tick, finish up their transcription.
                for ssrc in voice_tick.silent.iter() {
                    if let Some(user) = btfm_data.users.get_mut(ssrc) {
                        user.speaking = false;
                        // This closes the audio sending channel which causes the worker to hang up the text
                        // sending channel, which causes the handle_text() function to break from its
                        // receiving loop and look for a clip match.
                        user.transcriber.take();
                    }
                }

                // Adjust the currently-playing clip if someone is speaking (or not)
                let call = self.call.lock().await;
                if let Some(track) = call.queue().current() {
                    if btfm_data.users.values().any(|user| user.speaking) {
                        if let Err(e) = track.set_volume(0.4) {
                            info!("Unable to lower volume playback: {}", e);
                        }
                    } else if let Err(e) = track.set_volume(1.0) {
                        info!("Unable to boost volume playback: {}", e);
                    }
                }
            }
            EventContext::ClientDisconnect(ClientDisconnect { user_id, .. }) => {
                debug!("User ({}) disconnected", user_id);
                let mut btfm_data = self.btfm_data.lock().await;

                if let Some(ssrc) = btfm_data.ssrc_map.remove(&user_id.0) {
                    btfm_data.users.remove(&ssrc);
                    debug!("Dropped voice buffer for {}", user_id);
                }
            }
            _ => {}
        }

        None
    }
}

/// A task to find and play audio clips when the speech has been transcribed
async fn handle_text(
    btfm_data: Arc<Mutex<BtfmData>>,
    http: Arc<serenity::http::Http>,
    call: Arc<Mutex<Call>>,
    text_receiver: oneshot::Receiver<String>,
) {
    lazy_static::lazy_static! {
        static ref RE: Regex = Regex::new(r"[^\w\s]").unwrap();
    }

    let punctuated_text = text_receiver.await.unwrap_or_default();
    if punctuated_text.trim().is_empty() {
        debug!("It didn't sound like anything to the bot");
        return;
    }
    let text = RE.replace_all(&punctuated_text, "").to_lowercase();

    let current_time = chrono::Utc::now().naive_utc();
    let mut btfm = btfm_data.lock().await;

    if text.contains("status report")
        || text.contains("freeze all motor functions")
        || text.contains("bot why did you say that")
        || text.contains("all together")
        || text.contains("altogether")
    {
        if let Some(endpoint) = btfm.config.mimic_endpoint.clone() {
            let cache_dir = btfm.config.data_directory.join("tts_cache/");
            if let Ok(voices) = crate::mimic::voices(&btfm.http_client, endpoint.clone()).await {
                let voice = voices.into_iter().choose(&mut rand::thread_rng());
                if let Some(voice) = voice {
                    let mut body = btfm
                        .status_report
                        .clone()
                        .unwrap_or_else(|| "It doesn't look like anything to me.".to_string());

                    if let Some(result) = all_together_now(&punctuated_text) {
                        body = result;
                    }
                    match crate::mimic::tts(&cache_dir, &btfm.http_client, endpoint, body, voice)
                        .await
                    {
                        Ok(input) => {
                            call.lock().await.enqueue_input(input.into()).await;
                        }
                        Err(err) => tracing::error!(err = %err, "Failed to create TTS audio"),
                    }
                } else {
                    tracing::error!("The mimic server didn't return any voices");
                }
            }
        }
    }

    let rate_adjuster = btfm.config.rate_adjuster;
    let mut conn = btfm.db.acquire().await.unwrap();
    if !text.contains("excuse me")
        && rate_limit(
            current_time - db::last_play_time(&mut conn).await,
            rate_adjuster,
            &mut rand::thread_rng(),
        )
    {
        debug!("Rate-limited and the user wasn't polite");
        return;
    }

    let clips = db::match_phrase(&mut conn, &text).await.unwrap();
    let clip_count = clips.len();
    let clip = clips.into_iter().choose(&mut rand::thread_rng());
    if let Some(mut clip) = clip {
        db::mark_played(&mut conn, &mut clip).await.unwrap();
        let phrases = db::phrases_for_clip(&mut conn, clip.uuid.clone())
            .await
            .unwrap_or_else(|_| vec![])
            .iter()
            .map(|p| format!("`{}`", &p.phrase))
            .collect::<Vec<String>>()
            .join(", ");
        let msg = format!(
            "This technological terror matched against {} clips;
            ```{}``` was randomly selected. Phrases that would trigger this clip: {}",
            clip_count, &clip, phrases
        );
        btfm.status_report = Some(format!(
            "The last clip that was played, described as \"{}\", is triggered by the phrases {}.",
            clip.description.unwrap_or_default(),
            phrases
        ));
        log_event_to_channel(btfm.config.log_channel_id.map(|i| i.into()), &http, &msg).await;

        let clip_path = btfm.config.data_directory.join(clip.audio_file);
        call.lock()
            .await
            .enqueue_input(songbird::input::File::new(clip_path).into())
            .await;
    } else {
        debug!("No phrases matched what the bot heard");
    }
}

/// Return true if we should not play a clip (i.e., we are rate limited).
///
/// # Arguments
///
/// `last_play` - The time a clip was last played.
/// `current_time` - The current time. Bet you didn't guess that.
/// `rate_adjuster` - Play chance is 1 - e^(-x/rate_adjuster). With 256 this is a 20% chance
///                   after a minute, 50% after 3 minutes, and 69% after 5 minutes.
/// `rng` - A Random number generator, used to add some spice to this otherwise cold, heartless
///         bot.
///
/// # Returns
///
/// true if a clip should not be played, or false if we should play a clip.
fn rate_limit(
    since_last_play: chrono::Duration,
    rate_adjuster: f64,
    rng: &mut dyn rand::RngCore,
) -> bool {
    debug!(
        "It's been {:?} since the last time a clip was played",
        since_last_play
    );
    let play_chance = 1.0 - (-since_last_play.num_seconds() as f64 / rate_adjuster).exp();
    info!(
        "Clips have a {} percent chance (repeating of course) of being played",
        play_chance * 100.0
    );
    let random_roll = rng.gen::<f64>();
    if random_roll > play_chance {
        info!(
            "Random roll of {} is higher than play chance {}; ignoring",
            random_roll, play_chance,
        );
        return true;
    }
    false
}

/// Send the given message to an optional channel.
async fn log_event_to_channel(
    channel_id: Option<ChannelId>,
    http_client: &Arc<serenity::http::Http>,
    message: &str,
) {
    if let Some(channel_id) = channel_id {
        let chan = http_client.get_channel(channel_id).await;
        if let Ok(chan) = chan {
            if let Some(chan) = chan.guild() {
                if let Err(e) = chan.say(http_client, message).await {
                    error!("Unable to send message to channel: {:?}", e);
                }
            } else {
                error!(
                    "Channel {:} is not a guild channel, not logging {:}",
                    channel_id, message
                );
            }
        } else {
            error!(
                "Did not get a valid channel for the log channel id {:}",
                channel_id
            );
        }
    } else {
        debug!(
            "No channel id provided to log events, not logging {:}",
            message
        );
    }
}

fn all_together_now(text: &str) -> Option<String> {
    lazy_static::lazy_static! {
        static ref ALL_TOGETHER: Regex = regex::RegexBuilder::new(r"(\s?)(al|all )together").case_insensitive(true).build().expect("Regex should be valid");
    }

    if let Some(x) = ALL_TOGETHER.find_iter(text).last() {
        let (text, _) = text.split_at(x.start());
        Some(text.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::all_together_now;

    #[test]
    fn test_all_together() {
        let result = all_together_now("There is no match.");
        assert_eq!(result, None)
    }

    #[test]
    fn test_all_together_match() {
        let result = all_together_now("A different type of flying all together.");
        assert_eq!(result, Some("A different type of flying".to_string()))
    }

    #[test]
    fn test_altogether_match() {
        let result = all_together_now("A different type of flying altogether.");
        assert_eq!(result, Some("A different type of flying".to_string()))
    }

    #[test]
    fn test_multi_altogether_match() {
        let result = all_together_now("It's an altogether different type of flying all together.");
        assert_eq!(
            result,
            Some("It's an altogether different type of flying".to_string())
        )
    }
}
