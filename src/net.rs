//! Network layer: owns the iroh [`Endpoint`], the gossip subscription, and
//! the voice protocol handler. Bridges the UI ↔ network channels.
//!
//! All gossip text messages are **end-to-end encrypted** with
//! ChaCha20-Poly1305 using a key derived from the room code. Voice calls are
//! E2E encrypted via iroh's QUIC TLS 1.3.

use crate::crypto::FlockCrypto;
#[cfg(feature = "audio")]
use crate::event::BirdStatus;
use crate::event::{AppEvent, ChatMessage, Command, GossipPayload};
#[cfg(any(feature = "audio", feature = "video"))]
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointId, endpoint::presets, protocol::Router};
use iroh_gossip::{
    api::Event,
    net::{GOSSIP_ALPN, Gossip},
    proto::TopicId,
};
use n0_future::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc;

use iroh_gossip::api::GossipSender;

#[allow(dead_code)]
struct FlockHandle {
    sender: GossipSender,
    crypto: FlockCrypto,
}

/// Derive a deterministic gossip [`TopicId`] from a name via SHA-256.
pub fn topic_for(name: &str) -> TopicId {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(name.as_bytes());
    TopicId::from_bytes(hash.into())
}

/// Encode a node ID as the full room code / invite (`BIRD-RRGGBB-...`).
/// This single string is both what's displayed and what peers join with.
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
#[allow(dead_code)]
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
/// * `my_node_id` - our own node identity (from the persistent secret key).
/// * `name` - our display name, announced to peers and used as chat author.
/// * `input_device` - optional CPAL input device name for the mic.
pub async fn run(
    bootstrap: Vec<EndpointId>,
    mut cmd_rx: mpsc::UnboundedReceiver<Command>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    muted: Arc<AtomicBool>,
    my_node_id: EndpointId,
    name: String,
    input_device: Option<String>,
) -> anyhow::Result<()> {
    let _ = (&muted, &input_device);
    let secret = crate::config::Profile::load_or_create_secret();
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .bind()
        .await?;
    endpoint.online().await;

    let my_code = encode_node_id(&my_node_id);
    crate::logger::warn(&format!("endpoint bound: room_code={my_code}"));
    let _ = evt_tx.send(AppEvent::Ticket(my_code));

    let gossip = Gossip::builder().spawn(endpoint.clone());
    let history: crate::sync::History = Default::default();

    #[allow(unused_mut)]
    let mut builder = Router::builder(endpoint.clone()).accept(GOSSIP_ALPN, gossip.clone());
    #[cfg(feature = "audio")]
    {
        builder = builder.accept(
            crate::call::VOICE_ALPN,
            VoiceProto {
                evt_tx: evt_tx.clone(),
            },
        );
    }
    #[cfg(feature = "video")]
    {
        builder = builder.accept(
            crate::call::VIDEO_ALPN,
            VideoProto {
                evt_tx: evt_tx.clone(),
            },
        );
    }
    let _router = builder
        .accept(
            crate::sync::SYNC_ALPN,
            crate::sync::SyncProto {
                history: history.clone(),
            },
        )
        .spawn();

    // Build the flock map. No initial flock — the user creates or joins
    // a room via Ctrl+N or Ctrl+J.
    let mut flocks: HashMap<String, FlockHandle> = HashMap::new();

    // If bootstrapping from CLI (starling join BIRD-...), join immediately.
    if let Some(&opener) = bootstrap.first() {
        let room = encode_node_id(&opener);
        join_flock(
            &gossip,
            room,
            bootstrap,
            &mut flocks,
            evt_tx.clone(),
            my_node_id,
            name.clone(),
        )
        .await?;
        if opener != my_node_id {
            let (ep, tx) = (endpoint.clone(), evt_tx.clone());
            tokio::spawn(async move {
                let _ = crate::sync::backfill(ep, opener, 0, tx).await;
            });
        }
    }

    #[cfg(feature = "audio")]
    #[allow(unused)]
    let mut _mic_stream: Option<cpal::Stream> = None;
    #[cfg(feature = "video")]
    #[allow(unused)]
    let mut _cam_thread: Option<std::thread::JoinHandle<()>> = None;

    loop {
        let Some(cmd) = cmd_rx.recv().await else {
            break;
        };
        match cmd {
            Command::SendText { flock, body } => {
                if let Some(h) = flocks.get(&flock) {
                    let msg = ChatMessage {
                        id: uuid::Uuid::new_v4().to_string(),
                        author: name.clone(),
                        body,
                        ts: chrono::Utc::now().timestamp_millis(),
                    };
                    let plaintext = postcard::to_stdvec(&GossipPayload::Chat(msg.clone()))?;
                    h.sender
                        .broadcast(h.crypto.encrypt(&plaintext).into())
                        .await?;
                    let _ = evt_tx.send(AppEvent::Message { flock, msg });
                }
            }

            Command::JoinFlock { code } => {
                if let Some(opener) = decode_node_id(&code) {
                    let room = encode_node_id(&opener);
                    let _ = join_flock(
                        &gossip,
                        room,
                        vec![opener],
                        &mut flocks,
                        evt_tx.clone(),
                        my_node_id,
                        name.clone(),
                    )
                    .await;
                }
            }

            #[cfg(feature = "audio")]
            Command::StartCall(addr) => {
                let (mic_tx, mic_rx) = mpsc::unbounded_channel();
                _mic_stream = Some(crate::voice::start_capture(
                    mic_tx,
                    muted.clone(),
                    input_device.as_deref(),
                )?);
                let ep = endpoint.clone();
                tokio::spawn(async move {
                    let _ = crate::call::place_call(ep, addr, mic_rx).await;
                });
            }
            #[cfg(not(feature = "audio"))]
            Command::StartCall(_) => {
                crate::logger::warn("voice call not supported (audio feature disabled)");
            }

            #[cfg(feature = "audio")]
            Command::HangUp => {
                _mic_stream = None;
            }
            #[cfg(not(feature = "audio"))]
            Command::HangUp => {}

            #[cfg(feature = "video")]
            Command::StartVideo(addr) => {
                let (cam_tx, cam_rx) = mpsc::unbounded_channel();
                _cam_thread = Some(crate::video::start_camera(cam_tx)?);
                let ep = endpoint.clone();
                tokio::spawn(async move {
                    let _ = crate::call::place_video(ep, addr, cam_rx).await;
                });
            }
            #[cfg(not(feature = "video"))]
            Command::StartVideo(_) => {
                crate::logger::warn("video not supported (video feature disabled)");
            }
            #[cfg(feature = "video")]
            Command::StopVideo => {
                _cam_thread = None;
            }
            #[cfg(not(feature = "video"))]
            Command::StopVideo => {}

            Command::Quit => break,
        }
    }

    Ok(())
}

/// Protocol handler for incoming voice call connections.
#[cfg(feature = "audio")]
#[derive(Debug)]
struct VoiceProto {
    evt_tx: mpsc::UnboundedSender<AppEvent>,
}

#[cfg(feature = "audio")]
impl iroh::protocol::ProtocolHandler for VoiceProto {
    async fn accept(&self, conn: Connection) -> Result<(), iroh::protocol::AcceptError> {
        let _ = crate::call::handle_incoming(conn, self.evt_tx.clone()).await;
        Ok(())
    }
}

/// Protocol handler for incoming video call connections.
#[cfg(feature = "video")]
#[derive(Debug)]
struct VideoProto {
    evt_tx: mpsc::UnboundedSender<AppEvent>,
}

#[cfg(feature = "video")]
impl iroh::protocol::ProtocolHandler for VideoProto {
    async fn accept(&self, conn: Connection) -> Result<(), iroh::protocol::AcceptError> {
        let _ = crate::call::recv_video(conn, self.evt_tx.clone()).await;
        Ok(())
    }
}

async fn join_flock(
    gossip: &Gossip,
    code: String,
    boot: Vec<EndpointId>,
    flocks: &mut HashMap<String, FlockHandle>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    my_id: EndpointId,
    name: String,
) -> anyhow::Result<()> {
    if flocks.contains_key(&code) {
        return Ok(());
    }

    let topic = topic_for(&format!("starling/flock/{code}"));
    let crypto = FlockCrypto::from_room_code(&code);
    let (sender, mut receiver) = gossip.subscribe(topic, boot).await?.split();

    let (rx_crypto, rx_code, rx_tx, rx_sender, rx_my_id, rx_name) = (
        FlockCrypto::from_room_code(&code),
        code.clone(),
        evt_tx.clone(),
        sender.clone(),
        my_id,
        name,
    );
    tokio::spawn(async move {
        while let Some(event) = receiver.next().await {
            match event {
                Ok(Event::Received(msg)) => {
                    if let Some(plain) = rx_crypto.decrypt(&msg.content) {
                        match postcard::from_bytes::<GossipPayload>(&plain) {
                            Ok(GossipPayload::Chat(m)) => {
                                let _ = rx_tx.send(AppEvent::Message {
                                    flock: rx_code.clone(),
                                    msg: m,
                                });
                            }
                            Ok(GossipPayload::Profile { id, name }) => {
                                let _ = rx_tx.send(AppEvent::PeerNamed(id, name));
                            }
                            Ok(GossipPayload::Status { id, status }) => {
                                let _ = rx_tx.send(AppEvent::PeerStatus(id, status));
                            }
                            Err(e) => {
                                crate::logger::error(&format!("gossip deserialize error: {e}"));
                            }
                        }
                    }
                }

                Ok(Event::NeighborUp(id)) => {
                    let _ = rx_tx.send(AppEvent::PeerConnected(id));
                    // Announce our profile so the new peer sees our name.
                    let payload = GossipPayload::Profile {
                        id: rx_my_id,
                        name: rx_name.clone(),
                    };
                    if let Ok(plain) = postcard::to_stdvec(&payload) {
                        let _ = rx_sender.broadcast(rx_crypto.encrypt(&plain).into()).await;
                    }
                }
                Ok(Event::NeighborDown(id)) => {
                    let _ = rx_tx.send(AppEvent::PeerDisconnected(id));
                }
                _ => {}
            }
        }
    });

    flocks.insert(code.clone(), FlockHandle { sender, crypto });
    let _ = evt_tx.send(AppEvent::JoinedFlock { code });
    Ok(())
}
