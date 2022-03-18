//! Handlers for Discord non-voice events.

use std::sync::Arc;

use songbird::CoreEvent;
use tracing::{debug, error, info, warn};

use serenity::{
    async_trait,
    client::{Context, EventHandler},
    model::{
        gateway::Ready,
        id::{ChannelId, GuildId},
        voice::VoiceState,
    },
    prelude::*,
};

use super::voice::{hello_there, Receiver};
use super::BtfmData;

pub struct HttpClient;
impl TypeMapKey for HttpClient {
    type Value = Arc<serenity::CacheAndHttp>;
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
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, context: Context, ready: Ready) {
        info!("Connected as {}", ready.user.name);
        self.manage_voice_channel(&context).await;
    }

    async fn voice_state_update(
        &self,
        context: Context,
        guild_id: Option<GuildId>,
        old: Option<VoiceState>,
        new: VoiceState,
    ) {
        if guild_id.is_none() {
            return;
        }

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
