// SPDX-License-Identifier: GPL-2.0-or-later

use std::io::Cursor;
use std::path::Path;
use std::process::Stdio;

use byteorder::{ByteOrder, LittleEndian};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::{error, info, instrument};

/// Prepare a file for DeepSpeech
#[instrument]
pub async fn file_to_wav(audio: &Path, target_sample: i32) -> Vec<i16> {
    let mut ffmpeg = Command::new("ffmpeg")
        .args([
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
    data
}

#[instrument]
pub fn wrap_pcm(audio: Vec<i16>) -> Vec<u8> {
    let spec = hound::WavSpec {
        bits_per_sample: 16,
        channels: 2,
        sample_format: hound::SampleFormat::Int,
        sample_rate: 48_000,
    };
    let mut cursor = Cursor::new(Vec::<u8>::with_capacity(audio.len() * 2));
    let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
    let mut i16_writer = writer.get_i16_writer(audio.len() as u32);
    for sample in audio.iter() {
        i16_writer.write_sample(*sample);
    }
    i16_writer.flush().unwrap();
    writer.finalize().unwrap();
    cursor.into_inner()
}

/// Converts voice data to the target freqency, mono, and apply some ffmpeg filters.
///
/// This expects that the voice packets are all stereo, 16 bits per sample, and at
/// sampled at 48kHz. This is what Discord has documented it uses.
///
/// Returns: Audio prepped for deepspeech.
#[instrument]
pub async fn discord_to_wav(voice_data: Vec<i16>, target_sample: u32) -> Vec<i16> {
    let data = Vec::<u8>::with_capacity(voice_data.len() * 2);
    let mut cursor = Cursor::new(data);
    // Convert audio to mono, at the sample rate of the deepspeech model, and add a bit
    // of silence to the beginning and end of the audio, which appears to help DeepSpeech
    // not clip the beginning of the transcription
    let mut ffmpeg = Command::new("ffmpeg")
        .args([
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
            "adelay=2s:all=true",
            "-af",
            "apad=pad_dur=2",
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
    data
}
