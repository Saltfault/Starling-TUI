//! Audio playback: decodes incoming Opus frames and plays them through a cpal
//! output stream.
//!
//! Uses stereo (2-channel) audio for higher quality voice calls.

use crate::opus_ffi::{Channels, Decoder};
use crate::voice::{CHANNELS, FRAME, SAMPLE_RATE, find_device};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{CachingCons, CachingProd, SharedRb, storage::Heap, traits::*};

/// Ring buffer capacity: ~2 seconds of stereo audio.
const BUFFER_CAPACITY: usize = SAMPLE_RATE as usize * CHANNELS * 2;

type Prod = CachingProd<std::sync::Arc<SharedRb<Heap<f32>>>>;
type Cons = CachingCons<std::sync::Arc<SharedRb<Heap<f32>>>>;

pub struct Playback {
    decoder: Decoder,
    producer: Prod,
    _stream: cpal::Stream,
}

impl Playback {
    pub fn new(device_name: Option<&str>) -> anyhow::Result<Self> {
        starling::util::suppress_stderr(|| Self::new_inner(device_name))
    }

    fn new_inner(device_name: Option<&str>) -> anyhow::Result<Self> {
        let host = cpal::default_host();

        let device = find_device(
            device_name,
            host.output_devices().ok(),
            host.default_output_device(),
        )
        .ok_or_else(|| anyhow::anyhow!("no audio output device found"))?;

        let cfg = cpal::StreamConfig {
            channels: CHANNELS as u16,
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
            |e| starling::logger::error(&format!("playback error: {e}")),
            None,
        )?;

        stream.play()?;
        let decoder = Decoder::new(SAMPLE_RATE, Channels::Stereo)?;

        Ok(Self {
            decoder,
            producer,
            _stream: stream,
        })
    }

    pub fn push_opus(&mut self, bytes: &[u8]) {
        let mut pcm = [0f32; FRAME];
        match self.decoder.decode_float(bytes, &mut pcm, false) {
            // decode_float returns samples per channel; multiply for the total.
            Ok(n) => {
                let total = n * CHANNELS;
                self.producer.push_slice(&pcm[..total]);
            }
            Err(e) => starling::logger::error(&format!("opus decode error: {e}")),
        }
    }
}
