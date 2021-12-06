//! Handlers for Discord voice channels.
//!
//! Serenity does not ship with direct support for voice channels. Instead,
//! support is provided via Songbird.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::NaiveDateTime;
use rand::prelude::*;
use serenity::CacheAndHttp;
use serenity::{
    async_trait, client::Context, http::client::Http, model::id::ChannelId, prelude::*,
};
use songbird::{
    input::Input,
    model::payload::{ClientConnect, ClientDisconnect, Speaking},
    Call, CoreEvent, Event, EventContext, EventHandler as VoiceEventHandler,
};
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn, Instrument};
use ulid::Ulid;

use super::text::HttpClient;
use super::{BtfmData, User};
use crate::db;

/// Join or part from the configured voice channel. Called on startup and
/// if an event happens for the voice channel. If the bot is the only user in the
/// channel it will leave. If there's a non-bot user in the channel it'll join.
///
/// # Arguments
///
/// `context` - The serenity context for the event. Either when the ready event
///             fires or a user joins/parts/mutes/etc.
pub async fn manage_voice_channel(context: &Context) {
    let manager = songbird::get(context)
        .await
        .expect("Songbird client missing")
        .clone();

    let btfm_data_lock = context
        .data
        .read()
        .await
        .get::<BtfmData>()
        .cloned()
        .expect("Expected BtfmData in TypeMap");
    let mut btfm_data = btfm_data_lock.lock().await;

    if let Ok(channel) = context.http.get_channel(btfm_data.config.channel_id).await {
        match channel.guild() {
            Some(guild_channel) => {
                if let Ok(members) = guild_channel.members(&context.cache).await {
                    if !members.iter().any(|m| !m.user.bot) {
                        if let Err(e) = manager.remove(btfm_data.config.guild_id).await {
                            info!("Failed to remove guild? {:?}", e);
                        }
                        btfm_data.user_history.clear();
                    } else if manager.get(btfm_data.config.guild_id).is_none() {
                        let (handler_lock, result) = manager
                            .join(btfm_data.config.guild_id, btfm_data.config.channel_id)
                            .await;
                        if result.is_ok() {
                            let mut handler = handler_lock.lock().await;
                            handler.add_global_event(
                                CoreEvent::SpeakingUpdate.into(),
                                Receiver::new(context.data.clone(), handler_lock.clone()),
                            );
                            handler.add_global_event(
                                CoreEvent::VoicePacket.into(),
                                Receiver::new(context.data.clone(), handler_lock.clone()),
                            );
                            handler.add_global_event(
                                CoreEvent::ClientConnect.into(),
                                Receiver::new(context.data.clone(), handler_lock.clone()),
                            );
                            handler.add_global_event(
                                CoreEvent::ClientDisconnect.into(),
                                Receiver::new(context.data.clone(), handler_lock.clone()),
                            );
                        } else {
                            error!(
                                "Unable to join {:?} on {:?}",
                                &btfm_data.config.guild_id, &btfm_data.config.channel_id
                            );
                        }
                    }
                }
            }
            None => {
                error!(
                    "{:?} is not a Guild channel and is not supported!",
                    btfm_data.config.channel_id
                );
            }
        }
    } else {
        warn!(
            "Unable to retrieve channel details for {:?}, ignoring voice state update.",
            btfm_data.config.channel_id
        );
    }
}

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
    client_data: Arc<RwLock<TypeMap>>,
    locked_call: Arc<Mutex<Call>>,
}

impl Receiver {
    pub fn new(client_data: Arc<RwLock<TypeMap>>, locked_call: Arc<Mutex<Call>>) -> Receiver {
        Receiver {
            client_data,
            locked_call,
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
                let locked_btfm_data = Arc::clone(&self.client_data)
                    .read()
                    .await
                    .get::<BtfmData>()
                    .cloned()
                    .expect("Expected voice manager");
                let mut btfm_data = locked_btfm_data.lock().await;
                if let Some(user_id) = user_id {
                    btfm_data.ssrc_map.entry(user_id.0).or_insert(*ssrc);
                }
            }

            EventContext::SpeakingUpdate(data) => {
                let (ssrc, speaking) = (data.ssrc, data.speaking);
                debug!("SSRC {:?} speaking state update to {:?}", ssrc, speaking);
                if speaking {
                    let locked_btfm_data = Arc::clone(&self.client_data)
                        .read()
                        .await
                        .get::<BtfmData>()
                        .cloned()
                        .expect("Expected voice manager");

                    let mut btfm_data = locked_btfm_data.lock().await;
                    if let Some(user) = btfm_data.users.get_mut(&ssrc) {
                        user.speaking = true;
                    }
                    drop(btfm_data);

                    let call = self.locked_call.lock().await;
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
                    let locked_btfm_data = Arc::clone(&self.client_data)
                        .read()
                        .await
                        .get::<BtfmData>()
                        .cloned()
                        .expect("Expected voice manager");

                    let mut btfm_data = locked_btfm_data.lock().await;
                    if let Some(user) = btfm_data.users.get_mut(&ssrc) {
                        user.speaking = false;
                    }

                    // Bump the clip volume back up if no one is talking
                    if btfm_data.users.values().all(|user| !user.speaking) {
                        let call = self.locked_call.lock().await;
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
                let locked_btfm_data = Arc::clone(&self.client_data)
                    .read()
                    .await
                    .get::<BtfmData>()
                    .cloned()
                    .expect("Expected voice manager");
                let mut btfm_data = locked_btfm_data.lock().await;
                let http_and_cache = Arc::clone(&self.client_data)
                    .read()
                    .await
                    .get::<HttpClient>()
                    .cloned()
                    .expect("Expected HttpClient in TypeMap");

                let transcriber = &btfm_data.transcriber.clone();
                let user = btfm_data.users.entry(packet.ssrc).or_insert_with(User::new);

                if let Some(audio) = audio {
                    if user.transcriber.is_none() {
                        let (audio_sender, audio_receiver) = mpsc::channel(2048);
                        let span = tracing::info_span!("stream", id = %Ulid::new());
                        let text_receiver =
                            transcriber.stream(audio_receiver).instrument(span).await;
                        let locked_call = self.locked_call.clone();
                        tokio::task::spawn(handle_text(
                            locked_btfm_data.clone(),
                            http_and_cache,
                            locked_call,
                            text_receiver,
                        ));
                        user.transcriber = Some(audio_sender);
                    }
                    if let Some(transcriber) = &user.transcriber {
                        if let Err(e) = transcriber.send(audio.to_vec()).await {
                            warn!("Failed to send audio to transcriber: {:?}", e);
                        }
                    }
                } else {
                    error!("RTP packet event received, but there was no audio. Decode support may not be enabled?");
                }
            }

            EventContext::ClientConnect(ClientConnect {
                audio_ssrc,
                user_id,
                ..
            }) => {
                debug!("New user ({}) connected", user_id);
                let locked_btfm_data = Arc::clone(&self.client_data)
                    .read()
                    .await
                    .get::<BtfmData>()
                    .cloned()
                    .expect("Expected voice manager");
                let mut btfm_data = locked_btfm_data.lock().await;
                btfm_data.ssrc_map.entry(user_id.0).or_insert(*audio_ssrc);
            }

            EventContext::ClientDisconnect(ClientDisconnect { user_id, .. }) => {
                debug!("User ({}) disconnected", user_id);
                let locked_btfm_data = Arc::clone(&self.client_data)
                    .read()
                    .await
                    .get::<BtfmData>()
                    .cloned()
                    .expect("Expected voice manager");
                let mut btfm_data = locked_btfm_data.lock().await;

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
        snippets.push(snippet);
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
    if !text.contains("excuse me")
        && rate_limit(
            db::last_play_time(&btfm.db).await,
            current_time,
            rate_adjuster,
            &mut rand::thread_rng(),
        )
    {
        let msg = format!("The bot heard `{:}`, but was rate-limited", &text);
        log_event_to_channel(
            btfm.config.log_channel_id.map(ChannelId),
            &http_client.http,
            &msg,
        )
        .await;
        return;
    }

    let clips = db::match_phrase(&btfm.db, &text).await.unwrap();
    let clip_count = clips.len();
    let clip = clips.into_iter().choose(&mut rand::thread_rng());
    if let Some(mut clip) = clip {
        clip.mark_played(&btfm.db).await.unwrap();
        let mut conn = btfm.db.acquire().await.unwrap();
        let phrases = db::phrases_for_clip(&mut conn, clip.uuid)
            .await
            .unwrap_or_else(|_| vec![])
            .iter()
            .map(|p| format!("`{}`", &p.phrase))
            .collect::<Vec<String>>()
            .join(", ");
        let msg = format!(
                                        "This technological terror heard `{:}`, which matched against {} clips;
                                        ```{}``` was randomly selected. Phrases that would trigger this clip: {}",
                                        &text, clip_count, &clip, phrases);
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
        let msg = format!("No phrases matched `{}`", &text);
        log_event_to_channel(
            btfm.config.log_channel_id.map(ChannelId),
            &http_client.http,
            &msg,
        )
        .await;
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