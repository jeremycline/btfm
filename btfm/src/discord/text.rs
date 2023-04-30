//! Handlers for Discord non-voice events.

use std::{sync::Arc, time::Duration};

use rand::prelude::IteratorRandom;
use serenity::{
    async_trait,
    client::{Context, EventHandler},
    model::{gateway::Ready, id::ChannelId, voice::VoiceState},
    prelude::*,
};
use songbird::{Call, CoreEvent};
use sqlx::SqlitePool;
use tracing::{debug, error, info, warn};

use crate::db::Clip;

use super::voice::{hello_there, Receiver};
use super::BtfmData;

pub struct HttpClient;
impl TypeMapKey for HttpClient {
    type Value = Arc<serenity::CacheAndHttp>;
}

async fn play_clip_at_interval(call: Arc<Mutex<Call>>, db_pool: SqlitePool) {
    info!("Starting task to play clips an interval");
    let config = crate::CONFIG
        .get()
        .expect("Configuration must be set prior to starting the Serenity app");

    loop {
        tokio::time::sleep(Duration::from_secs(config.random_clip_interval)).await;
        {
            let handle = call.lock().await;
            if handle.current_connection().is_none() {
                info!("Shutting down the random clip player since we don't seem to be connected");
                return;
            }
        }
        {
            let mut conn = db_pool.acquire().await.unwrap();
            if let Ok(clips) = crate::db::clips_list(&mut conn).await {
                if let Some(clip) = select_clip(clips) {
                    if let Ok(source) =
                        songbird::ffmpeg(config.data_directory.join(clip.audio_file)).await
                    {
                        info!("Playing a random clip to keep things spicy");
                        let mut handle = call.lock().await;
                        handle.enqueue_source(source);
                    }
                }
            }
        }
    }
}

/// This function only exists to work around the compiler being upset that the RNG might be used
/// after an await, and even dropping it immediately doesn't help.
fn select_clip(clips: Vec<Clip>) -> Option<Clip> {
    clips.into_iter().choose(&mut rand::thread_rng())
}

pub struct Handler;

impl Handler {
    /// Join or part from the configured voice channel. Called on startup and
    /// if an event happens for the voice channel. If the bot is the only user in the
    /// channel it will leave. If there's a non-bot user in the channel it'll join.
    ///
    /// # Arguments
    ///
    /// `context` - The serenity context for the event. Either when the ready event
    ///             fires or a user joins/parts/mutes/etc.
    async fn manage_voice_channel(&self, context: &Context) {
        let manager = songbird::get(context)
            .await
            .expect("Songbird client missing")
            .clone();

        let btfm_data = context
            .data
            .read()
            .await
            .get::<BtfmData>()
            .cloned()
            .expect("Expected BtfmData in TypeMap");
        let mut btfm = btfm_data.lock().await;

        if let Ok(channel) = context.http.get_channel(btfm.config.channel_id).await {
            match channel.guild() {
                Some(guild_channel) => {
                    if let Ok(members) = guild_channel.members(&context.cache).await {
                        if !members.iter().any(|m| !m.user.bot) {
                            if let Err(e) = manager.remove(btfm.config.guild_id).await {
                                tracing::trace!("Failed to remove guild? {:?}", e);
                            }
                            btfm.user_history.clear();
                        } else if manager.get(btfm.config.guild_id).is_none() {
                            let (call, result) = manager
                                .join(btfm.config.guild_id, btfm.config.channel_id)
                                .await;
                            if result.is_ok() {
                                tokio::spawn(play_clip_at_interval(
                                    Arc::clone(&call),
                                    btfm.db.clone(),
                                ));

                                let mut handler = call.lock().await;
                                let http = context
                                    .data
                                    .read()
                                    .await
                                    .get::<HttpClient>()
                                    .cloned()
                                    .expect("Expected HTTP client");
                                handler.add_global_event(
                                    CoreEvent::SpeakingUpdate.into(),
                                    Receiver::new(
                                        Arc::clone(&btfm_data),
                                        Arc::clone(&http),
                                        Arc::clone(&call),
                                    ),
                                );
                                handler.add_global_event(
                                    CoreEvent::VoicePacket.into(),
                                    Receiver::new(
                                        Arc::clone(&btfm_data),
                                        Arc::clone(&http),
                                        Arc::clone(&call),
                                    ),
                                );
                                handler.add_global_event(
                                    CoreEvent::ClientDisconnect.into(),
                                    Receiver::new(
                                        Arc::clone(&btfm_data),
                                        Arc::clone(&http),
                                        Arc::clone(&call),
                                    ),
                                );
                            } else {
                                error!(
                                    "Unable to join {:?} on {:?}",
                                    &btfm.config.guild_id, &btfm.config.channel_id
                                );
                            }
                        }
                    }
                }
                None => {
                    error!(
                        "{:?} is not a Guild channel and is not supported!",
                        btfm.config.channel_id
                    );
                }
            }
        } else {
            warn!(
                "Unable to retrieve channel details for {:?}, ignoring voice state update.",
                btfm.config.channel_id
            );
        }
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, context: Context, ready: Ready) {
        info!("Connected as {}", ready.user.name);
        self.manage_voice_channel(&context).await;
    }

    async fn voice_state_update(&self, context: Context, old: Option<VoiceState>, new: VoiceState) {
        debug!("voice_state_update: old={:?}  new={:?}", old, new);
        self.manage_voice_channel(&context).await;

        let manager = songbird::get(&context)
            .await
            .expect("Songbird client missing")
            .clone();
        let config = crate::CONFIG.get().unwrap();

        if let Some(locked_handler) = manager.get(config.guild_id) {
            // This event pertains to the channel we care about.
            let mut handler = locked_handler.lock().await;
            if Some(ChannelId(config.channel_id)) == new.channel_id {
                match old {
                    Some(old_state) => {
                        // Order matters here, the UI mutes users who deafen
                        // themselves so look at deafen events before dealing
                        // with muting
                        if old_state.self_deaf != new.self_deaf && new.self_deaf {
                            debug!("Someone deafened themselves in a channel we care about");
                            hello_there("deaf")
                                .await
                                .map(|s| Some(handler.play_source(s)));
                        } else if old_state.self_deaf != new.self_deaf && !new.self_deaf {
                            debug!("Someone un-deafened themselves in a channel we care about");
                            hello_there("undeaf")
                                .await
                                .map(|s| Some(handler.play_source(s)));
                        } else if old_state.self_mute != new.self_mute && new.self_mute {
                            debug!("Someone muted in the channel we care about");
                            hello_there("mute")
                                .await
                                .map(|s| Some(handler.play_source(s)));
                        } else if old_state.self_mute != new.self_mute && !new.self_mute {
                            debug!("Someone un-muted in the channel we care about");
                            hello_there("unmute")
                                .await
                                .map(|s| Some(handler.play_source(s)));
                        }
                    }
                    None => {
                        debug!("User just joined our channel");
                        hello_there("hello")
                            .await
                            .map(|s| Some(handler.play_source(s)));
                        let join_count = increment_join_count(context, *new.user_id.as_u64()).await;
                        if join_count > 1 {
                            info!("Someone just rejoined; let them know how we feel");
                            let rng: f64 = rand::random();
                            if 1_f64 - (join_count as f64 * 0.1).exp() > rng {
                                hello_there("rejoin")
                                    .await
                                    .map(|s| Some(handler.play_source(s)));
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn increment_join_count(context: Context, user_id: u64) -> u32 {
    let btfm_data_lock = context
        .data
        .read()
        .await
        .get::<BtfmData>()
        .cloned()
        .expect("Expected BtfmData in TypeMap");
    let mut btfm_data = btfm_data_lock.lock().await;
    let join_count = btfm_data.user_history.entry(user_id).or_insert(0);
    *join_count += 1;
    *join_count
}
