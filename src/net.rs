use crate::event::{AppEvent, Command};
#[cfg(any(feature = "audio", feature = "video"))]
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointId, endpoint::presets, protocol::Router};
use iroh_gossip::{
    api::Event,
    net::{GOSSIP_ALPN, Gossip},
};
use n0_future::StreamExt;
use starling::crypto::FlockCrypto;
use starling::event::{ChatMessage, GossipPayload};
use starling::roost::RoostState;
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
    bootstrap: Option<String>,
    mut cmd_rx: mpsc::UnboundedReceiver<Command>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    muted: Arc<AtomicBool>,
    my_node_id: EndpointId,
    mut name: String,
    input_device: Option<String>,
) -> anyhow::Result<()> {
    #[cfg(feature = "audio")]
    let mut input_device = input_device;
    #[cfg(not(feature = "audio"))]
    let _ = (&muted, &input_device);
    let secret = starling::config::Profile::load_or_create_secret();
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .bind()
        .await?;
    endpoint.online().await;

    starling::logger::warn(&format!("endpoint bound: node_id={my_node_id}"));
    let _ = evt_tx.send(AppEvent::Ticket(my_node_id));

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
                muted: muted.clone(),
                input_device: input_device.clone(),
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

    if let Some(code) = bootstrap {
        join_by_code(
            &gossip,
            &endpoint,
            code,
            &mut flocks,
            evt_tx.clone(),
            my_node_id,
            name.clone(),
        )
        .await?;
    }

    #[cfg(feature = "audio")]
    #[allow(unused)]
    let mut _mic_stream: Option<cpal::Stream> = None;
    #[cfg(feature = "video")]
    #[allow(unused)]
    let mut _camera: Option<crate::video::CameraHandle> = None;
    #[cfg(feature = "video")]
    let mut _video_tx: Option<tokio::sync::broadcast::Sender<Vec<u8>>> = None;

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

            Command::Join { code } => {
                if let Err(error) = join_by_code(
                    &gossip,
                    &endpoint,
                    code,
                    &mut flocks,
                    evt_tx.clone(),
                    my_node_id,
                    name.clone(),
                )
                .await
                {
                    let _ = evt_tx.send(AppEvent::Error(format!("join failed: {error}")));
                }
            }

            Command::UpdateProfile {
                name: new_name,
                input_device: new_input_device,
            } => {
                name = new_name;
                #[cfg(feature = "audio")]
                {
                    input_device = new_input_device;
                }
                #[cfg(not(feature = "audio"))]
                let _ = new_input_device;
            }

            #[cfg(feature = "audio")]
            Command::StartCall(peer) => {
                let (mic_tx, mic_rx) = mpsc::unbounded_channel();
                match crate::voice::start_capture(mic_tx, muted.clone(), input_device.as_deref()) {
                    Ok(stream) => {
                        _mic_stream = Some(stream);
                        let ep = endpoint.clone();
                        let tx = evt_tx.clone();
                        tokio::spawn(async move {
                            if let Err(error) =
                                crate::call::place_call(ep, peer, mic_rx, tx.clone()).await
                            {
                                let _ = tx.send(AppEvent::Error(format!(
                                    "could not start call: {error}"
                                )));
                            }
                        });
                    }
                    Err(error) => {
                        let _ = evt_tx.send(AppEvent::Error(format!(
                            "could not start microphone: {error}"
                        )));
                    }
                }
            }

            #[cfg(feature = "audio")]
            Command::HangUp => {
                _mic_stream = None;
            }

            #[cfg(feature = "video")]
            Command::StartVideo(peers) => {
                _camera = None;
                _video_tx = None;
                let (video_tx, _) = tokio::sync::broadcast::channel(2);
                match crate::video::start_camera(video_tx.clone(), evt_tx.clone()) {
                    Ok(camera) => {
                        _camera = Some(camera);
                        for peer in peers {
                            let ep = endpoint.clone();
                            let frames = video_tx.subscribe();
                            let tx = evt_tx.clone();
                            tokio::spawn(async move {
                                if let Err(error) = crate::call::place_video(ep, peer, frames).await
                                {
                                    let _ = tx.send(AppEvent::Error(format!(
                                        "video to {peer} ended: {error}"
                                    )));
                                }
                            });
                        }
                        _video_tx = Some(video_tx);
                    }
                    Err(error) => {
                        let _ = evt_tx
                            .send(AppEvent::Error(format!("could not start camera: {error}")));
                    }
                }
            }
            #[cfg(feature = "video")]
            Command::StopVideo => {
                _camera = None;
                _video_tx = None;
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
    muted: Arc<AtomicBool>,
    input_device: Option<String>,
}

#[cfg(feature = "audio")]
impl iroh::protocol::ProtocolHandler for VoiceProto {
    async fn accept(&self, conn: Connection) -> Result<(), iroh::protocol::AcceptError> {
        let (mic_tx, mic_rx) = mpsc::unbounded_channel();
        let stream = match crate::voice::start_capture(
            mic_tx,
            self.muted.clone(),
            self.input_device.as_deref(),
        ) {
            Ok(stream) => stream,
            Err(error) => {
                let _ = self
                    .evt_tx
                    .send(AppEvent::Error(format!("could not answer call: {error}")));
                return Ok(());
            }
        };
        let _stream = stream;
        let _ = crate::call::handle_incoming(conn, mic_rx, self.evt_tx.clone()).await;
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

#[allow(clippy::too_many_arguments)]
async fn join_by_code(
    gossip: &Gossip,
    endpoint: &Endpoint,
    code: String,
    flocks: &mut HashMap<String, FlockHandle>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    my_id: EndpointId,
    name: String,
) -> anyhow::Result<()> {
    let decoded = starling::net::decode_typed_code(&code)
        .ok_or_else(|| anyhow::anyhow!("invalid or unsupported typed code"))?;
    match decoded.code_type {
        starling::net::CodeType::Flock => {
            let flock = starling::net::decode_flock_code(&decoded)
                .ok_or_else(|| anyhow::anyhow!("flock code has an invalid payload"))?;
            join_flock(
                gossip,
                code.clone(),
                vec![flock.opener],
                flocks,
                evt_tx.clone(),
                my_id,
                name,
            )
            .await?;
            if flock.opener != my_id {
                let (ep, tx) = (endpoint.clone(), evt_tx.clone());
                tokio::spawn(async move {
                    if let Err(error) =
                        crate::sync::backfill(ep, flock.opener, code, 0, tx.clone()).await
                    {
                        let _ =
                            tx.send(AppEvent::Error(format!("history backfill failed: {error}")));
                    }
                });
            }
            Ok(())
        }
        starling::net::CodeType::Roost => {
            let opener = starling::net::typed_code_node_id(&decoded)
                .ok_or_else(|| anyhow::anyhow!("roost code has an invalid endpoint payload"))?;
            join_roost(gossip, endpoint, code, opener, flocks, evt_tx, my_id, name).await
        }
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
    endpoint: &Endpoint,
    code: String,
    opener: EndpointId,
    flocks: &mut HashMap<String, FlockHandle>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    my_id: EndpointId,
    name: String,
) -> anyhow::Result<()> {
    let control_key = format!("{code}/_control");
    let topic = starling::net::topic_for(&format!("starling/roost/{control_key}"));
    let crypto = FlockCrypto::from_room_code(&control_key);
    let (_sender, mut receiver) = gossip.subscribe(topic, vec![opener]).await?.split();

    let state = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(event) = receiver.next().await {
            if let Ok(Event::Received(msg)) = event
                && let Some(plain) = crypto.decrypt(&msg.content)
                && let Ok(state) = postcard::from_bytes::<RoostState>(&plain)
            {
                return Ok(state);
            }
        }
        anyhow::bail!("roost control subscription ended before state arrived")
    })
    .await
    .map_err(|_| anyhow::anyhow!("roost server did not answer within 10 seconds"))??;

    for channel in &state.channels {
        join_roost_channel(
            gossip,
            &code,
            channel,
            opener,
            flocks,
            evt_tx.clone(),
            my_id,
            name.clone(),
        )
        .await?;

        let ep = endpoint.clone();
        let tx = evt_tx.clone();
        let roost_code = code.clone();
        let channel = channel.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::sync::backfill_roost_channel(
                ep,
                opener,
                &roost_code,
                &channel,
                0,
                tx.clone(),
            )
            .await
            {
                let _ = tx.send(AppEvent::Error(format!(
                    "history backfill failed for #{channel}: {e}"
                )));
            }
        });
    }

    let _ = evt_tx.send(AppEvent::JoinedRoost {
        code,
        name: state.name,
        channels: state.channels,
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn join_roost_channel(
    gossip: &Gossip,
    roost_code: &str,
    channel: &str,
    opener: EndpointId,
    flocks: &mut HashMap<String, FlockHandle>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    my_id: EndpointId,
    name: String,
) -> anyhow::Result<()> {
    let code = format!("{roost_code}/{channel}");
    if flocks.contains_key(&code) {
        return Ok(());
    }

    let topic = starling::net::topic_for(&format!("starling/roost/{code}"));
    let crypto = FlockCrypto::from_room_code(&code);
    let (sender, mut receiver) = gossip.subscribe(topic, vec![opener]).await?.split();
    let rx_crypto = FlockCrypto::from_room_code(&code);
    let rx_code = code.clone();
    let rx_tx = evt_tx.clone();
    let rx_sender = sender.clone();

    tokio::spawn(async move {
        while let Some(event) = receiver.next().await {
            match event {
                Ok(Event::Received(msg)) => {
                    if let Some(plain) = rx_crypto.decrypt(&msg.content) {
                        match postcard::from_bytes::<GossipPayload>(&plain) {
                            Ok(GossipPayload::Chat(msg)) => {
                                let _ = rx_tx.send(AppEvent::Message {
                                    flock: rx_code.clone(),
                                    msg,
                                });
                            }
                            Ok(GossipPayload::Profile { id, name }) => {
                                let _ = rx_tx.send(AppEvent::PeerNamed(id, name));
                            }
                            Ok(GossipPayload::Status { id, status }) => {
                                let _ = rx_tx.send(AppEvent::PeerStatus(id, status));
                            }
                            Err(e) => starling::logger::warn(&format!(
                                "roost channel deserialize error: {e}"
                            )),
                        }
                    }
                }
                Ok(Event::NeighborUp(id)) => {
                    let _ = rx_tx.send(AppEvent::PeerConnected(id));
                    let payload = GossipPayload::Profile {
                        id: my_id,
                        name: name.clone(),
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

    flocks.insert(code, FlockHandle { sender, crypto });
    Ok(())
}
