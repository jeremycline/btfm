// SPDX-License-Identifier: GPL-2.0-or-later

use std::time::{SystemTime, UNIX_EPOCH};
use std::{collections::HashMap, process::Stdio};
use std::{io::Cursor, path::PathBuf};
use std::{path::Path, sync::Arc};

use byteorder::{ByteOrder, LittleEndian};
use chrono::NaiveDateTime;
use deepspeech::Model;
use log::{debug, error, info, trace, warn};
use rand::prelude::*;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use serenity::{
    async_trait,
    client::{Context, EventHandler},
    http::client::Http,
    model::{
        gateway::Ready,
        id::{ChannelId, GuildId},
        voice::VoiceState,
    },
    prelude::*,
};

use songbird::{
    input::Input,
    model::payload::{ClientConnect, ClientDisconnect, Speaking},
    Call, CoreEvent, Event, EventContext, EventHandler as VoiceEventHandler,
};

use crate::db;

pub struct HttpClient;
impl TypeMapKey for HttpClient {
    type Value = Arc<serenity::CacheAndHttp>;
}

pub struct BtfmData {
    data_dir: PathBuf,
    deepspeech_model: PathBuf,
    deepspeech_external_scorer: Option<PathBuf>,
    guild_id: GuildId,
    channel_id: ChannelId,
    log_channel_id: Option<ChannelId>,
    rate_adjuster: f64,
    users: HashMap<u32, User>,
    ssrc_map: HashMap<u64, u32>,
    // How many times the given user has joined the channel so we can give them rejoin messages.
    user_history: HashMap<u64, u32>,
    db: sqlx::PgPool,
}
impl TypeMapKey for BtfmData {
    type Value = Arc<Mutex<BtfmData>>;
}
impl BtfmData {
    #[allow(clippy::clippy::too_many_arguments)]
    pub fn new(
        data_dir: PathBuf,
        deepspeech_model: PathBuf,
        deepspeech_external_scorer: Option<PathBuf>,
        guild_id: u64,
        channel_id: u64,
        log_channel_id: Option<u64>,
        rate_adjuster: f64,
        db: sqlx::PgPool,
    ) -> BtfmData {
        BtfmData {
            data_dir,
            deepspeech_model,
            deepspeech_external_scorer,
            guild_id: GuildId(guild_id),
            channel_id: ChannelId(channel_id),
            log_channel_id: log_channel_id.map(ChannelId),
            rate_adjuster,
            users: HashMap::new(),
            ssrc_map: HashMap::new(),
            user_history: HashMap::new(),
            db,
        }
    }
}

/// Join or part from the configured voice channel. Called on startup and
/// if an event happens for the voice channel. If the bot is the only user in the
/// channel it will leave. If there's a non-bot user in the channel it'll join.
///
/// # Arguments
///
/// `context` - The serenity context for the event. Either when the ready event
///             fires or a user joins/parts/mutes/etc.
///
/// # Returns
///
/// true if it created a new connection, false otherwise.
async fn manage_voice_channel(context: &Context) -> bool {
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

    if let Ok(channel) = context
        .http
        .get_channel(*btfm_data.channel_id.as_u64())
        .await
    {
        match channel.guild() {
            Some(guild_channel) => {
                if let Ok(members) = guild_channel.members(&context.cache).await {
                    if members.iter().find(|m| !m.user.bot).is_none() {
                        if let Err(e) = manager.remove(btfm_data.guild_id).await {
                            info!("Failed to remove guild? {:?}", e);
                        }
                        btfm_data.user_history.clear();
                    } else if manager.get(btfm_data.guild_id).is_none() {
                        let (handler_lock, result) =
                            manager.join(btfm_data.guild_id, btfm_data.channel_id).await;
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
                                &btfm_data.guild_id, &btfm_data.channel_id
                            );
                        }
                    }
                }
            }
            None => {
                error!(
                    "{:?} is not a Guild channel and is not supported!",
                    btfm_data.channel_id
                );
            }
        }
    } else {
        warn!(
            "Unable to retrieve channel details for {:?}, ignoring voice state update.",
            &btfm_data.channel_id
        );
    }
    false
}

/// Return an AudioSource to greet a new user (or the channel at large).
async fn hello_there(btfm_data: &BtfmData, event_name: &str) -> Option<Input> {
    let hello = btfm_data.data_dir.join(event_name);
    if hello.exists() {
        Some(songbird::ffmpeg(hello).await.unwrap())
    } else {
        None
    }
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
        if manage_voice_channel(&context).await {
            return;
        }

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

        if let Some(locked_handler) = manager.get(btfm_data.guild_id) {
            // This event pertains to the channel we care about.
            let mut handler = locked_handler.lock().await;
            if Some(btfm_data.channel_id) == new.channel_id {
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

/// Convert the raw audio to text.
pub async fn voice_to_text(
    deepspeech_model: PathBuf,
    deepspeech_scorer: Option<PathBuf>,
    audio_buffer: Vec<i16>,
) -> String {
    let text = tokio::task::spawn_blocking(move || {
        let mut deepspeech_model =
            Model::load_from_files(&deepspeech_model).expect("Unable to load deepspeech model");
        if let Some(scorer) = &deepspeech_scorer {
            deepspeech_model.enable_external_scorer(scorer).unwrap();
        }
        info!("Successfully loaded voice recognition model");
        let result = deepspeech_model.speech_to_text(&audio_buffer).unwrap();
        info!("STT thinks someone said \"{}\"", result);
        result
    });

    text.await.unwrap_or_else(|_| "".to_string())
}

struct User {
    _packet_buffer: Mutex<Vec<i16>>,
}

impl User {
    pub fn new() -> User {
        User {
            _packet_buffer: Mutex::new(Vec::new()),
        }
    }
}

impl User {
    pub async fn push(&mut self, audio: &[i16]) {
        let mut buf = self._packet_buffer.lock().await;
        buf.extend(audio);
    }

    pub async fn reset(&mut self) -> Vec<i16> {
        let mut voice_data = self._packet_buffer.lock().await;
        let cloned_data = voice_data.clone();
        voice_data.clear();
        cloned_data
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

            EventContext::SpeakingUpdate { ssrc, speaking } => {
                debug!("SSRC {:?} speaking state update to {:?}", ssrc, speaking);
                if !speaking {
                    let http_and_cache = Arc::clone(&self.client_data)
                        .read()
                        .await
                        .get::<HttpClient>()
                        .cloned()
                        .expect("Expected HttpClient in TypeMap");

                    let locked_btfm_data = Arc::clone(&self.client_data)
                        .read()
                        .await
                        .get::<BtfmData>()
                        .cloned()
                        .expect("Expected voice manager");
                    let mut btfm_data = locked_btfm_data.lock().await;
                    let deepspeech_model = btfm_data.deepspeech_model.clone();
                    let rate_adjuster = btfm_data.rate_adjuster;
                    let deepspeech_scorer = btfm_data.deepspeech_external_scorer.clone();
                    if let Some(user) = btfm_data.users.get_mut(ssrc) {
                        let voice_data = user.reset().await;
                        let voice_data = discord_to_wav(voice_data, 16_000).await;
                        let text =
                            voice_to_text(deepspeech_model, deepspeech_scorer, voice_data).await;
                        if text.is_empty() {
                            return None;
                        }

                        let current_time = NaiveDateTime::from_timestamp(
                            SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .expect("It's time to check your system clock")
                                .as_secs() as i64,
                            0,
                        );
                        if !text.contains("excuse me")
                            && rate_limit(
                                db::last_play_time(&btfm_data.db).await,
                                current_time,
                                rate_adjuster,
                                &mut rand::thread_rng(),
                            )
                        {
                            let msg = format!("The bot heard `{:}`, but was rate-limited", &text);
                            log_event_to_channel(
                                btfm_data.log_channel_id,
                                &http_and_cache.http,
                                &msg,
                            )
                            .await;
                            return None;
                        }

                        let clips = db::match_phrase(&btfm_data.db, &text).await.unwrap();
                        let clip_count = clips.len();
                        let clip = clips.into_iter().choose(&mut rand::thread_rng());
                        if let Some(mut clip) = clip {
                            clip.mark_played(&btfm_data.db).await.unwrap();

                            let phrases = db::phrases_for_clip(&btfm_data.db, clip.id)
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
                                btfm_data.log_channel_id,
                                &http_and_cache.http,
                                &msg,
                            )
                            .await;

                            let source = songbird::ffmpeg(btfm_data.data_dir.join(clip.audio_file))
                                .await
                                .unwrap();
                            let mut call = self.locked_call.lock().await;
                            call.play_source(source);
                        } else {
                            let msg = format!("No phrases matched `{}`", &text);
                            log_event_to_channel(
                                btfm_data.log_channel_id,
                                &http_and_cache.http,
                                &msg,
                            )
                            .await;
                        }
                    }
                }
            }

            EventContext::VoicePacket { audio, packet, .. } => {
                trace!(
                    "Received voice packet from ssrc {}, sequence {}",
                    packet.ssrc,
                    packet.sequence.0,
                );
                let locked_client_data = Arc::clone(&self.client_data)
                    .read()
                    .await
                    .get::<BtfmData>()
                    .cloned()
                    .expect("Expected voice manager");
                let mut client_data = locked_client_data.lock().await;

                let user = client_data
                    .users
                    .entry(packet.ssrc)
                    .or_insert_with(User::new);

                if let Some(audio) = audio {
                    user.push(audio).await;
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

/// Prepare a file for DeepSpeech
pub async fn file_to_wav(audio: &Path, target_sample: i32) -> Vec<i16> {
    let mut ffmpeg = Command::new("ffmpeg")
        .args(&[
            "-i",
            audio.to_str().unwrap(),
            "-acodec",
            "pcm_s16le",
            "-f",
            "s16le",
            "-ar",
            target_sample.to_string().as_str(),
            "-ac",
            "1",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("Unable to spawn ffmpeg; is it installed?");

    let mut stdout = ffmpeg
        .stdout
        .take()
        .expect("Unable to get stdout for ffmpeg");

    tokio::spawn(async move {
        match ffmpeg.wait().await {
            Ok(status) => {
                info!("ffmpeg exited with {}", status);
            }
            Err(err) => {
                error!("ffmpeg encountered an error: {:?}", err);
            }
        }
    });

    let mut data: Vec<i16> = Vec::new();
    loop {
        let mut buf = [0; 2];
        match stdout.read_exact(&mut buf).await {
            Ok(_) => data.push(LittleEndian::read_i16(&buf)),
            Err(_) => break,
        }
    }
    return data;
}

/// Converts voice data to the target freqency, mono, and apply some ffmpeg filters.
///
/// This expects that the voice packets are all stereo, 16 bits per sample, and at
/// sampled at 48kHz. This is what Discord has documented it uses.
///
/// Returns: Audio prepped for deepspeech.
async fn discord_to_wav(voice_data: Vec<i16>, target_sample: u32) -> Vec<i16> {
    let data = Vec::<u8>::with_capacity(voice_data.len() * 2);
    let mut cursor = Cursor::new(data);
    // Convert audio to mono, at the sample rate of the deepspeech model, and trim silence
    // from the clip via ffmpeg. This is pretty hacky, but it works okay and we're not going
    // to Mars here.
    let mut ffmpeg = Command::new("ffmpeg")
        .args(&[
            "-f",
            "s16le",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-i",
            "pipe:0",
            "-f",
            "s16le",
            "-ar",
            target_sample.to_string().as_str(),
            "-ac",
            "1",
            "-af",
            "silenceremove=1:0:-50dB",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("Unable to spawn ffmpeg; is it installed?");

    let mut stdin = ffmpeg.stdin.take().expect("Unable to get stdin for ffmpeg");
    let mut stdout = ffmpeg
        .stdout
        .take()
        .expect("Unable to get stdout for ffmpeg");

    tokio::spawn(async move {
        match ffmpeg.wait().await {
            Ok(status) => {
                info!("ffmpeg exited with {}", status);
            }
            Err(err) => {
                error!("ffmpeg encountered an error: {:?}", err);
            }
        }
    });

    tokio::spawn(async move {
        for sample in voice_data.iter() {
            <Cursor<Vec<u8>> as byteorder::WriteBytesExt>::write_i16::<LittleEndian>(
                &mut cursor,
                *sample,
            )
            .unwrap();
        }
        let data = cursor.into_inner();
        info!("writing {} bytes to ffmpeg", data.len());
        stdin
            .write_all(&data)
            .await
            .expect("Failed to write to ffmpeg stdin");
    });

    let mut data: Vec<i16> = Vec::new();
    loop {
        let mut buf = [0; 2];
        match stdout.read_exact(&mut buf).await {
            Ok(_) => data.push(LittleEndian::read_i16(&buf)),
            Err(_) => break,
        }
    }
    return data;
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
