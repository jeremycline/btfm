// SPDX-License-Identifier: GPL-2.0-or-later

use futures::StreamExt;
use gstreamer::prelude::Cast;
use gstreamer::prelude::GstBinExtManual;
use gstreamer::traits::{ElementExt, GstBinExt};
use gstreamer::Element;
use tracing::instrument;

/// Convert arbitrary raw audio to the target format required by Whisper
fn whisper_bin() -> gstreamer::Bin {
    let bin = gstreamer::Bin::new(None);

    let parser = gstreamer::ElementFactory::make("rawaudioparse")
        .build()
        .unwrap();
    let audio_resampler = gstreamer::ElementFactory::make("audioresample")
        .build()
        .expect("Install GStreamer plugins");
    let audio_converter = gstreamer::ElementFactory::make("audioconvert")
        .build()
        .expect("Install GStreamer plugins");
    let appsink = gstreamer_app::AppSink::builder().build();
    let caps = gstreamer_audio::AudioInfo::builder(gstreamer_audio::AudioFormat::F32le, 16_000, 1)
        .build()
        .expect("Previously valid audio info is now invalid")
        .to_caps()
        .expect("Previously valid capabilities are now invalid");
    appsink.set_caps(Some(&caps));
    appsink.set_async(false);
    appsink.set_sync(false);

    let target_pad = parser.static_pad("sink").unwrap();
    let bin_pad = gstreamer::GhostPad::with_target(Some("sink"), &target_pad).unwrap();
    bin.add_pad(&bin_pad).unwrap();

    let elements = [
        &parser,
        &audio_resampler,
        &audio_converter,
        appsink.upcast_ref(),
    ];
    bin.add_many(&elements)
        .expect("Failed to add elements to pipeline");
    gstreamer::Element::link_many(&elements).expect("Failed to link pipeline");
    elements
        .into_iter()
        .map(|e| e.sync_state_with_parent().expect("failed to sync state"))
        .for_each(drop);

    bin
}

/// Convert Discord audio to a format we can send to Whisper.
fn discord_to_whisper_pipeline() -> gstreamer::Pipeline {
    let pipeline = gstreamer::Pipeline::new(Some("discord-to-whisper"));

    let appsrc = gstreamer_app::AppSrc::builder()
        .name("whisper-appsrc")
        .build();
    let caps = gstreamer_audio::AudioInfo::builder(gstreamer_audio::AudioFormat::S16le, 48_000, 2)
        .build()
        .expect("Previously valid audio info is now invalid")
        .to_caps()
        .expect("Previously valid capabilities are now invalid");
    appsrc.set_caps(Some(&caps));

    let bin = whisper_bin();
    let elements: [&Element; 2] = [appsrc.upcast_ref(), bin.upcast_ref()];
    pipeline
        .add_many(&elements)
        .expect("Failed to add elements to pipeline");
    gstreamer::Element::link_many(&elements).expect("Failed to link pipeline");
    elements
        .into_iter()
        .map(|e| e.sync_state_with_parent().expect("failed to sync state"))
        .for_each(drop);

    pipeline
}

#[instrument(skip_all)]
pub(crate) async fn discord_to_whisper(data: Vec<u8>) -> anyhow::Result<Vec<f32>> {
    let pipeline = discord_to_whisper_pipeline();
    let mut bus = pipeline
        .bus()
        .ok_or_else(|| anyhow::anyhow!("GStreamer pipeline is missing a bus"))?
        .stream();
    pipeline.set_state(gstreamer::State::Playing)?;

    let appsrc = pipeline
        .by_name("whisper-appsrc")
        .unwrap()
        .downcast::<gstreamer_app::AppSrc>()
        .unwrap();
    let appsink = pipeline
        .by_name("whisper-appsink")
        .unwrap()
        .downcast::<gstreamer_app::AppSink>()
        .unwrap();
    let events = async {
        while let Some(message) = bus.next().await {
            match message.view() {
                gstreamer::MessageView::Eos(_) => {
                    tracing::debug!("End of data stream received");
                    break;
                }
                gstreamer::MessageView::Error(e) => {
                    panic!("Transcoding failed: {e:?}");
                }
                event => tracing::debug!("GStreamer event: {:?}", event),
            }
        }
    };

    let data_reader = async {
        let mut transcoded_data = vec![];
        let mut stream = appsink.stream();
        while let Some(sample) = stream.next().await {
            let buffer_map = sample
                .buffer()
                .and_then(|buf| buf.map_readable().ok())
                .unwrap();
            transcoded_data.extend_from_slice(buffer_map.as_slice());
        }

        let samples = transcoded_data
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("chunks_exact lied to us")))
            .collect::<Vec<_>>();

        samples
    };

    let mut buffer = gstreamer::Buffer::with_size(data.len()).unwrap();
    let mut_buf = buffer.get_mut().unwrap();
    let mut mut_buf = mut_buf.map_writable().unwrap();
    mut_buf.copy_from_slice(&data);
    drop(mut_buf);

    appsrc.push_buffer(buffer).unwrap();
    appsrc.end_of_stream().unwrap();

    let res = data_reader.await;
    events.await;
    pipeline.set_state(gstreamer::State::Null)?;
    Ok(res)
}
