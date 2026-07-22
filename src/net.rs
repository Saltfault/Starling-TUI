//! Network layer: owns the iroh [`Endpoint`], the gossip subscription, and
//! the voice protocol handler. Bridges the UI ↔ network channels.
//!
//! All gossip text messages are **end-to-end encrypted** with
//! ChaCha20-Poly1305 using a key derived from the room code. Voice calls are
//! E2E encrypted via iroh's QUIC TLS 1.3.

use crate::crypto::FlockCrypto;
use crate::event::{AppEvent, ChatMessage, Command, GossipPayload};
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

/// Derive a deterministic gossip [`TopicId`] from a name via SHA-256.
pub fn topic_for(name: &str) -> TopicId {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(name.as_bytes());
    TopicId::from_bytes(hash.into())
}

/// Derive a short room code from a node ID (first 3 bytes as hex).
pub fn room_code_from_node_id(node_id: &EndpointId) -> String {
    let bytes = node_id.as_bytes();
    let hex: String = (0..3).map(|i| format!("{:02X}", bytes[i])).collect();
    format!("BIRD{hex}")
}

/// Encode a node ID as a shareable ticket string (`BIRD-RRGGBB-...`).
pub fn encode_node_id(node_id: &EndpointId) -> String {
    let bytes = node_id.as_bytes();
    let mut padded = bytes.to_vec();
    while padded.len() % 3 != 0 {
        padded.push(0);
    }
    let colors: Vec<String> = padded
        .chunks(3)
        .map(|c| format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2]))
        .collect();
    format!("BIRD-{}", colors.join("-"))
}

/// Decode a ticket string back into a node ID, or `None` if malformed.
pub fn decode_node_id(code: &str) -> Option<EndpointId> {
    let code = code
        .strip_prefix("BIRD-")
        .or_else(|| code.strip_prefix("BIRD"))?;
    let mut bytes = Vec::new();
    for group in code.split('-') {
        if group.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&group[0..2], 16).ok()?;
        let g = u8::from_str_radix(&group[2..4], 16).ok()?;
        let b = u8::from_str_radix(&group[4..6], 16).ok()?;
        bytes.push(r);
        bytes.push(g);
        bytes.push(b);
    }
    if bytes.len() < 32 {
        return None;
    }
    let arr: [u8; 32] = bytes[..32].try_into().ok()?;
    EndpointId::from_bytes(&arr).ok()
}

/// Encrypt, serialize, and broadcast a gossip payload.
async fn broadcast_payload(
    sender: &iroh_gossip::api::GossipSender,
    crypto: &FlockCrypto,
    payload: &GossipPayload,
) -> anyhow::Result<()> {
    let plaintext = postcard::to_stdvec(payload)?;
    let ciphertext = crypto.encrypt(&plaintext);
    sender.broadcast(ciphertext.into()).await?;
    Ok(())
}

/// Run the network loop: bind an endpoint, join the flock's gossip topic, and
/// shuttle [`Command`]s and [`AppEvent`]s until [`Command::Quit`] arrives.
///
/// * `bootstrap` - known peers to dial when joining an existing flock.
/// * `cmd_rx` - commands from the UI.
/// * `evt_tx` - events back to the UI.
/// * `muted` - shared flag toggled by the UI to mute the mic.
/// * `name` - our display name, announced to peers and used as chat author.
/// * `input_device` - optional CPAL input device name for the mic.
pub async fn run(
    bootstrap: Vec<EndpointId>,
    mut cmd_rx: mpsc::UnboundedReceiver<Command>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    muted: Arc<AtomicBool>,
    name: String,
    input_device: Option<String>,
) -> anyhow::Result<()> {
    let secret = crate::config::Profile::load_or_create_secret();
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .bind()
        .await?;
    endpoint.online().await;

    let my_node_id = endpoint.addr().id;
    let opener_id = bootstrap.first().copied().unwrap_or(my_node_id);
    let room_code = room_code_from_node_id(&opener_id);
    let topic = topic_for(&format!("starling/flock/{room_code}"));
    let crypto = FlockCrypto::from_room_code(&room_code);

    let ticket = encode_node_id(&my_node_id);
    crate::logger::warn(&format!(
        "endpoint bound: room_code={} ticket={}",
        room_code, ticket
    ));
    let _ = evt_tx.send(AppEvent::Ticket(ticket));

    let gossip = Gossip::builder().spawn(endpoint.clone());

    let _router = Router::builder(endpoint.clone())
        .accept(GOSSIP_ALPN, gossip.clone())
        .accept(
            crate::call::VOICE_ALPN,
            VoiceProto {
                evt_tx: evt_tx.clone(),
            },
        )
        .accept(
            crate::call::VIDEO_ALPN,
            VideoProto {
                evt_tx: evt_tx.clone(),
            },
        )
        .spawn();

    let (sender, mut receiver) = gossip.subscribe(topic, bootstrap).await?.split();

    let mut _mic_stream: Option<cpal::Stream> = None;
    let mut _cam_thread: Option<std::thread::JoinHandle<()>> = None;

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
                    broadcast_payload(&sender, &crypto, &GossipPayload::Chat(msg.clone())).await?;
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

                Command::StartVideo(addr) => {
                    let (cam_tx, cam_rx) = mpsc::unbounded_channel();
                    _cam_thread = Some(crate::video::start_camera(cam_tx)?);
                    let ep = endpoint.clone();
                    tokio::spawn(async move {
                        let _ = crate::call::place_video(ep, addr, cam_rx).await;
                    });
                }
                Command::StopVideo => { _cam_thread = None; }

                Command::Quit => break,
            },

            Some(event) = receiver.next() => {
                match event {
                    Ok(Event::Received(msg)) => {
                        if let Some(plaintext) = crypto.decrypt(&msg.content) {
                            match postcard::from_bytes::<GossipPayload>(&plaintext) {
                                Ok(GossipPayload::Chat(m)) => {
                                    let _ = evt_tx.send(AppEvent::Message(m));
                                }
                                Ok(GossipPayload::Profile { id, name }) => {
                                    let _ = evt_tx.send(AppEvent::PeerNamed(id, name));
                                }
                                Err(e) => {
                                    crate::logger::error(&format!("gossip deserialize error: {e}"));
                                }
                            }
                        }
                    }
                    Ok(Event::NeighborUp(id)) => {
                        crate::logger::warn(&format!("neighbor up: {}", id));
                        let _ = evt_tx.send(AppEvent::PeerConnected(id));
                        // Announce our profile to the new peer.
                        let payload = GossipPayload::Profile {
                            id: my_node_id,
                            name: name.clone(),
                        };
                        if let Err(e) = broadcast_payload(&sender, &crypto, &payload).await {
                            crate::logger::error(&format!("profile broadcast failed: {e}"));
                        }
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

/// Protocol handler for incoming voice call connections.
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

/// Protocol handler for incoming video call connections.
#[derive(Debug)]
struct VideoProto {
    evt_tx: mpsc::UnboundedSender<AppEvent>,
}

impl iroh::protocol::ProtocolHandler for VideoProto {
    async fn accept(&self, conn: Connection) -> Result<(), iroh::protocol::AcceptError> {
        let _ = crate::call::recv_video(conn, self.evt_tx.clone()).await;
        Ok(())
    }
}
