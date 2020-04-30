// SPDX-License-Identifier: GPL-2.0-or-later

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Cursor;
use std::io::{Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use audrey::hound;
use audrey::read::Reader;
use audrey::sample::interpolate::{Converter, Linear};
use audrey::sample::signal::{from_iter, Signal};
use deepspeech::Model;
use log::{debug, error, info, trace};
use serenity::{
    client::{bridge::voice::ClientVoiceManager, Context, EventHandler},
    model::{
        channel::GuildChannel,
        gateway::Ready,
        id::{ChannelId, GuildId},
        voice::VoiceState,
    },
    prelude::*,
    voice,
};

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
    guild_id: GuildId,
    channel_id: ChannelId,
}
impl TypeMapKey for BtfmData {
    type Value = Arc<Mutex<BtfmData>>;
}
impl BtfmData {
    pub fn new(
        data_dir: PathBuf,
        deepspeech_model: PathBuf,
        guild_id: u64,
        channel_id: u64,
    ) -> BtfmData {
        BtfmData {
            data_dir,
            deepspeech_model,
            guild_id: GuildId(guild_id),
            channel_id: ChannelId(channel_id),
        }
    }
}

pub struct Handler;
impl EventHandler for Handler {
    fn ready(&self, _: Context, ready: Ready) {
        info!("Connected as {}", ready.user.name);
    }

    fn voice_state_update(
        &self,
        context: Context,
        guild_id: Option<GuildId>,
        old: Option<VoiceState>,
        new: VoiceState,
    ) {
        let guild_id = match guild_id {
            Some(guild_id) => guild_id,
            None => return,
        };

        debug!("voice_state_update: old={:?}  new={:?}", old, new);

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

        if let Ok(channels) = context.http.get_channels(*btfm_data.guild_id.as_u64()) {
            let mut channel = channels
                .iter()
                .filter(|chan| chan.id == btfm_data.channel_id)
                .collect::<Vec<&GuildChannel>>();
            if let Some(channel) = channel.pop() {
                if let Ok(members) = channel.members(context.cache) {
                    if members.iter().find(|m| !m.user.read().bot).is_none() {
                        manager.remove(btfm_data.guild_id);
                    }
                }
            } else {
                return;
            }
        } else {
            return;
        }

        if let Some(channel_id) = new.channel_id {
            if btfm_data.channel_id != channel_id {
                debug!(
                    "Ignoring user joining {:?}, looking for {:?}",
                    channel_id, btfm_data.channel_id
                );
                return;
            }

            if let Some(handler) = manager.join(guild_id, channel_id) {
                handler.listen(Some(Box::new(Receiver::new(context.data))));

                // TODO make this less bad
                let hello = btfm_data.data_dir.join("hello");
                let source = voice::ffmpeg(hello).unwrap();
                handler.play(source);
            } else {
                error!("Unable to join channel");
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
    // Doesn't seem possible to see if the thread is joinable or not so this may not be useful
    buffer_thread: thread::JoinHandle<()>,
    packet_buffer: mpsc::Sender<VoicePacket>,
}

impl User {
    pub fn new(voice_processor: mpsc::Sender<Vec<i16>>) -> User {
        let (sender, receiver) = mpsc::channel::<VoicePacket>();
        // Spawn a thread to buffer audio for the user and pre-process it for recognition
        let t = thread::Builder::new()
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
                                error!("Got disconnect from voice handler");
                                break 'outer;
                            }
                        }
                    }
                }
            })
            .unwrap();

        User {
            buffer_thread: t,
            packet_buffer: sender,
        }
    }
}

pub struct Receiver {
    voice_sender: mpsc::Sender<Vec<i16>>,
    users: HashMap<u32, User>,
}

impl Receiver {
    pub fn new(client_data: Arc<RwLock<TypeMap>>) -> Receiver {
        let (voice_sender, voice_receiver) = mpsc::channel::<Vec<i16>>();
        voice_recognition(voice_receiver, client_data);
        Receiver {
            voice_sender,
            users: HashMap::new(),
        }
    }
}

impl voice::AudioReceiver for Receiver {
    fn speaking_update(&mut self, _ssrc: u32, _user_id: u64, _speaking: bool) {
        debug!("Got speaking update ({}) for {}", _speaking, _user_id);
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
            let mut deepspeech_model = Model::load_from_files(&btfm_data.deepspeech_model)
                .expect("Unable to load deepspeech model");
            drop(btfm_data);
            drop(btfm_data_lock);
            loop {
                match voice_rx.recv() {
                    Ok(audio_buffer) => {
                        let result = deepspeech_model.speech_to_text(&audio_buffer).unwrap();
                        info!("STT thinks someone said \"{}\"", result);
                        play_clip(Arc::clone(&client_data), &result);
                    }
                    Err(mpsc::RecvError) => {
                        error!("Got disconnect from voice handler");
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

fn play_clip(client_data: Arc<RwLock<TypeMap>>, result: &str) {
    let manager_lock = client_data
        .read()
        .get::<VoiceManager>()
        .cloned()
        .expect("Expected voice manager");

    use crate::models;
    use crate::schema::clips::dsl::*;
    use diesel::prelude::*;

    let btfm_data_lock = client_data
        .read()
        .get::<BtfmData>()
        .cloned()
        .expect("Expected BtfmData in TypeMap");
    let btfm_data = btfm_data_lock.lock();
    let conn = SqliteConnection::establish(btfm_data.data_dir.join(DB_NAME).to_str().unwrap())
        .expect("Unabled to connect to database");
    let _clips = clips
        .load::<models::Clip>(&conn)
        .expect("Database query failed");
    let mut manager = manager_lock.lock();
    if let Some(handler) = manager.get_mut(&btfm_data.guild_id) {
        for clip in _clips {
            if result.contains(&clip.phrase) {
                info!("Matched on '{}'", &clip.phrase);
                let source = voice::ffmpeg(btfm_data.data_dir.join(&clip.audio_file)).unwrap();
                handler.play(source);
            }
        }
    } else {
        panic!("Handler missing for guild");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
