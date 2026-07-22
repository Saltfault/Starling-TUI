//! Network layer: owns the iroh [`Endpoint`], the gossip subscription, and
//! the voice protocol handler. Bridges the UI ↔ network channels.
//!
//! All gossip text messages are **end-to-end encrypted** with
//! ChaCha20-Poly1305 using a key derived from the room code. Voice calls are
//! E2E encrypted via iroh's QUIC TLS 1.3.

use crate::crypto::FlockCrypto;
use crate::event::{AppEvent, ChatMessage, Command};
use iroh::{
    Endpoint, EndpointId,
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
/// it with SHA-256.
pub fn topic_for(name: &str) -> TopicId {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(name.as_bytes());
    TopicId::from_bytes(hash.into())
}

/// Derive a short room code from a node ID: "BIRD" + first 6 hex chars.
pub fn room_code_from_node_id(node_id: &EndpointId) -> String {
    let bytes = node_id.as_bytes();
    let hex: String = (0..3).map(|i| format!("{:02X}", bytes[i])).collect();
    format!("BIRD{hex}")
}

/// The main network loop. Spawned once by `main`.
pub async fn run(
    bootstrap: Vec<EndpointId>,
    mut cmd_rx: mpsc::UnboundedReceiver<Command>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    muted: Arc<AtomicBool>,
    name: String,
    input_device: Option<String>,
) -> anyhow::Result<()> {
    crate::logger::warn("binding endpoint...");

    let endpoint = Endpoint::bind(presets::N0).await?;
    endpoint.online().await;

    let my_node_id = endpoint.addr().id;
    let opener_id = bootstrap.first().copied().unwrap_or(my_node_id);
    let room_code = room_code_from_node_id(&opener_id);
    let topic = topic_for(&format!("starling/flock/{room_code}"));
    let crypto = FlockCrypto::from_room_code(&room_code);

    crate::logger::warn(&format!(
        "endpoint bound: node_id={} room_code={}",
        my_node_id, room_code
    ));

    // Send our node ID to the UI (this is the invite ticket for openers).
    let _ = evt_tx.send(AppEvent::Ticket(my_node_id.to_string()));

    let gossip = Gossip::builder().spawn(endpoint.clone());

    let _router = Router::builder(endpoint.clone())
        .accept(GOSSIP_ALPN, gossip.clone())
        .accept(
            crate::call::VOICE_ALPN,
            VoiceProto {
                evt_tx: evt_tx.clone(),
            },
        )
        .spawn();

    // Subscribe to the gossip topic. Use `subscribe` (not `subscribe_and_join`)
    // so it returns immediately. Bootstrap peers are connected in the
    // background; NeighborUp events fire when connections establish.
    let (sender, mut receiver) = gossip.subscribe(topic, bootstrap).await?.split();

    crate::logger::warn("subscribed to gossip topic");

    let mut _mic_stream: Option<cpal::Stream> = None;

    loop {
        tokio::select! {
            Some(cmd) = cmd_rx.recv() => match cmd {
                Command::SendText(text) => {
                    let msg = ChatMessage {
                        id: uuid::Uuid::new_v4().to_string(),
                        author: name.clone(),
                        body: text,
                        ts: chrono::Utc::now().timestamp_millis(),
                    };
                    let plaintext = postcard::to_stdvec(&msg)?;
                    let ciphertext = crypto.encrypt(&plaintext);
                    sender.broadcast(ciphertext.into()).await?;
                    let _ = evt_tx.send(AppEvent::Message(msg));
                }

                Command::StartCall(addr) => {
                    let (mic_tx, mic_rx) = mpsc::unbounded_channel();
                    _mic_stream = Some(crate::voice::start_capture(
                        mic_tx, muted.clone(), input_device.as_deref(),
                    )?);
                    let ep = endpoint.clone();
                    tokio::spawn(async move {
                        let _ = crate::call::place_call(ep, addr, mic_rx).await;
                    });
                }

                Command::HangUp => { _mic_stream = None; }

                Command::Quit => break,
            },

            Some(event) = receiver.next() => {
                match event {
                    Ok(Event::Received(msg)) => {
                        if let Some(plaintext) = crypto.decrypt(&msg.content) {
                            if let Ok(m) = postcard::from_bytes::<ChatMessage>(&plaintext) {
                                let _ = evt_tx.send(AppEvent::Message(m));
                            }
                        }
                    }
                    Ok(Event::NeighborUp(id)) => {
                        crate::logger::warn(&format!("neighbor up: {}", id));
                        let _ = evt_tx.send(AppEvent::PeerConnected(id));
                    }
                    Ok(Event::NeighborDown(id)) => {
                        crate::logger::warn(&format!("neighbor down: {}", id));
                        let _ = evt_tx.send(AppEvent::PeerDisconnected(id));
                    }
                    Ok(_) => {}
                    Err(e) => {
                        crate::logger::error(&format!("gossip stream error: {e}"));
                    }
                }
            }
        }
    }

    Ok(())
}

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
