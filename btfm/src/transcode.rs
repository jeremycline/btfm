// SPDX-License-Identifier: GPL-2.0-or-later

use anyhow::Context;
use futures::StreamExt;
use gstreamer::prelude::Cast;
use gstreamer::prelude::GstBinExtManual;
use gstreamer::traits::{ElementExt, GstBinExt};
use gstreamer::Element;
use tokio::sync::mpsc::Receiver;
use tracing::instrument;

/// Convert arbitrary raw audio to the target format required by Whisper
fn whisper_bin() -> anyhow::Result<gstreamer::Bin> {
    let bin = gstreamer::Bin::new(None);

    let parser = gstreamer::ElementFactory::make("rawaudioparse")
        .build()
        .context("Install the rawaudioparse GStreamer plugin")?;
    let audio_resampler = gstreamer::ElementFactory::make("audioresample")
        .build()
        .context("Install the audioresample GStreamer plugins")?;
    let audio_converter = gstreamer::ElementFactory::make("audioconvert")
        .build()
        .context("Install the audioconvert GStreamer plugins")?;
    let appsink = gstreamer_app::AppSink::builder().build();
    let caps = gstreamer_audio::AudioInfo::builder(gstreamer_audio::AudioFormat::F32le, 16_000, 1)
        .build()
        .context("Previously valid audio info is now invalid")?
        .to_caps()
        .context("Failed to convert AudioInfo into valid caps")?;
    appsink.set_caps(Some(&caps));
    appsink.set_async(false);
    appsink.set_sync(false);

    let target_pad = parser
        .static_pad("sink")
        .expect("The rawaudioparse GStreamer API changed; no sink pad found.");
    let bin_pad = gstreamer::GhostPad::with_target(Some("sink"), &target_pad)
        .context("Unable to link parse pad to the bin ghost pad.")?;
    bin.add_pad(&bin_pad)
        .context("Failed to add sink pad to the bin")?;

    let elements = [
        &parser,
        &audio_resampler,
        &audio_converter,
        appsink.upcast_ref(),
    ];
    bin.add_many(&elements)
        .context("Failed to add elements to whisper bin")?;
    gstreamer::Element::link_many(&elements).context("Failed to link whisper bin elements")?;
    elements
        .into_iter()
        .map(|e| e.sync_state_with_parent())
        .collect::<Result<Vec<_>, gstreamer::glib::BoolError>>()?;

    Ok(bin)
}

/// Convert Discord audio to a format we can send to Whisper.
fn discord_to_whisper_pipeline() -> anyhow::Result<gstreamer::Pipeline> {
    let pipeline = gstreamer::Pipeline::new(Some("discord-to-whisper"));

    let appsrc = gstreamer_app::AppSrc::builder()
        .name("whisper-appsrc")
        .build();
    let caps = gstreamer_audio::AudioInfo::builder(gstreamer_audio::AudioFormat::S16le, 48_000, 2)
        .build()
        .context("Previously valid audio info is now invalid")?
        .to_caps()
        .context("Failed to convert AudioInfo into valid caps")?;
    appsrc.set_caps(Some(&caps));

    let bin = whisper_bin()?;
    let elements: [&Element; 2] = [appsrc.upcast_ref(), bin.upcast_ref()];
    pipeline
        .add_many(&elements)
        .expect("Failed to add elements to pipeline");
    gstreamer::Element::link_many(&elements).context("Failed to link pipeline")?;
    elements
        .into_iter()
        .map(|e| e.sync_state_with_parent())
        .collect::<Result<Vec<_>, gstreamer::glib::BoolError>>()?;

    Ok(pipeline)
}

#[instrument(skip_all)]
pub(crate) async fn discord_to_whisper(
    mut data: Receiver<bytes::Bytes>,
) -> anyhow::Result<Vec<f32>> {
    let pipeline = discord_to_whisper_pipeline()?;
    let mut bus = pipeline
        .bus()
        .ok_or_else(|| anyhow::anyhow!("GStreamer pipeline is missing a bus"))?
        .stream();
    pipeline.set_state(gstreamer::State::Playing)?;

    let appsrc = pipeline
        .by_name("whisper-appsrc")
        .ok_or_else(|| {
            anyhow::anyhow!("Programmer error: pipline must have whisper-appsrc element")
        })?
        .downcast::<gstreamer_app::AppSrc>()
        .map_err(|e| {
            anyhow::anyhow!(
                "Programmer error: whisper-appsrc ({:?}) element couldn't be downcast to an appsrc",
                e
            )
        })?;
    let appsink = pipeline
        .by_name("whisper-appsink")
        .ok_or_else(|| {
            anyhow::anyhow!("Programmer error: pipline must have whisper-appsink element")
        })?
        .downcast::<gstreamer_app::AppSink>()
        .map_err(|e| {
            anyhow::anyhow!(
                "Programmer error: whisper-appsink ({:?}) element couldn't be downcast to an appsrc",
                e
            )
        })?;

    let data_reader = tokio::spawn(async move {
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
    });

    let data_writer: tokio::task::JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
        while let Some(bytes) = data.recv().await {
            let mut buffer = gstreamer::Buffer::with_size(bytes.len())?;
            let mut_buf = buffer
                .get_mut()
                .ok_or_else(|| anyhow::anyhow!("Gstreamer buffer is not mutable"))?;
            let mut mut_buf = mut_buf.map_writable()?;
            mut_buf.copy_from_slice(&bytes);
            drop(mut_buf);
            appsrc.push_buffer(buffer)?;
        }
        appsrc.end_of_stream()?;
        Ok(())
    });

    let events = tokio::spawn(async move {
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
    });
    data_writer.await??;
    let res = data_reader.await?;
    events.await?;
    pipeline.set_state(gstreamer::State::Null)?;
    Ok(res)
}
