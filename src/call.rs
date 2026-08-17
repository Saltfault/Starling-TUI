//! Voice and video call layer: opens direct QUIC connections to peers and
//! streams Opus audio (datagrams) or JPEG video (unidirectional streams).
//!
//! Outgoing calls use [`place_call`]/[`place_video`]; incoming calls are
//! handled by [`handle_incoming`]/[`recv_video`], invoked by the protocol
//! handlers in [`crate::net`].

#[cfg(any(feature = "audio", feature = "video"))]
use crate::event::AppEvent;
#[cfg(any(feature = "audio", feature = "video"))]
use iroh::{Endpoint, EndpointAddr, EndpointId, endpoint::Connection};
#[cfg(feature = "video")]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(any(feature = "audio", feature = "video"))]
use tokio::sync::{broadcast, mpsc};

/// ALPN string for the voice protocol.
#[cfg(feature = "audio")]
pub const VOICE_ALPN: &[u8] = b"starling/voice/0";

/// ALPN string for the video protocol.
#[cfg(feature = "video")]
pub const VIDEO_ALPN: &[u8] = b"starling/video/0";

/// Place an outgoing voice call: connect to `peer` and stream mic frames as
/// QUIC datagrams until the mic channel closes (hang-up).
#[cfg(feature = "audio")]
pub async fn place_call(
    endpoint: Endpoint,
    peer: EndpointId,
    mut frame_rx: broadcast::Receiver<Vec<u8>>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
) -> anyhow::Result<()> {
    let conn = endpoint
        .connect(EndpointAddr::from(peer), VOICE_ALPN)
        .await?;
    // The connection is the join signal: announce the peer immediately so the
    // UI clears its waiting notice even before the first audio datagram.
    let _ = evt_tx.send(AppEvent::CallStarted(peer));
    let mut got_audio = false;
    let connect_deadline = tokio::time::sleep(std::time::Duration::from_secs(10));
    tokio::pin!(connect_deadline);

    loop {
        tokio::select! {
            frame = frame_rx.recv() => {
                let Ok(frame) = frame else { break };
                let _ = conn.send_datagram(frame.into());
            }
            datagram = conn.read_datagram() => {
                let Ok(bytes) = datagram else { break };
                got_audio = true;
                let _ = evt_tx.send(AppEvent::VoiceFrame(bytes.to_vec()));
            }
            _ = &mut connect_deadline, if !got_audio => {
                break;
            }
        }
    }
    let _ = evt_tx.send(AppEvent::CallEnded(peer));
    Ok(())
}

/// Handle an incoming voice call: forward datagrams to the UI.
#[cfg(feature = "audio")]
pub async fn handle_incoming(
    conn: Connection,
    mut frame_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
) -> anyhow::Result<()> {
    let peer = conn.remote_id();

    // The connection is the join signal: announce the peer immediately so the
    // UI clears its waiting notice even before the first audio datagram.
    let _ = evt_tx.send(AppEvent::CallStarted(peer));
    let mut got_audio = false;
    let connect_deadline = tokio::time::sleep(std::time::Duration::from_secs(10));
    tokio::pin!(connect_deadline);

    loop {
        tokio::select! {
            frame = frame_rx.recv() => {
                let Some(frame) = frame else { break };
                let _ = conn.send_datagram(frame.into());
            }
            datagram = conn.read_datagram() => {
                let Ok(bytes) = datagram else { break };
                got_audio = true;
                let _ = evt_tx.send(AppEvent::VoiceFrame(bytes.to_vec()));
            }
            _ = &mut connect_deadline, if !got_audio => {
                break;
            }
        }
    }
    let _ = evt_tx.send(AppEvent::CallEnded(peer));
    Ok(())
}

// ===========================================================================
// Per-peer media session
// ===========================================================================
//
// Keeps a single [`MediaSession`] per active call. Each remote peer gets
// its own Opus decoder so packets from different peers can be decoded
// independently and then mixed into a single output buffer for playback.
// Signalling (invite/accept/leave) uses [`starling::call::SignedCallSignalV1`]
// over the ALPNs; the session itself only owns the decode/mix pipeline and
// the task lifecycle.

#[cfg(feature = "audio")]
pub mod v1 {
    pub use starling::call::{VIDEO_V1_ALPN, VOICE_V1_ALPN};

    #[cfg(test)]
    use crate::opus_ffi::{Channels, Decoder};
    #[cfg(test)]
    use iroh::EndpointId;
    #[cfg(test)]
    use std::collections::HashMap;
    #[cfg(test)]
    use tokio::task::JoinHandle;
    #[cfg(test)]
    use tokio_util::sync::CancellationToken;

    #[cfg(test)]
    pub const SAMPLE_RATE: u32 = 48_000;
    #[cfg(test)]
    pub const FRAME_SIZE: usize = 960;
    #[cfg(test)]
    pub const MONO_FRAME_SAMPLES: usize = FRAME_SIZE;

    #[cfg(test)]
    pub struct MediaSession {
        pub decoders: HashMap<EndpointId, Decoder>,
        pub cancel: CancellationToken,
        pub tasks: Vec<JoinHandle<()>>,
    }

    #[cfg(test)]
    #[derive(Debug)]
    pub struct DecoderError(pub crate::opus_ffi::Error);

    #[cfg(test)]
    impl std::fmt::Display for DecoderError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "opus decoder error: {}", self.0)
        }
    }

    #[cfg(test)]
    impl std::error::Error for DecoderError {}

    #[cfg(test)]
    impl From<crate::opus_ffi::Error> for DecoderError {
        fn from(error: crate::opus_ffi::Error) -> Self {
            Self(error)
        }
    }

    #[cfg(test)]
    impl Default for MediaSession {
        fn default() -> Self {
            Self::new()
        }
    }

    #[cfg(test)]
    impl MediaSession {
        /// Create an empty session with a fresh cancellation token.
        pub fn new() -> Self {
            Self {
                decoders: HashMap::new(),
                cancel: CancellationToken::new(),
                tasks: Vec::new(),
            }
        }

        /// Ensure a decoder exists for `peer`, creating one if needed.
        fn ensure_decoder(&mut self, peer: EndpointId) -> Result<(), DecoderError> {
            use std::collections::hash_map::Entry;
            if let Entry::Vacant(e) = self.decoders.entry(peer) {
                let decoder = Decoder::new(SAMPLE_RATE, Channels::Mono)?;
                e.insert(decoder);
            }
            Ok(())
        }

        /// Decode a single Opus packet from `peer` into `output`.
        ///
        /// `output` must hold at least [`MONO_FRAME_SAMPLES`] samples; the
        /// returned `usize` is the decoded sample count *per channel* as
        /// reported by the decoder. If `decode_fec` is true the decoder will
        /// attempt forward error correction (useful when a packet was lost).
        pub fn decode(
            &mut self,
            peer: EndpointId,
            packet: &[u8],
            output: &mut [f32],
            decode_fec: bool,
        ) -> Result<usize, DecoderError> {
            self.ensure_decoder(peer)?;
            // Take the decoder out of the map for the &mut call, then put it
            // back. The borrow checker can't see through the HashMap lookup
            // while we hold a mutable borrow of `self`.
            let mut decoder = self
                .decoders
                .remove(&peer)
                .expect("decoder was just ensured");
            let result = decoder.decode_float(packet, output, decode_fec);
            self.decoders.insert(peer, decoder);
            result.map_err(DecoderError)
        }

        /// Register a task spawned for this session so it can be awaited on
        /// leave.
        pub fn track(&mut self, handle: JoinHandle<()>) {
            self.tasks.push(handle);
        }

        /// Leave the call: cancel every tracked task and wait for them to
        /// finish. Consumes the session.
        pub async fn leave(self) {
            self.cancel.cancel();
            for task in self.tasks {
                // Errors here mean the task already panicked/cancelled; leave
                // is best-effort teardown so we don't surface them.
                let _ = task.await;
            }
        }
    }

    #[cfg(test)]
    pub fn mix(inputs: &[&[f32]], out: &mut [f32]) {
        for sample in out.iter_mut() {
            *sample = 0.0;
        }
        for input in inputs {
            let n = input.len().min(out.len());
            for (i, sample) in input[..n].iter().enumerate() {
                out[i] += sample;
            }
        }
        for sample in out.iter_mut() {
            *sample = (*sample).clamp(-1.0, 1.0);
        }
    }
}

#[cfg(all(test, feature = "audio"))]
mod v1_tests {
    use super::v1::{MONO_FRAME_SAMPLES, MediaSession, mix};
    use iroh::EndpointId;

    fn peer(byte: u8) -> EndpointId {
        // Derive a stable endpoint id for tests without touching the network.
        let secret = iroh::SecretKey::from_bytes(&[byte; 32]);
        secret.public()
    }

    #[test]
    fn mix_sums_and_clips() {
        let a = vec![0.6_f32; 4];
        let b = vec![0.6_f32; 4];
        let c = vec![-0.9_f32; 4];
        let mut out = [0.0_f32; 4];
        mix(&[&a, &b, &c], &mut out);
        // 0.6 + 0.6 - 0.9 = 0.3 (within range; allow float rounding).
        for s in out {
            assert!((s - 0.3).abs() < 1e-5, "mixed sample {s} should be ~0.3");
        }

        let loud = vec![1.5_f32; 4];
        let mut out = [0.0_f32; 4];
        mix(&[&loud, &loud], &mut out);
        // 1.5 + 1.5 = 3.0 -> clipped to 1.0
        assert_eq!(out, [1.0, 1.0, 1.0, 1.0]);

        let neg = vec![-1.5_f32; 4];
        let mut out = [0.0_f32; 4];
        mix(&[&neg, &neg], &mut out);
        assert_eq!(out, [-1.0, -1.0, -1.0, -1.0]);
    }

    #[test]
    fn mix_zeros_short_inputs() {
        let short = vec![0.25_f32; 2];
        let mut out = [0.0_f32; 4];
        mix(&[&short], &mut out);
        assert_eq!(out, [0.25, 0.25, 0.0, 0.0]);
    }

    #[test]
    fn mix_with_no_inputs_zeros_output() {
        let mut out = [0.5_f32; 4];
        mix(&[], &mut out);
        assert_eq!(out, [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn media_session_starts_with_fresh_token_and_no_tasks() {
        let session = MediaSession::new();
        assert!(session.tasks.is_empty());
        assert!(session.decoders.is_empty());
        assert!(!session.cancel.is_cancelled());
    }

    #[tokio::test]
    async fn media_session_leave_cancels_and_awaits_tasks() {
        let mut session = MediaSession::new();
        let cancel = session.cancel.clone();
        let task = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                cancel.cancelled().await;
            }
        });
        session.track(task);
        assert!(!cancel.is_cancelled());
        session.leave().await;
        assert!(cancel.is_cancelled());
    }

    #[tokio::test]
    async fn media_session_leave_completes_even_if_task_loops() {
        let mut session = MediaSession::new();
        let cancel = session.cancel.clone();
        let task = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                // Loop until cancelled, simulating a long-lived media task.
                loop {
                    if cancel.is_cancelled() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                }
            }
        });
        session.track(task);
        session.leave().await;
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn media_session_decode_creates_decoder_per_peer() {
        let mut session = MediaSession::new();
        let p1 = peer(1);
        let p2 = peer(2);
        let mut out = vec![0.0_f32; MONO_FRAME_SAMPLES];
        // Decoding a bogus packet still inserts a decoder for the peer via
        // `ensure_decoder`; the decode itself errors but the entry remains.
        let _ = session.decode(p1, &[], &mut out, false);
        assert!(session.decoders.contains_key(&p1));
        assert!(!session.decoders.contains_key(&p2));
        let _ = session.decode(p2, &[], &mut out, false);
        assert!(session.decoders.contains_key(&p2));
        assert_eq!(session.decoders.len(), 2);
    }
}

/// Place an outgoing video call: connect to `peer` and stream JPEG frames
/// over a unidirectional QUIC stream. Each frame is prefixed with a u32
/// length (big-endian).
#[cfg(feature = "video")]
pub async fn place_video(
    endpoint: Endpoint,
    peer: EndpointId,
    mut frame_rx: broadcast::Receiver<Vec<u8>>,
) -> anyhow::Result<()> {
    let conn = endpoint
        .connect(EndpointAddr::from(peer), VIDEO_ALPN)
        .await?;
    let mut tx = conn.open_uni().await?;
    loop {
        let jpeg = match frame_rx.recv().await {
            Ok(jpeg) => jpeg,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        };
        tx.write_u32(jpeg.len() as u32).await?;
        tx.write_all(&jpeg).await?;
    }
    Ok(())
}

/// Handle an incoming video call: read JPEG frames from a unidirectional
/// QUIC stream and forward them to the UI.
#[cfg(feature = "video")]
pub async fn recv_video(
    conn: Connection,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
) -> anyhow::Result<()> {
    let peer = conn.remote_id();
    let mut rx = conn.accept_uni().await?;
    let result = async {
        loop {
            let len = rx.read_u32().await? as usize;
            if len > 8 * 1024 * 1024 {
                anyhow::bail!("video frame exceeds 8 MiB limit");
            }
            let mut buf = vec![0u8; len];
            rx.read_exact(&mut buf).await?;
            let _ = evt_tx.send(AppEvent::RemoteVideoFrame { peer, jpeg: buf });
        }
    }
    .await;
    let _ = evt_tx.send(AppEvent::RemoteVideoStopped(peer));
    result
}

#[cfg(test)]
#[cfg(feature = "audio")]
mod voice_tests {
    use super::*;
    use iroh::{
        Endpoint, SecretKey,
        endpoint::{Connection, presets},
        protocol::{AcceptError, ProtocolHandler, Router},
    };

    /// Two endpoints connect via VOICE_ALPN, one sends datagrams (simulating
    /// Opus frames), the other receives them — end-to-end voice flow.
    #[tokio::test]
    async fn voice_datagram_flow() {
        let secret1 = SecretKey::generate();
        let secret2 = SecretKey::generate();
        let ep1 = Endpoint::builder(presets::Minimal)
            .secret_key(secret1)
            .alpns(vec![VOICE_ALPN.to_vec()])
            .ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
            .bind()
            .await
            .unwrap();
        let ep2 = Endpoint::builder(presets::Minimal)
            .secret_key(secret2)
            .alpns(vec![VOICE_ALPN.to_vec()])
            .ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
            .bind()
            .await
            .unwrap();
        let _r1 = Router::builder(ep1.clone())
            .accept(VOICE_ALPN, DummyVoice)
            .spawn();

        let conn = ep2.connect(ep1.addr(), VOICE_ALPN).await.unwrap();

        let frame = vec![1u8, 2, 3, 4];
        conn.send_datagram(frame.clone().into()).unwrap();

        let received = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                match conn.read_datagram().await {
                    Ok(bytes) => return bytes.to_vec(),
                    Err(_) => continue,
                }
            }
        })
        .await
        .expect("datagram not received within 3 seconds");

        assert_eq!(received, frame);
    }

    #[derive(Debug, Clone)]
    struct DummyVoice;

    impl ProtocolHandler for DummyVoice {
        async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
            // Echo back any datagram received.
            while let Ok(datagram) = conn.read_datagram().await {
                let _ = conn.send_datagram(datagram);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
#[cfg(feature = "video")]
mod video_tests {
    use super::*;
    use iroh::{
        Endpoint, SecretKey,
        endpoint::{Connection, presets},
        protocol::{AcceptError, ProtocolHandler, Router},
    };
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Two endpoints connect via VIDEO_ALPN, one sends a JPEG frame over a
    /// unidirectional stream, the other receives it — end-to-end video flow.
    #[tokio::test]
    async fn video_frame_flow() {
        let secret1 = SecretKey::generate();
        let secret2 = SecretKey::generate();
        let ep1 = Endpoint::builder(presets::Minimal)
            .secret_key(secret1)
            .alpns(vec![VIDEO_ALPN.to_vec()])
            .ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
            .bind()
            .await
            .unwrap();
        let ep2 = Endpoint::builder(presets::Minimal)
            .secret_key(secret2)
            .alpns(vec![VIDEO_ALPN.to_vec()])
            .ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
            .bind()
            .await
            .unwrap();

        let frame = b"fake-jpeg-data".to_vec();
        let received: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
        let received_clone = received.clone();

        // Server: accept connection, read the uni stream, store the frame.
        let _r1 = Router::builder(ep1.clone())
            .accept(
                VIDEO_ALPN,
                VideoReceiver {
                    expected: frame.clone(),
                    received: received_clone,
                },
            )
            .spawn();

        // Client: connect and send the frame.
        let conn = ep2.connect(ep1.addr(), VIDEO_ALPN).await.unwrap();
        let mut tx_stream = conn.open_uni().await.unwrap();
        tx_stream.write_u32(frame.len() as u32).await.unwrap();
        tx_stream.write_all(&frame).await.unwrap();
        tx_stream.finish().unwrap();

        // Give the server time to process.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let stored = received
            .lock()
            .unwrap()
            .take()
            .expect("server did not receive frame");
        assert_eq!(stored, frame);
    }

    #[derive(Debug, Clone)]
    struct VideoReceiver {
        expected: Vec<u8>,
        received: Arc<Mutex<Option<Vec<u8>>>>,
    }

    impl ProtocolHandler for VideoReceiver {
        async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
            let mut rx = conn
                .accept_uni()
                .await
                .map_err(|e| AcceptError::from_err(std::io::Error::other(e.to_string())))?;
            let len = rx
                .read_u32()
                .await
                .map_err(|e| AcceptError::from_err(std::io::Error::other(e.to_string())))?
                as usize;
            let mut buf = vec![0u8; len];
            rx.read_exact(&mut buf)
                .await
                .map_err(|e| AcceptError::from_err(std::io::Error::other(e.to_string())))?;
            assert_eq!(buf, self.expected);
            *self.received.lock().unwrap() = Some(buf);
            Ok(())
        }
    }
}
