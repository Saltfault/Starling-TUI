//! Microphone capture: reads from the selected input device, encodes 20 ms
//! frames with Opus, and sends the compressed bytes over an mpsc channel.
//!
//! Uses stereo (2-channel) audio for higher quality voice calls.

use crate::opus_ffi::{Channels, Encoder};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

pub(crate) const SAMPLE_RATE: u32 = 48_000;
pub(crate) const CHANNELS: usize = 2;
/// Samples per channel per 20 ms frame at 48 kHz.
pub(crate) const FRAME_SAMPLES: usize = 960;
pub(crate) const FRAME: usize = FRAME_SAMPLES * CHANNELS;

pub fn start_capture(
    net_tx: mpsc::UnboundedSender<Vec<u8>>,
    muted: Arc<AtomicBool>,
    device_name: Option<&str>,
) -> anyhow::Result<cpal::Stream> {
    starling::util::suppress_stderr(|| start_capture_inner(net_tx, muted, device_name))
}

fn start_capture_inner(
    net_tx: mpsc::UnboundedSender<Vec<u8>>,
    muted: Arc<AtomicBool>,
    device_name: Option<&str>,
) -> anyhow::Result<cpal::Stream> {
    let host = cpal::default_host();

    let device = find_device(
        device_name,
        host.input_devices().ok(),
        host.default_input_device(),
    )
    .ok_or_else(|| anyhow::anyhow!("no microphone input device found"))?;

    let cfg = cpal::StreamConfig {
        channels: CHANNELS as u16,
        sample_rate: SAMPLE_RATE,
        buffer_size: cpal::BufferSize::Default,
    };

    let mut enc = Encoder::new(SAMPLE_RATE, Channels::Stereo)?;
    let mut acc: Vec<f32> = Vec::with_capacity(FRAME);

    let stream = device.build_input_stream(
        cfg,
        move |data: &[f32], _: &_| {
            acc.extend_from_slice(data);
            while acc.len() >= FRAME {
                let frame: Vec<f32> = acc.drain(..FRAME).collect();
                if muted.load(Ordering::Relaxed) {
                    continue;
                }
                let mut out = vec![0u8; 400];
                if let Ok(n) = enc.encode_float(&frame, &mut out) {
                    out.truncate(n);
                    let _ = net_tx.send(out);
                }
            }
        },
        |e| starling::logger::error(&format!("mic error: {e}")),
        None,
    )?;

    stream.play()?;
    Ok(stream)
}

/// Find an audio device by name, falling back to the host default when the
/// name is `None` or no listed device matches.
pub(crate) fn find_device<I: Iterator<Item = cpal::Device>>(
    name: Option<&str>,
    devices: Option<I>,
    default: Option<cpal::Device>,
) -> Option<cpal::Device> {
    name.and_then(|target| {
        devices.and_then(|mut iter| iter.find(|d| d.to_string() == target))
    })
    .or(default)
}
