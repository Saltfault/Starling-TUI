//! Safe wrappers around the opus C API.
//!
//! The FFI bindings are downloaded at build time from the shiguredo/opus-rs
//! pre-built binaries. The `bindings.rs` file contains `#[link_name]`
//! attributes that map standard opus function names to the shiguredo-prefixed
//! symbols in the pre-built static library.

use std::ffi::c_int;
use std::fmt;

#[allow(
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    dead_code,
    unused_imports,
    clippy::all
)]
mod bindings {
    include!(concat!(env!("OUT_DIR"), "/opus_bindings.rs"));
}

use bindings::{
    OpusDecoder, OpusEncoder, opus_decode_float, opus_decoder_create, opus_decoder_destroy,
    opus_encode_float, opus_encoder_create, opus_encoder_destroy,
};

/// Opus encoder. Encodes f32 PCM to compressed Opus bytes.
pub struct Encoder {
    inner: *mut OpusEncoder,
}

impl Encoder {
    /// Create a new encoder for the given sample rate and channel count.
    pub fn new(sample_rate: u32, channels: Channels, _app: Application) -> Result<Self, Error> {
        let mut error: c_int = 0;
        let inner = unsafe {
            opus_encoder_create(
                sample_rate as c_int,
                channels as c_int,
                bindings::OPUS_APPLICATION_VOIP as c_int,
                &mut error,
            )
        };
        if error != 0 || inner.is_null() {
            return Err(Error { code: error });
        }
        Ok(Self { inner })
    }

    /// Encode a frame of f32 PCM samples. Returns the number of encoded bytes.
    pub fn encode_float(&mut self, pcm: &[f32], output: &mut [u8]) -> Result<usize, Error> {
        let n = unsafe {
            opus_encode_float(
                self.inner,
                pcm.as_ptr(),
                (pcm.len() / 1) as c_int,
                output.as_mut_ptr(),
                output.len() as c_int,
            )
        };
        if n < 0 {
            return Err(Error { code: n });
        }
        Ok(n as usize)
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe { opus_encoder_destroy(self.inner) };
    }
}

unsafe impl Send for Encoder {}

/// Opus decoder. Decodes compressed Opus bytes to f32 PCM samples.
pub struct Decoder {
    inner: *mut OpusDecoder,
}

impl Decoder {
    /// Create a new decoder for the given sample rate and channel count.
    pub fn new(sample_rate: u32, channels: Channels) -> Result<Self, Error> {
        let mut error: c_int = 0;
        let inner =
            unsafe { opus_decoder_create(sample_rate as c_int, channels as c_int, &mut error) };
        if error != 0 || inner.is_null() {
            return Err(Error { code: error });
        }
        Ok(Self { inner })
    }

    /// Decode an Opus packet to f32 PCM. Returns the number of decoded
    /// samples per channel.
    pub fn decode_float(
        &mut self,
        data: &[u8],
        output: &mut [f32],
        decode_fec: bool,
    ) -> Result<usize, Error> {
        let n = unsafe {
            opus_decode_float(
                self.inner,
                data.as_ptr(),
                data.len() as c_int,
                output.as_mut_ptr(),
                (output.len() / 1) as c_int,
                if decode_fec { 1 } else { 0 },
            )
        };
        if n < 0 {
            return Err(Error { code: n });
        }
        Ok(n as usize)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { opus_decoder_destroy(self.inner) };
    }
}

unsafe impl Send for Decoder {}

/// Channel count.
#[derive(Clone, Copy)]
pub enum Channels {
    Mono = 1,
    Stereo = 2,
}

/// Application mode (we only use VoIP).
#[derive(Clone, Copy)]
pub enum Application {
    Voip,
}

/// Opus error.
pub struct Error {
    pub code: c_int,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "opus error: code {}", self.code)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Error({})", self.code)
    }
}

impl std::error::Error for Error {}
