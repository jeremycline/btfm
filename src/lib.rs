use std::cmp::Ordering;
use std::env;
use std::io::Cursor;
use std::io::{Seek, SeekFrom};
use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use audrey::hound;
use audrey::read::Reader;
use audrey::sample::interpolate::{Converter, Linear};
use audrey::sample::signal::{from_iter, Signal};
use deepspeech::Model;
use serenity::{
    client::{bridge::voice::ClientVoiceManager, Context, EventHandler},
    model::gateway::Ready,
    prelude::*,
    voice,
};

const SAMPLE_RATE: u32 = 16_000;
// These goes away in the next deepspeech-rs release
const BEAM_WIDTH: u16 = 500;
const LM_WEIGHT: f32 = 0.75;
const VALID_WORD_COUNT_WEIGHT: f32 = 1.85;

pub struct VoiceManager;
impl TypeMapKey for VoiceManager {
    type Value = Arc<Mutex<ClientVoiceManager>>;
}

pub struct Handler;
impl EventHandler for Handler {
    fn ready(&self, _: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
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

pub struct Receiver {
    voice_sender: mpsc::Sender<VoicePacket>,
    client_data: Arc<RwLock<ShareMap>>,
}

impl Receiver {
    pub fn new(client_data: Arc<RwLock<ShareMap>>) -> Receiver {
        let (voice_sender, voice_receiver) = mpsc::channel::<VoicePacket>();
        voice_recognition(voice_receiver, client_data.clone());
        Receiver {
            voice_sender,
            client_data,
        }
    }
}

impl voice::AudioReceiver for Receiver {
    fn speaking_update(&mut self, _ssrc: u32, _user_id: u64, _speaking: bool) {
        println!("Got speaking update {}", _speaking);
    }

    fn client_connect(&mut self, _ssrc: u32, _user_id: u64) {
        println!("Hey..... Agent");
    }

    fn client_disconnect(&mut self, _user_id: u64) {
        println!("Got client disconnect");
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
        match self
            .voice_sender
            .send(VoicePacket::new(_timestamp, _stereo, _data))
        {
            Ok(_) => {
                println!("Pushed a voice packet to the voice processing thread");
            }
            Err(e) => {
                println!("Error: {:?}; restarting voice thread", e);
                let (voice_sender, voice_receiver) = mpsc::channel::<VoicePacket>();
                self.voice_sender = voice_sender;
                // TODO hold onto the thread and inspect/kill it here?
                voice_recognition(voice_receiver, self.client_data.clone());
            }
        }
    }
}

fn voice_recognition(voice_rx: mpsc::Receiver<VoicePacket>, client_data: Arc<RwLock<ShareMap>>) {
    // TODO return the thread reference so the it can be associated with the client and cleaned up
    thread::Builder::new()
        .name("voice_recognition".to_string())
        .spawn(move || {
            let model_dir = env::var("DEEPSPEECH_MODEL_DIR").expect("Missing DEEPSPEECH_MODEL_DIR");
            let model_dir = Path::new(&model_dir);
            let mut deepspeech_model =
                Model::load_from_files(&model_dir.join("output_graph.pb"), BEAM_WIDTH)
                    .expect("Unable to load deepspeech model");
            deepspeech_model.enable_decoder_with_lm(
                &model_dir.join("lm.binary"),
                &model_dir.join("trie"),
                LM_WEIGHT,
                VALID_WORD_COUNT_WEIGHT,
            );

            let timeout = Duration::from_secs(1);
            'outer: loop {
                let mut voice_packets = Vec::<VoicePacket>::new();
                loop {
                    match voice_rx.recv_timeout(timeout) {
                        Ok(packet) => {
                            // TODO separate out by ssrc
                            voice_packets.push(packet)
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if voice_packets.is_empty() {
                                continue;
                            }
                            println!("Got voice data in recognition thread");
                            let cursor = voice_as_wav_cursor(voice_packets);
                            let mut reader = Reader::new(cursor).unwrap();
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

                            // let result = deepspeech_model.speech_to_text_with_metadata(&audio_buffer).unwrap();
                            let result = deepspeech_model.speech_to_text(&audio_buffer).unwrap();
                            println!("Heard {}", result);

                            let manager_lock = client_data
                                .read()
                                .get::<VoiceManager>()
                                .cloned()
                                .expect("Expected voice manager");
                            let mut manager = manager_lock.lock();
                            // Being super lazy, there must be an API to figure out the guild and get that data here
                            if let Some(handler) = manager.get_mut(184_441_685_088_927_744) {
                                // TODO look up audio file from the database
                                let data_dir = env::var("BTFM_DATA_DIR").expect(
                                    "The BTFM_DATA_DIR environment variable must be defined.",
                                );
                                let hello_agent = Path::new(&data_dir);
                                let source =
                                    voice::ffmpeg(hello_agent.join(Path::new("hello.wav")))
                                        .unwrap();
                                handler.play(source);
                            } else {
                                panic!("I really should have handled this case");
                            }
                            continue 'outer;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            break 'outer;
                        }
                    }
                }
            }
        })
        .unwrap();
}

fn voice_as_wav_cursor(mut voice_packets: Vec<VoicePacket>) -> Cursor<Vec<u8>> {
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
