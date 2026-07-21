//! Network layer: owns the iroh [`Endpoint`], the gossip subscription, and
//! the voice protocol handler. Bridges the UI ↔ network channels.
//!
//! This module is spawned as a single tokio task by `main`. It runs a
//! `tokio::select!` loop that:
//!
//! 1. Receives [`Command`]s from the UI and acts on them (sending text,
//!    starting/hanging up calls, quitting).
//! 2. Receives gossip [`Event`]s and forwards them to the UI as [`AppEvent`]s.
//!
//! The mic capture stream (`_mic_stream`) is kept alive here for the duration
//! of a call. Dropping it stops the mic.

use crate::event::{AppEvent, ChatMessage, Command};
use iroh::{
    Endpoint,
    endpoint::{Connection, presets},
    protocol::Router,
};
use iroh_gossip::{
    api::Event,
    net::{GOSSIP_ALPN, Gossip},
    proto::TopicId,
};
use n0_future::StreamExt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc;

/// Derive a stable 32-byte [`TopicId`] from a human-readable name by hashing
/// it with SHA-256. Everyone who uses the same name gets the same topic.
pub fn topic_for(name: &str) -> TopicId {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(name.as_bytes());
    TopicId::from_bytes(hash.into())
}

/// The main network loop. Spawned once by `main`.
///
/// * `topic` — the gossip topic to subscribe to (derived from the room code).
/// * `cmd_rx` — receives commands from the UI.
/// * `evt_tx` — sends events to the UI.
/// * `muted` — shared mute flag, passed through to the mic capture callback.
/// * `name` — the bird's display name, used as the author on chat messages.
///
/// Both the opener and joiner call this with the same topic (derived from the
/// room code) and an empty bootstrap list. iroh's relay connects them on the
/// topic automatically — no node IDs need to be exchanged.
pub async fn run(
    topic: TopicId,
    mut cmd_rx: mpsc::UnboundedReceiver<Command>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    muted: Arc<AtomicBool>,
    name: String,
) -> anyhow::Result<()> {
    // Bind a QUIC endpoint with n0's relay and discovery presets.
    let endpoint = Endpoint::bind(presets::N0).await?;
    endpoint.online().await; // wait until a home relay is assigned

    // Spawn the gossip sub-protocol.
    let gossip = Gossip::builder().spawn(endpoint.clone());

    // Build the protocol router: accept gossip for chat, and our custom
    // VoiceProto for voice calls.
    let _router = Router::builder(endpoint.clone())
        .accept(GOSSIP_ALPN, gossip.clone())
        .accept(
            crate::call::VOICE_ALPN,
            VoiceProto {
                evt_tx: evt_tx.clone(),
            },
        )
        .spawn();

    // Subscribe to the gossip topic. Both opener and joiner use the same
    // topic (derived from the room code) with empty bootstrap — iroh's
    // relay connects peers on the same topic automatically.
    let (sender, mut receiver) = gossip.subscribe_and_join(topic, vec![]).await?.split();

    // Keep the mic capture stream alive for the duration of a call.
    // Dropping this stops the mic.
    let mut _mic_stream: Option<cpal::Stream> = None;

    loop {
        tokio::select! {
            // ── Commands from the UI ───────────────────────────────────
            Some(cmd) = cmd_rx.recv() => match cmd {
                Command::SendText(text) => {
                    let msg = ChatMessage {
                        id: uuid::Uuid::new_v4().to_string(),
                        author: name.clone(),
                        body: text,
                        ts: chrono::Utc::now().timestamp_millis(),
                    };
                    let bytes = postcard::to_stdvec(&msg)?;
                    sender.broadcast(bytes.into()).await?;
                    let _ = evt_tx.send(AppEvent::Message(msg));
                }

                Command::StartCall(addr) => {
                    let (mic_tx, mic_rx) = mpsc::unbounded_channel();
                    _mic_stream = Some(crate::voice::start_capture(mic_tx, muted.clone())?);
                    let ep = endpoint.clone();
                    tokio::spawn(async move {
                        let _ = crate::call::place_call(ep, addr, mic_rx).await;
                    });
                }

                Command::HangUp => {
                    _mic_stream = None;
                }

                Command::Quit => break,
            },

            // ── Gossip events from the network ────────────────────────
            Some(event) = receiver.next() => {
                match event {
                    Ok(Event::Received(msg)) => {
                        if let Ok(m) = postcard::from_bytes::<ChatMessage>(&msg.content) {
                            let _ = evt_tx.send(AppEvent::Message(m));
                        }
                    }
                    Ok(Event::NeighborUp(id)) => {
                        let _ = evt_tx.send(AppEvent::PeerConnected(id));
                    }
                    Ok(Event::NeighborDown(id)) => {
                        let _ = evt_tx.send(AppEvent::PeerDisconnected(id));
                    }
                    Ok(_) => {}
                    Err(e) => {
                        // Don't kill the network task on a stream error
                        // (e.g. lagged receiver) — just log and continue.
                        eprintln!("gossip stream error: {e}");
                    }
                }
            }
        }
    }

    Ok(())
}

/// Protocol handler for incoming voice calls. When a peer dials our
/// `VOICE_ALPN`, this accepts the connection and forwards voice frames to the
/// UI as [`AppEvent::VoiceFrame`].
#[derive(Debug)]
struct VoiceProto {
    evt_tx: mpsc::UnboundedSender<AppEvent>,
}

impl iroh::protocol::ProtocolHandler for VoiceProto {
    async fn accept(&self, conn: Connection) -> Result<(), iroh::protocol::AcceptError> {
        let _ = crate::call::handle_incoming(conn, self.evt_tx.clone()).await;
        Ok(())
    }
}
