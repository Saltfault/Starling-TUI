//! Microphone capture: reads from the selected input device, encodes 20 ms
//! frames with Opus, and sends the compressed bytes over an mpsc channel.

use crate::opus_ffi::{Application, Channels, Encoder};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

const SAMPLE_RATE: u32 = 48_000;
const FRAME: usize = 960;

pub fn start_capture(
    net_tx: mpsc::UnboundedSender<Vec<u8>>,
    muted: Arc<AtomicBool>,
    device_name: Option<&str>,
) -> anyhow::Result<cpal::Stream> {
    crate::util::suppress_stderr(|| start_capture_inner(net_tx, muted, device_name))
}

fn start_capture_inner(
    net_tx: mpsc::UnboundedSender<Vec<u8>>,
    muted: Arc<AtomicBool>,
    device_name: Option<&str>,
) -> anyhow::Result<cpal::Stream> {
    let host = cpal::default_host();

    let device = if let Some(name) = device_name {
        let mut found = None;
        if let Ok(devices) = host.input_devices() {
            for d in devices {
                let dname = d.to_string();
                if dname == name {
                    found = Some(d);
                    break;
                }
            }
        }
        found
    } else {
        None
    }
    .or_else(|| host.default_input_device())
    .ok_or_else(|| anyhow::anyhow!("no microphone input device found"))?;

    let cfg = cpal::StreamConfig {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        buffer_size: cpal::BufferSize::Default,
    };

    let mut enc = Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Voip)?;
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
        |e| crate::logger::error(&format!("mic error: {e}")),
        None,
    )?;

    stream.play()?;
    Ok(stream)
}
