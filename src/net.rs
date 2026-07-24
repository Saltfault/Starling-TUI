use starling::crypto::FlockCrypto;
use starling::event::{ChatMessage, GossipPayload};
use crate::event::{AppEvent, Command};
use starling::roost::RoostState;
#[cfg(any(feature = "audio", feature = "video"))]
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointId, endpoint::presets, protocol::Router};
use iroh_gossip::{
    api::Event,
    net::{GOSSIP_ALPN, Gossip},
};
use n0_future::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc;

use iroh_gossip::api::GossipSender;

struct FlockHandle {
    sender: GossipSender,
    crypto: FlockCrypto,
}

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
    let secret = starling::config::Profile::load_or_create_secret();
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .bind()
        .await?;
    endpoint.online().await;

    let my_code = starling::net::encode_node_id(&my_node_id);
    starling::logger::warn(&format!("endpoint bound: room_code={my_code}"));
    let _ = evt_tx.send(AppEvent::Ticket(my_code));

    let gossip = Gossip::builder().spawn(endpoint.clone());
    let history: starling::sync::History = Default::default();

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
            starling::sync::SYNC_ALPN,
            starling::sync::SyncProto {
                history: history.clone(),
            },
        )
        .spawn();

    let mut flocks: HashMap<String, FlockHandle> = HashMap::new();

    if let Some(&opener) = bootstrap.first() {
        let room = starling::net::encode_node_id(&opener);
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
                if let Some(opener) = starling::net::decode_node_id(&code) {
                    let room = starling::net::encode_node_id(&opener);
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

            Command::JoinRoost { code } => {
                if let Some(opener) = starling::net::decode_node_id(&code) {
                    let _ = join_roost(&gossip, opener, evt_tx.clone()).await;
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

            #[cfg(feature = "audio")]
            Command::HangUp => {
                _mic_stream = None;
            }

            #[cfg(feature = "video")]
            Command::StartVideo(addr) => {
                let (cam_tx, cam_rx) = mpsc::unbounded_channel();
                _cam_thread = Some(crate::video::start_camera(cam_tx)?);
                let ep = endpoint.clone();
                tokio::spawn(async move {
                    let _ = crate::call::place_video(ep, addr, cam_rx).await;
                });
            }
            #[cfg(feature = "video")]
            Command::StopVideo => {
                _cam_thread = None;
            }

            Command::Quit => break,
        }
    }

    Ok(())
}

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

    let topic = starling::net::topic_for(&format!("starling/flock/{code}"));
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
                                starling::logger::error(&format!("gossip deserialize error: {e}"));
                            }
                        }
                    }
                }

                Ok(Event::NeighborUp(id)) => {
                    let _ = rx_tx.send(AppEvent::PeerConnected(id));
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

async fn join_roost(
    gossip: &Gossip,
    opener: EndpointId,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
) -> anyhow::Result<()> {
    let code = starling::net::encode_node_id(&opener);
    let topic = starling::net::topic_for(&format!("starling/roost/{code}"));
    let (_sender, mut receiver) = gossip.subscribe(topic, vec![opener]).await?.split();

    let tx = evt_tx.clone();
    let rx_code = code.clone();
    tokio::spawn(async move {
        while let Some(event) = receiver.next().await {
            match event {
                Ok(Event::Received(msg)) => {
                    if let Ok(state) = postcard::from_bytes::<RoostState>(&msg.content) {
                        let _ = tx.send(AppEvent::RoostUpdate {
                            code: rx_code.clone(),
                            name: state.name,
                            channels: state.channels,
                        });
                    }
                }
                _ => {}
            }
        }
    });

    let _ = evt_tx.send(AppEvent::JoinedRoost {
        code,
        name: String::new(),
        channels: vec![],
    });
    Ok(())
}
