//! Handlers for Discord voice channels.
//!
//! Serenity does not ship with direct support for voice channels. Instead,
//! support is provided via Songbird.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::NaiveDateTime;
use rand::prelude::*;
use serenity::CacheAndHttp;
use serenity::{async_trait, http::client::Http, model::id::ChannelId, prelude::*};
use songbird::{
    input::Input,
    model::payload::{ClientDisconnect, Speaking},
    Call, Event, EventContext, EventHandler as VoiceEventHandler,
};
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn, Instrument};
use ulid::Ulid;

use super::{BtfmData, User};
use crate::db;

/// Return an AudioSource to greet a new user (or the channel at large).
pub async fn hello_there(event_name: &str) -> Option<Input> {
    let hello = crate::CONFIG
        .get()
        .map(|config| config.data_directory.join(event_name))
        .and_then(|f| if f.exists() { Some(f) } else { None });
    if let Some(path) = hello {
        Some(songbird::ffmpeg(path).await.unwrap())
    } else {
        None
    }
}

pub struct Receiver {
    btfm_data: Arc<Mutex<BtfmData>>,
    http: Arc<CacheAndHttp>,
    call: Arc<Mutex<Call>>,
}

impl Receiver {
    pub fn new(
        btfm_data: Arc<Mutex<BtfmData>>,
        http: Arc<CacheAndHttp>,
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

            EventContext::SpeakingUpdate(data) => {
                let (ssrc, speaking) = (data.ssrc, data.speaking);
                debug!("SSRC {:?} speaking state update to {:?}", ssrc, speaking);
                if speaking {
                    let mut btfm_data = self.btfm_data.lock().await;
                    if let Some(user) = btfm_data.users.get_mut(&ssrc) {
                        user.speaking = true;
                    }
                    drop(btfm_data);

                    let call = self.call.lock().await;
                    if let Some(track) = call.queue().current() {
                        if let Some(duration) = track.metadata().duration {
                            if duration > core::time::Duration::new(3, 0) {
                                if let Err(e) = track.set_volume(0.4) {
                                    info!("Unable to lower volume of playback: {}", e);
                                }
                            } else {
                                info!("Track less than 3 seconds; playing at full volume");
                            }
                        }
                    }
                } else {
                    let mut btfm_data = self.btfm_data.lock().await;
                    if let Some(user) = btfm_data.users.get_mut(&ssrc) {
                        user.speaking = false;
                    }

                    // Bump the clip volume back up if no one is talking
                    if btfm_data.users.values().all(|user| !user.speaking) {
                        let call = self.call.lock().await;
                        if let Some(track) = call.queue().current() {
                            if let Err(e) = track.set_volume(1.0) {
                                info!("Unable to boost volume playback: {}", e);
                            }
                        }
                    }

                    if let Some(user) = btfm_data.users.get_mut(&ssrc) {
                        // This closes the audio sending channel which causes the worker to hang up the text
                        // sending channel, which causes the handle_text() function to break from its
                        // receiving loop and look for a clip match.
                        user.transcriber.take();
                    }
                }
            }

            EventContext::VoicePacket(voice_data) => {
                let (packet, audio) = (voice_data.packet, voice_data.audio);
                trace!(
                    "Received voice packet from ssrc {}, sequence {}",
                    packet.ssrc,
                    packet.sequence.0,
                );
                let mut btfm_data = self.btfm_data.lock().await;

                let transcriber = &btfm_data.transcriber.clone();
                let user = btfm_data.users.entry(packet.ssrc).or_insert_with(User::new);

                if let Some(audio) = audio {
                    if user.transcriber.is_none() {
                        let (audio_sender, audio_receiver) = mpsc::channel(2048);
                        let span =
                            tracing::info_span!("stream", id = %Ulid::new(), ssrc = %packet.ssrc);
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

                    let transcriber = user.transcriber.take();
                    if let Some(handle) = transcriber {
                        if handle.send(audio.to_vec()).await.is_err() {
                            warn!("Failed to send audio to transcriber");
                        } else {
                            user.transcriber.replace(handle);
                        }
                    }
                } else {
                    error!("RTP packet event received, but there was no audio. Decode support may not be enabled?");
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
    http_client: Arc<CacheAndHttp>,
    call: Arc<Mutex<Call>>,
    text_receiver: mpsc::Receiver<String>,
) {
    let mut snippets = Vec::new();
    let mut receiver = text_receiver;
    while let Some(snippet) = receiver.recv().await {
        snippets.push(snippet.replace('\"', ""));
    }
    let text = snippets.join(" ");
    if text.trim().is_empty() {
        info!("It didn't sound like anything to the bot");
        return;
    }

    let current_time = NaiveDateTime::from_timestamp(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("It's time to check your system clock")
            .as_secs() as i64,
        0,
    );
    let btfm = btfm_data.lock().await;
    let rate_adjuster = btfm.config.rate_adjuster;
    let mut conn = btfm.db.acquire().await.unwrap();
    if !text.contains("excuse me")
        && rate_limit(
            db::last_play_time(&mut conn).await,
            current_time,
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
        let phrases = db::phrases_for_clip(&mut conn, clip.uuid)
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
        log_event_to_channel(
            btfm.config.log_channel_id.map(ChannelId),
            &http_client.http,
            &msg,
        )
        .await;

        let source = songbird::ffmpeg(btfm.config.data_directory.join(clip.audio_file))
            .await
            .unwrap();
        let mut unlocked_call = call.lock().await;
        unlocked_call.enqueue_source(source);
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
    last_play: chrono::NaiveDateTime,
    current_time: chrono::NaiveDateTime,
    rate_adjuster: f64,
    rng: &mut dyn rand::RngCore,
) -> bool {
    let since_last_play = current_time - last_play;
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
    http_client: &Arc<Http>,
    message: &str,
) {
    if let Some(channel_id) = channel_id {
        let chan = http_client.get_channel(*channel_id.as_u64()).await;
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
