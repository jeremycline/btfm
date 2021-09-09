//! Handlers for Discord non-voice events.

use std::sync::Arc;

use log::{debug, info};

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

use super::voice::{hello_there, manage_voice_channel};
use super::BtfmData;

pub struct HttpClient;
impl TypeMapKey for HttpClient {
    type Value = Arc<serenity::CacheAndHttp>;
}

pub struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, context: Context, ready: Ready) {
        info!("Connected as {}", ready.user.name);
        manage_voice_channel(&context).await;
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
        manage_voice_channel(&context).await;

        let manager = songbird::get(&context)
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

        if let Some(locked_handler) = manager.get(btfm_data.config.guild_id) {
            // This event pertains to the channel we care about.
            let mut handler = locked_handler.lock().await;
            if Some(ChannelId(btfm_data.config.channel_id)) == new.channel_id {
                match old {
                    Some(old_state) => {
                        // Order matters here, the UI mutes users who deafen
                        // themselves so look at deafen events before dealing
                        // with muting
                        if old_state.self_deaf != new.self_deaf && new.self_deaf {
                            debug!("Someone deafened themselves in a channel we care about");
                            hello_there(&btfm_data, "deaf")
                                .await
                                .map(|s| Some(handler.play_source(s)));
                        } else if old_state.self_deaf != new.self_deaf && !new.self_deaf {
                            debug!("Someone un-deafened themselves in a channel we care about");
                            hello_there(&btfm_data, "undeaf")
                                .await
                                .map(|s| Some(handler.play_source(s)));
                        } else if old_state.self_mute != new.self_mute && new.self_mute {
                            debug!("Someone muted in the channel we care about");
                            hello_there(&btfm_data, "mute")
                                .await
                                .map(|s| Some(handler.play_source(s)));
                        } else if old_state.self_mute != new.self_mute && !new.self_mute {
                            debug!("Someone un-muted in the channel we care about");
                            hello_there(&btfm_data, "unmute")
                                .await
                                .map(|s| Some(handler.play_source(s)));
                        }
                    }
                    None => {
                        debug!("User just joined our channel");
                        hello_there(&btfm_data, "hello")
                            .await
                            .map(|s| Some(handler.play_source(s)));
                        let join_count = btfm_data
                            .user_history
                            .entry(*new.user_id.as_u64())
                            .or_insert(0);
                        *join_count += 1;
                        if *join_count > 1 {
                            info!("Someone just rejoined; let them know how we feel");
                            let rng: f64 = rand::random();
                            if 1_f64 - (*join_count as f64 * 0.1).exp() > rng {
                                hello_there(&btfm_data, "rejoin")
                                    .await
                                    .map(|s| Some(handler.play_source(s)));
                            }
                        }
                    }
                }
                info!("user_history={:?}", &btfm_data.user_history);
            }
        }
    }
}
