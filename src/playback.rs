//! Audio playback: decodes incoming Opus frames and plays them through a cpal
//! output stream.
//!
//! A lock-free ring buffer bridges the decode thread (producer) and the audio
//! output thread (consumer). If the buffer is empty (underrun), silence is
//! played.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use opus::{Channels, Decoder};
use ringbuf::{CachingCons, CachingProd, SharedRb, storage::Heap, traits::*};

const SAMPLE_RATE: u32 = 48_000;
const FRAME: usize = 960;
const BUFFER_CAPACITY: usize = SAMPLE_RATE as usize * 2;

type Prod = CachingProd<std::sync::Arc<SharedRb<Heap<f32>>>>;
type Cons = CachingCons<std::sync::Arc<SharedRb<Heap<f32>>>>;

pub struct Playback {
    decoder: Decoder,
    producer: Prod,
    _stream: cpal::Stream,
}

impl Playback {
    pub fn new(device_name: Option<&str>) -> anyhow::Result<Self> {
        crate::util::suppress_stderr(|| Self::new_inner(device_name))
    }

    fn new_inner(device_name: Option<&str>) -> anyhow::Result<Self> {
        let host = cpal::default_host();

        let device = if let Some(name) = device_name {
            let mut found = None;
            if let Ok(devices) = host.output_devices() {
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
        .or_else(|| host.default_output_device())
        .ok_or_else(|| anyhow::anyhow!("no audio output device found"))?;

        let cfg = cpal::StreamConfig {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            buffer_size: cpal::BufferSize::Default,
        };

        let rb = SharedRb::<Heap<f32>>::new(BUFFER_CAPACITY);
        let (producer, mut consumer): (Prod, Cons) = rb.split();

        let stream = device.build_output_stream(
            cfg,
            move |data: &mut [f32], _: &_| {
                let n = consumer.pop_slice(data);
                for sample in &mut data[n..] {
                    *sample = 0.0;
                }
            },
            |e| crate::logger::error(&format!("playback error: {e}")),
            None,
        )?;

        stream.play()?;
        let decoder = Decoder::new(SAMPLE_RATE, Channels::Mono)?;

        Ok(Self {
            decoder,
            producer,
            _stream: stream,
        })
    }

    pub fn push_opus(&mut self, bytes: &[u8]) {
        let mut pcm = [0f32; FRAME];
        match self.decoder.decode_float(bytes, &mut pcm, false) {
            Ok(n) => {
                self.producer.push_slice(&pcm[..n]);
            }
            Err(e) => crate::logger::error(&format!("opus decode error: {e}")),
        }
    }
}
