//! Voice and video call layer: opens direct QUIC connections to peers and
//! streams Opus audio (datagrams) or JPEG video (unidirectional streams).
//!
//! Outgoing calls use [`place_call`]/[`place_video`]; incoming calls are
//! handled by [`handle_incoming`]/[`recv_video`], invoked by the protocol
//! handlers in [`crate::net`].

#[cfg(any(feature = "audio", feature = "video"))]
use crate::event::AppEvent;
#[cfg(any(feature = "audio", feature = "video"))]
use iroh::{Endpoint, EndpointAddr, endpoint::Connection};
#[cfg(feature = "video")]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(any(feature = "audio", feature = "video"))]
use tokio::sync::mpsc;

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
    peer: EndpointAddr,
    mut frame_rx: mpsc::UnboundedReceiver<Vec<u8>>,
) -> anyhow::Result<()> {
    let conn = endpoint.connect(peer, VOICE_ALPN).await?;
    while let Some(frame) = frame_rx.recv().await {
        let _ = conn.send_datagram(frame.into());
    }
    Ok(())
}

/// Handle an incoming voice call: forward datagrams to the UI.
#[cfg(feature = "audio")]
pub async fn handle_incoming(
    conn: Connection,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
) -> anyhow::Result<()> {
    while let Ok(bytes) = conn.read_datagram().await {
        let _ = evt_tx.send(AppEvent::VoiceFrame(bytes.to_vec()));
    }
    Ok(())
}

/// Place an outgoing video call: connect to `peer` and stream JPEG frames
/// over a unidirectional QUIC stream. Each frame is prefixed with a u32
/// length (big-endian).
#[cfg(feature = "video")]
pub async fn place_video(
    endpoint: Endpoint,
    peer: EndpointAddr,
    mut frame_rx: mpsc::UnboundedReceiver<Vec<u8>>,
) -> anyhow::Result<()> {
    let conn = endpoint.connect(peer, VIDEO_ALPN).await?;
    let mut tx = conn.open_uni().await?;
    while let Some(jpeg) = frame_rx.recv().await {
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
    let mut rx = conn.accept_uni().await?;
    loop {
        let len = rx.read_u32().await? as usize;
        let mut buf = vec![0u8; len];
        rx.read_exact(&mut buf).await?;
        let _ = evt_tx.send(AppEvent::VideoFrame(buf));
    }
}
