// SPDX-License-Identifier: GPL-2.0-or-later

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Cursor;
use std::io::{Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use audrey::hound;
use audrey::read::Reader;
use audrey::sample::interpolate::{Converter, Linear};
use audrey::sample::signal::{from_iter, Signal};
use chrono::NaiveDateTime;
use deepspeech::Model;
use diesel::prelude::*;
use log::{debug, error, info, trace, warn};
use rand::prelude::*;
use serenity::{
    client::{bridge::voice::ClientVoiceManager, Context, EventHandler},
    model::{
        gateway::Ready,
        id::{ChannelId, GuildId},
        voice::VoiceState,
    },
    prelude::*,
    voice,
};

use crate::models;
use crate::schema;
use crate::DB_NAME;

/// The sample rate (in HZ) that DeepSpeech expects audio to be in.
/// Discord currently sends 48kHz stereo audio, so we need to convert
/// it to mono audio at this sample rate.
const SAMPLE_RATE: u32 = 16_000;

pub struct VoiceManager;
impl TypeMapKey for VoiceManager {
    type Value = Arc<Mutex<ClientVoiceManager>>;
}

pub struct BtfmData {
    data_dir: PathBuf,
    deepspeech_model: PathBuf,
    deepspeech_external_scorer: Option<PathBuf>,
    guild_id: GuildId,
    channel_id: ChannelId,
    rate_adjuster: f64,
}
impl TypeMapKey for BtfmData {
    type Value = Arc<Mutex<BtfmData>>;
}
impl BtfmData {
    pub fn new(
        data_dir: PathBuf,
        deepspeech_model: PathBuf,
        deepspeech_external_scorer: Option<PathBuf>,
        guild_id: u64,
        channel_id: u64,
        rate_adjuster: f64,
    ) -> BtfmData {
        BtfmData {
            data_dir,
            deepspeech_model,
            deepspeech_external_scorer,
            guild_id: GuildId(guild_id),
            channel_id: ChannelId(channel_id),
            rate_adjuster,
        }
    }
}

/// Join or part from the configured voice channel.
/// If the bot is the only user in the channel it will leave. If there's a
/// non-bot user in the channel it'll join.
///
/// Returns true if it created a new connection, false otherwise.
fn manage_voice_channel(context: &Context) -> bool {
    let manager_lock = context
        .data
        .read()
        .get::<VoiceManager>()
        .cloned()
        .expect("Expected VoiceManager in TypeMap");
    let mut manager = manager_lock.lock();

    let btfm_data_lock = context
        .data
        .read()
        .get::<BtfmData>()
        .cloned()
        .expect("Expected BtfmData in TypeMap");
    let btfm_data = btfm_data_lock.lock();

    if let Ok(channel) = context.http.get_channel(*btfm_data.channel_id.as_u64()) {
        match channel.guild() {
            Some(guild_channel_lock) => {
                let guild_channel = guild_channel_lock.read();
                if let Ok(members) = guild_channel.members(&context.cache) {
                    if members.iter().find(|m| !m.user.read().bot).is_none() {
                        manager.remove(btfm_data.guild_id);
                    } else if manager.get(&btfm_data.guild_id).is_none() {
                        if let Some(handler) =
                            manager.join(&btfm_data.guild_id, &btfm_data.channel_id)
                        {
                            handler.listen(Some(Box::new(Receiver::new(context.data.clone()))));
                            handler.play(hello_there(&btfm_data));
                            return true;
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
///
/// If the data directory doesn't contain a custom greeting file, the sound of
/// silence is returned. This is important because Discord doesn't seem
/// to send audio until the bot plays something.
fn hello_there(btfm_data: &BtfmData) -> Box<dyn voice::AudioSource> {
    let hello = btfm_data.data_dir.join("hello");
    if let Err(metadata) = hello.metadata() {
        info!(
            "Playing silence instead of a custom greating: {:?}",
            metadata
        );
        let sound_of_silence = Cursor::new(&[0xF8, 0xFF, 0xFE]);
        voice::opus(true, sound_of_silence)
    } else {
        voice::ffmpeg(hello).unwrap()
    }
}

pub struct Handler;
impl EventHandler for Handler {
    fn ready(&self, context: Context, ready: Ready) {
        info!("Connected as {}", ready.user.name);
        manage_voice_channel(&context);
    }

    fn voice_state_update(
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
        if manage_voice_channel(&context) {
            return;
        }

        let manager_lock = context
            .data
            .read()
            .get::<VoiceManager>()
            .cloned()
            .expect("Expected VoiceManager in TypeMap");
        let mut manager = manager_lock.lock();

        let btfm_data_lock = context
            .data
            .read()
            .get::<BtfmData>()
            .cloned()
            .expect("Expected BtfmData in TypeMap");
        let btfm_data = btfm_data_lock.lock();

        if let Some(handler) = manager.get_mut(btfm_data.guild_id) {
            if Some(btfm_data.channel_id) == new.channel_id {
                debug!("User just joined our channel");
                handler.play(hello_there(&btfm_data));
            }
        }
    }
}

#[derive(Eq)]
struct VoicePacket {
    timestamp: u32,
    stereo: bool,
    data: Vec<i16>,
}

impl VoicePacket {
    fn new(timestamp: u32, stereo: bool, data: &[i16]) -> VoicePacket {
        let mut _data: Vec<i16> = Vec::new();
        _data.extend_from_slice(data);
        VoicePacket {
            timestamp,
            stereo,
            data: _data,
        }
    }
}

impl PartialOrd for VoicePacket {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.timestamp.partial_cmp(&other.timestamp)
    }
}

impl PartialEq for VoicePacket {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp == other.timestamp
    }
}

struct User {
    packet_buffer: mpsc::Sender<VoicePacket>,
}

impl User {
    pub fn new(voice_processor: mpsc::Sender<Vec<i16>>) -> User {
        let (sender, receiver) = mpsc::channel::<VoicePacket>();
        // Spawn a thread to buffer audio for the user and pre-process it for recognition
        thread::Builder::new()
            .name("user_voice_buffer".to_string())
            .spawn(move || {
                let timeout = Duration::from_secs(1);
                'outer: loop {
                    let mut voice_packets = Vec::<VoicePacket>::new();
                    loop {
                        match receiver.recv_timeout(timeout) {
                            Ok(packet) => voice_packets.push(packet),
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                if voice_packets.is_empty() {
                                    continue;
                                }
                                let audio_buffer = packets_to_wav(voice_packets);
                                let audio_buffer = interpolate(audio_buffer);
                                match voice_processor.send(audio_buffer) {
                                    Ok(_) => {}
                                    Err(e) => {
                                        error!("Failed to send to voice thread {:?}", e);
                                        break 'outer;
                                    }
                                }
                                continue 'outer;
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => {
                                info!("User's voice buffer thread got a disconnect and is shutting down");
                                break 'outer;
                            }
                        }
                    }
                }
            })
            .unwrap();

        User {
            packet_buffer: sender,
        }
    }
}

pub struct Receiver {
    voice_sender: mpsc::Sender<Vec<i16>>,
    users: HashMap<u32, User>,
    ssrc_map: HashMap<u64, u32>,
}

impl Receiver {
    pub fn new(client_data: Arc<RwLock<TypeMap>>) -> Receiver {
        let (voice_sender, voice_receiver) = mpsc::channel::<Vec<i16>>();
        voice_recognition(voice_receiver, client_data);
        Receiver {
            voice_sender,
            users: HashMap::new(),
            ssrc_map: HashMap::new(),
        }
    }
}

impl voice::AudioReceiver for Receiver {
    fn speaking_update(&mut self, ssrc: u32, user_id: u64, speaking: bool) {
        debug!("Got speaking update ({}) for {}", speaking, user_id);
        self.ssrc_map.entry(user_id).or_insert(ssrc);
    }

    fn client_connect(&mut self, ssrc: u32, user_id: u64) {
        debug!("New user ({}) connected", user_id);
        self.ssrc_map.entry(user_id).or_insert(ssrc);
    }

    fn client_disconnect(&mut self, user_id: u64) {
        debug!("User ({}) disconnected", user_id);
        if let Some(ssrc) = self.ssrc_map.remove(&user_id) {
            self.users.remove(&ssrc);
            debug!("Dropped voice buffer for {}", user_id);
        }
    }

    fn voice_packet(
        &mut self,
        _ssrc: u32,
        _sequence: u16,
        _timestamp: u32,
        _stereo: bool,
        _data: &[i16],
        _compressed_size: usize,
    ) {
        trace!(
            "Received voice packet from ssrc {}, sequence {}, timestamp {}",
            _ssrc,
            _sequence,
            _timestamp
        );
        let voice_sender = &mut self.voice_sender;
        let users = &mut self.users;
        let user = users
            .entry(_ssrc)
            .or_insert_with(|| User::new(voice_sender.clone()));
        match user
            .packet_buffer
            .send(VoicePacket::new(_timestamp, _stereo, _data))
        {
            Ok(_) => {}
            Err(e) => {
                error!("Error pushing audio to user buffer: {:?}", e);
                self.users.remove(&_ssrc).unwrap();
            }
        }
    }
}

fn voice_recognition(voice_rx: mpsc::Receiver<Vec<i16>>, client_data: Arc<RwLock<TypeMap>>) {
    // TODO return the thread reference so the it can be associated with the client and cleaned up
    thread::Builder::new()
        .name("voice_recognition".to_string())
        .spawn(move || {
            let btfm_data_lock = client_data
                .read()
                .get::<BtfmData>()
                .cloned()
                .expect("Expected BtfmData in TypeMap");
            let btfm_data = btfm_data_lock.lock();
            // TODO use optional scorer for improved accuracy
            let mut deepspeech_model = Model::load_from_files(&btfm_data.deepspeech_model)
                .expect("Unable to load deepspeech model");
            if let Some(scorer) = &btfm_data.deepspeech_external_scorer {
                deepspeech_model.enable_external_scorer(scorer)
            }
            drop(btfm_data);
            drop(btfm_data_lock);
            info!("Successfully voice recognition model");
            loop {
                match voice_rx.recv() {
                    Ok(audio_buffer) => {
                        let result = deepspeech_model.speech_to_text(&audio_buffer).unwrap();
                        info!("STT thinks someone said \"{}\"", result);
                        play_clip(Arc::clone(&client_data), &result);
                    }
                    Err(mpsc::RecvError) => {
                        info!("Voice recognition thread channel closed; shutting down thread");
                        break;
                    }
                }
            }
        })
        .unwrap();
}

/// Converts voice data to a wav and returns it as an in-memory file-like object.
///
/// This expects that the voice packets are all stereo, 16 bits per sample, and at
/// sampled at 48kHz. This is what Discord has documented it uses.
fn packets_to_wav(mut voice_packets: Vec<VoicePacket>) -> Cursor<Vec<u8>> {
    assert!(voice_packets.iter().all(|p| p.stereo));
    voice_packets.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Less));
    let voice_data: Vec<i16> = voice_packets
        .into_iter()
        .map(|p| p.data)
        .flatten()
        .collect();
    let data = Vec::<u8>::new();
    let mut cursor = Cursor::new(data);

    // deepspeech-rs wants mono audio, but Discord sends stereo. In my
    // incredibly in-depth and scientic research, both channels are
    // identical so just throw out one channel.
    let wavspec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::new(&mut cursor, wavspec).unwrap();
    let mut i16_writer = writer.get_i16_writer((voice_data.len() / 2) as u32);
    for sample in voice_data.into_iter().step_by(2) {
        i16_writer.write_sample(sample);
    }
    i16_writer.flush().unwrap();
    drop(writer);
    cursor.seek(SeekFrom::Start(0)).unwrap();
    cursor
}

/// Interpolate the wav to the sample rate used by the deepspeech model.
fn interpolate<F>(wav: F) -> Vec<i16>
where
    F: std::io::Read,
    F: std::io::Seek,
{
    let mut reader = Reader::new(wav).unwrap();
    let description = reader.description();

    let audio_buffer: Vec<_> = if description.sample_rate() == SAMPLE_RATE {
        reader.samples().map(|s| s.unwrap()).collect()
    } else {
        let interpolator = Linear::new([0i16], [0]);
        let conv = Converter::from_hz_to_hz(
            from_iter(reader.samples::<i16>().map(|s| [s.unwrap()])),
            interpolator,
            description.sample_rate() as f64,
            SAMPLE_RATE as f64,
        );
        conv.until_exhausted().map(|v| v[0]).collect()
    };
    audio_buffer
}

/// Return true if we should not play a clip (i.e., we are rate limited).
///
/// # Arguments
///
/// `clips` - A list of all clips we know about
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
    clips: &[models::Clip],
    current_time: chrono::NaiveDateTime,
    rate_adjuster: f64,
    rng: &mut dyn rand::RngCore,
) -> bool {
    if let Some(last_play) = clips.iter().map(|c| &c.last_played).max() {
        let since_last_play = current_time - *last_play;
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
    }
    false
}

/// Select an audio clip to play given the phrase detected.
fn play_clip(client_data: Arc<RwLock<TypeMap>>, result: &str) {
    let manager_lock = client_data
        .read()
        .get::<VoiceManager>()
        .cloned()
        .expect("Expected voice manager");

    let btfm_data_lock = client_data
        .read()
        .get::<BtfmData>()
        .cloned()
        .expect("Expected BtfmData in TypeMap");
    let btfm_data = btfm_data_lock.lock();
    let conn = SqliteConnection::establish(btfm_data.data_dir.join(DB_NAME).to_str().unwrap())
        .expect("Unabled to connect to database");
    let clips = schema::clips::table
        .load::<models::Clip>(&conn)
        .expect("Database query failed");
    let mut manager = manager_lock.lock();
    let mut rng = rand::thread_rng();
    if let Some(handler) = manager.get_mut(&btfm_data.guild_id) {
        let current_time = NaiveDateTime::from_timestamp(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Check your system clock")
                .as_secs() as i64,
            0,
        );
        if result.contains("excuse me") {
            info!("Not rate limiting clip since someone was so polite");
        } else if rate_limit(&clips, current_time, btfm_data.rate_adjuster, &mut rng) {
            return;
        }

        let mut potential_clips = Vec::new();
        for clip in clips {
            if result.contains(&clip.phrase) {
                info!("Matched on '{}'", &clip.phrase);
                potential_clips.push(clip);
            }
        }

        if let Some(mut clip) = potential_clips.into_iter().choose(&mut rng) {
            let source = voice::ffmpeg(btfm_data.data_dir.join(&clip.audio_file)).unwrap();
            handler.play(source);
            clip.plays += 1;
            clip.last_played = current_time;
            let filter = schema::clips::table.filter(schema::clips::id.eq(clip.id));
            let update = diesel::update(filter).set(clip).execute(&conn);
            match update {
                Ok(rows_updated) => {
                    if rows_updated != 1 {
                        error!(
                            "Update applied to {} rows which is not expected",
                            rows_updated
                        );
                    } else {
                        debug!("Updated the play count and last_played time successfully");
                    }
                }
                Err(e) => {
                    error!("Updating the clip resulted in {:?}", e);
                }
            }
        }
    } else {
        panic!("Handler missing for guild");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeRng(u32);

    /// This allows our tests to have predictable results, and to have the same predictable results
    /// on both 32-bit and 64-bit architectures. This is used for all tests except for the Gaussian
    /// tests, since those do behave differently between 32-bit and 64-bit systems when using this
    /// rng.
    impl rand::RngCore for FakeRng {
        fn next_u32(&mut self) -> u32 {
            self.0 += 1;
            self.0 - 1
        }

        fn next_u64(&mut self) -> u64 {
            self.next_u32() as u64
        }

        fn fill_bytes(&mut self, dest: &mut [u8]) {
            rand_core::impls::fill_bytes_via_next(self, dest)
        }

        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
            self.fill_bytes(dest);
            Ok(())
        }
    }

    #[test]
    fn test_rate_limit_no_clips() {
        let clips = vec![];
        let current_time = NaiveDateTime::from_timestamp(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Check your system clock")
                .as_secs() as i64,
            0,
        );
        let mut rng = FakeRng(0);

        assert_eq!(rate_limit(&clips, current_time, 256.0, &mut rng), false);
    }

    #[test]
    fn test_packets_to_wav() {
        let packets = vec![
            VoicePacket::new(0, true, &[0, 0]),
            VoicePacket::new(0, true, &[1, 1]),
        ];
        let wav = packets_to_wav(packets);

        let reader = Reader::new(wav).unwrap();
        let description = reader.description();
        assert_eq!(description.channel_count(), 1);
        assert_eq!(description.sample_rate(), 48_000);
        assert_eq!(description.format(), audrey::Format::Wav);
    }

    #[test]
    fn test_packets_sorted() {
        let packets = vec![
            VoicePacket::new(1, true, &[1, 1]),
            VoicePacket::new(0, true, &[0, 0]),
        ];
        let wav = packets_to_wav(packets);

        let mut reader = Reader::new(wav).unwrap();
        assert_eq!(
            reader.samples().map(|s| s.unwrap()).collect::<Vec<i16>>(),
            vec![0, 1]
        );
    }
}
