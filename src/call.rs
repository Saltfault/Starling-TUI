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
    mut frame_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
) -> anyhow::Result<()> {
    let conn = endpoint
        .connect(EndpointAddr::from(peer), VOICE_ALPN)
        .await?;
    let _ = evt_tx.send(AppEvent::CallStarted(peer));
    loop {
        tokio::select! {
            frame = frame_rx.recv() => {
                let Some(frame) = frame else { break };
                let _ = conn.send_datagram(frame.into());
            }
            datagram = conn.read_datagram() => {
                let Ok(bytes) = datagram else { break };
                let _ = evt_tx.send(AppEvent::VoiceFrame(bytes.to_vec()));
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
    let _ = evt_tx.send(AppEvent::CallStarted(peer));
    loop {
        tokio::select! {
            frame = frame_rx.recv() => {
                let Some(frame) = frame else { break };
                let _ = conn.send_datagram(frame.into());
            }
            datagram = conn.read_datagram() => {
                let Ok(bytes) = datagram else { break };
                let _ = evt_tx.send(AppEvent::VoiceFrame(bytes.to_vec()));
            }
        }
    }
    let _ = evt_tx.send(AppEvent::CallEnded(peer));
    Ok(())
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
