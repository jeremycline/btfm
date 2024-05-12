// SPDX-License-Identifier: GPL-2.0-or-later

use anyhow::Context;
use futures::StreamExt;
use gstreamer::glib::object::ObjectExt;
use gstreamer::glib::{prelude::*, RustClosure};
use gstreamer::prelude::{Cast, ElementExt, GstBinExt, GstBinExtManual};
use gstreamer::{Element, Pipeline};
use tokio::sync::mpsc::Receiver;
use tracing::instrument;

/// Convert arbitrary raw audio to the target format required by Whisper
fn whisper_bin() -> anyhow::Result<gstreamer::Bin> {
    let bin = gstreamer::Bin::builder().name("raw-to-whisper").build();

    let queue = gstreamer::ElementFactory::make("queue").build()?;
    let parser = gstreamer::ElementFactory::make("rawaudioparse")
        .build()
        .context("Install the rawaudioparse GStreamer plugin")?;
    let audio_resampler = gstreamer::ElementFactory::make("audioresample")
        .build()
        .context("Install the audioresample GStreamer plugins")?;
    let audio_converter = gstreamer::ElementFactory::make("audioconvert")
        .build()
        .context("Install the audioconvert GStreamer plugins")?;
    let appsink = gstreamer_app::AppSink::builder()
        .name("btfm-appsink")
        .build();
    let caps = gstreamer_audio::AudioInfo::builder(gstreamer_audio::AudioFormat::F32le, 16_000, 1)
        .build()
        .context("Previously valid audio info is now invalid")?
        .to_caps()
        .context("Failed to convert AudioInfo into valid caps")?;
    appsink.set_caps(Some(&caps));
    appsink.set_async(false);
    appsink.set_sync(false);

    let elements = [
        &queue,
        &parser,
        &audio_resampler,
        &audio_converter,
        appsink.upcast_ref(),
    ];
    bin.add_many(elements)
        .context("Failed to add elements to whisper bin")?;

    let target_pad = queue
        .static_pad("sink")
        .expect("The queue GStreamer API changed; no sink pad found.");
    let bin_pad = gstreamer::GhostPad::with_target(&target_pad)
        .context("Unable to link queue pad to the bin ghost pad.")?;
    bin.add_pad(&bin_pad)
        .context("Failed to add sink pad to the bin")?;

    gstreamer::Element::link_many(elements).context("Failed to link whisper bin elements")?;
    elements
        .into_iter()
        .map(|e| e.sync_state_with_parent())
        .collect::<Result<Vec<_>, gstreamer::glib::BoolError>>()?;

    Ok(bin)
}

/// Convert Discord audio to a format we can send to Whisper.
pub(crate) fn discord_to_whisper_pipeline() -> anyhow::Result<gstreamer::Pipeline> {
    let pipeline = gstreamer::Pipeline::builder()
        .name("discord-to-whisper")
        .build();

    let appsrc = gstreamer_app::AppSrc::builder().name("btfm-appsrc").build();
    let caps = gstreamer_audio::AudioInfo::builder(gstreamer_audio::AudioFormat::S16le, 48_000, 2)
        .build()
        .context("Previously valid audio info is now invalid")?
        .to_caps()
        .context("Failed to convert AudioInfo into valid caps")?;
    appsrc.set_caps(Some(&caps));

    let bin = whisper_bin()?;
    let elements: [&Element; 2] = [appsrc.upcast_ref(), bin.upcast_ref()];
    pipeline
        .add_many(elements)
        .expect("Failed to add elements to pipeline");
    gstreamer::Element::link_many(elements).context("Failed to link pipeline")?;
    elements
        .into_iter()
        .map(|e| e.sync_state_with_parent())
        .collect::<Result<Vec<_>, gstreamer::glib::BoolError>>()?;

    Ok(pipeline)
}

pub(crate) fn anything_to_mp3_pipeline() -> anyhow::Result<gstreamer::Pipeline> {
    let pipeline = gstreamer::Pipeline::builder()
        .name("anything-to-mp3")
        .build();
    let appsrc = gstreamer_app::AppSrc::builder().name("btfm-appsrc").build();

    let decodebin = gstreamer::ElementFactory::make("decodebin3").build()?;
    decodebin.connect_closure(
        "select-stream",
        false,
        RustClosure::new(|values| {
            values
                .get(2)
                .map(|val| val.get::<gstreamer::Stream>().ok())
                .map(|stream| {
                    if let Some(stream) = stream {
                        if stream.stream_type() == gstreamer::StreamType::AUDIO {
                            tracing::info!("Found an audio stream");
                            1.to_value()
                        } else {
                            tracing::info!("Found a non-audio stream; ignoring");
                            0.to_value()
                        }
                    } else {
                        tracing::info!("No stream found in callback?");
                        0.to_value()
                    }
                })
        }),
    );
    let weak_pipeline = pipeline.downgrade();

    decodebin.connect_pad_added(move |decodebin_element, _pad| {
        tracing::info!("Connecting to decodebin pad");
        let audioconvert = gstreamer::ElementFactory::make("audioconvert")
            .build()
            .unwrap();
        let lame = gstreamer::ElementFactory::make("lamemp3enc")
            .build()
            .unwrap();
        lame.set_property("bitrate", 128_i32);
        lame.set_property("cbr", true);
        // lame.set_property("target", 1);

        let id3mux = gstreamer::ElementFactory::make("id3mux").build().unwrap();
        let appsink = gstreamer_app::AppSink::builder()
            .name("btfm-appsink")
            .build();

        let elements = [
            &decodebin_element,
            &audioconvert,
            &lame,
            &id3mux,
            appsink.upcast_ref(),
        ];

        if let Some(pipeline) = weak_pipeline.upgrade() {
            pipeline.add_many(elements[1..].iter()).unwrap();
            gstreamer::Element::link_many(elements)
                .context("Failed to link anything-to-mp3 elements")
                .unwrap();
            elements
                .into_iter()
                .map(|e| e.sync_state_with_parent())
                .collect::<Result<Vec<_>, gstreamer::glib::BoolError>>()
                .unwrap();
            pipeline.bus().map(|bus| {
                let structure = gstreamer::structure::Structure::builder("btfm").build();
                let message = gstreamer::message::Application::builder(structure)
                    .other_field("btfm-appsink", appsink)
                    .build();
                tracing::debug!("Posting message to bus for new btfm-appsink");
                bus.post(message)
            });
        }
    });

    let elements = [appsrc.upcast_ref(), &decodebin];
    pipeline
        .add_many(elements)
        .context("Failed to add elements to whisper bin")?;
    gstreamer::Element::link_many(elements).context("Failed to link anything-to-mp3 elements")?;
    elements
        .into_iter()
        .map(|e| e.sync_state_with_parent())
        .collect::<Result<Vec<_>, gstreamer::glib::BoolError>>()?;

    Ok(pipeline)
}

#[instrument(skip_all)]
pub(crate) async fn transcode_to_whisper(
    pipeline: Pipeline,
    mut data: Receiver<bytes::Bytes>,
) -> anyhow::Result<Vec<f32>> {
    let mut bus = pipeline
        .bus()
        .ok_or_else(|| anyhow::anyhow!("GStreamer pipeline is missing a bus"))?
        .stream();
    pipeline.set_state(gstreamer::State::Playing)?;

    let appsrc = pipeline
        .by_name("btfm-appsrc")
        .ok_or_else(|| anyhow::anyhow!("Programmer error: pipline must have btfm-appsrc element"))?
        .downcast::<gstreamer_app::AppSrc>()
        .map_err(|e| {
            anyhow::anyhow!(
                "Programmer error: btfm-appsrc ({:?}) element couldn't be downcast to an appsrc",
                e
            )
        })?;
    let appsink = pipeline
        .by_name("btfm-appsink")
        .ok_or_else(|| anyhow::anyhow!("Programmer error: pipline must have btfm-appsink element"))?
        .downcast::<gstreamer_app::AppSink>()
        .map_err(|e| {
            anyhow::anyhow!(
                "Programmer error: btfm-appsink ({:?}) element couldn't be downcast to an appsrc",
                e
            )
        })?;

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

    let events = tokio::spawn(async move {
        while let Some(message) = bus.next().await {
            match message.view() {
                gstreamer::MessageView::Eos(_) => {
                    tracing::debug!("End of data stream received");
                    break;
                }
                gstreamer::MessageView::Error(e) => {
                    tracing::error!("Transcoding failed: {e:?}");
                    break;
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

#[instrument(skip_all)]
pub(crate) async fn transcode_to_binary(
    pipeline: Pipeline,
    mut data: Receiver<bytes::Bytes>,
) -> anyhow::Result<Vec<u8>> {
    let mut bus = pipeline
        .bus()
        .ok_or_else(|| anyhow::anyhow!("GStreamer pipeline is missing a bus"))?
        .stream();
    pipeline.set_state(gstreamer::State::Playing)?;

    let appsrc = pipeline
        .by_name("btfm-appsrc")
        .ok_or_else(|| anyhow::anyhow!("Programmer error: pipline must have btfm-appsrc element"))?
        .downcast::<gstreamer_app::AppSrc>()
        .map_err(|e| {
            anyhow::anyhow!(
                "Programmer error: btfm-appsrc ({:?}) element couldn't be downcast to an appsrc",
                e
            )
        })?;

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

    let reader_tasks = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let readers = reader_tasks.clone();
    let events = tokio::spawn(async move {
        while let Some(message) = bus.next().await {
            match message.view() {
                gstreamer::MessageView::Eos(_) => {
                    tracing::debug!("End of data stream received");
                    break;
                }
                gstreamer::MessageView::Error(e) => {
                    tracing::error!("Transcoding failed: {e:?}");
                    break;
                }
                gstreamer::MessageView::Application(app) => {
                    tracing::debug!("Application message arrived {:?}", &app);
                    if let Some(structure) = app.structure() {
                        if structure.name().to_string() == "btfm" {
                            tracing::debug!("Attaching data reader to new btfm-appsink");
                            structure
                                .get::<Element>("btfm-appsink")
                                .map(|appsink| {
                                    tracing::info!("Transcoding audio stream");
                                    let appsink =
                                        appsink.downcast::<gstreamer_app::AppSink>().unwrap();
                                    let data_reader = tokio::spawn(async move {
                                        let mut transcoded_data = vec![];
                                        let mut stream = appsink.stream();
                                        while let Some(sample) = stream.next().await {
                                            let buffer_map = sample
                                                .buffer()
                                                .and_then(|buf| buf.map_readable().ok())
                                                .unwrap();
                                            transcoded_data
                                                .extend_from_slice(buffer_map.as_slice());
                                        }

                                        transcoded_data
                                    });
                                    readers.clone().lock().unwrap().push(data_reader);
                                })
                                .unwrap();
                        }
                    }
                }
                event => tracing::debug!("GStreamer event: {:?}", event),
            }
        }
    });
    data_writer.await??;
    events.await?;

    let readers = reader_tasks.lock().unwrap().len();
    let res = if readers == 0 {
        tracing::warn!("No audio streams found!");
        vec![]
    } else {
        if readers > 1 {
            tracing::error!("Multiple audio streams, selecting the first!");
        }
        let reader = reader_tasks.lock().unwrap().remove(0);
        reader.await?
    };
    pipeline.set_state(gstreamer::State::Null)?;
    Ok(res)
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write;

    use super::*;

    const OGG: &[u8] = include_bytes!("../test_data/they-found-me.ogg");

    #[test]
    fn test_whisper_bin() {
        gstreamer::init().unwrap();
        whisper_bin().expect("whisper_bin is unbuildable");
    }

    #[test]
    fn test_discord_to_whisper_pipeline() {
        gstreamer::init().unwrap();
        discord_to_whisper_pipeline().expect("discord-to-whisper pipeline failed");
    }

    #[test]
    fn test_anything_to_mp3_pipeline() {
        gstreamer::init().unwrap();
        anything_to_mp3_pipeline().expect("anything-to-mp3 pipeline failed");
    }

    #[tokio::test]
    async fn test_anything_to_mp3_transcode() {
        tracing_subscriber::fmt::init();
        gstreamer::init().unwrap();
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let pipeline = anything_to_mp3_pipeline().expect("anything-to-mp3 pipeline failed");
        let transcode_task = tokio::spawn(transcode_to_binary(pipeline, receiver));

        let bytes = bytes::Bytes::from_static(OGG);
        sender.send(bytes).await.unwrap();
        drop(sender);

        let data = transcode_task.await.unwrap().unwrap();
        let mut f = File::create("they-found-me.mp3").unwrap();
        f.write_all(&data).unwrap();

        assert_eq!(37819, data.len());
    }

    #[tokio::test]
    async fn test_discord_to_whisper_transcoding() {
        gstreamer::init().unwrap();
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let pipeline = discord_to_whisper_pipeline().unwrap();
        let transcode_task = tokio::spawn(transcode_to_whisper(pipeline, receiver));

        let bytes = bytes::Bytes::from_static(&[0, 0, 0, 0]);
        sender.send(bytes).await.unwrap();
        drop(sender);

        let data = transcode_task.await.unwrap().unwrap();

        assert_eq!(1, data.len());
    }
}
