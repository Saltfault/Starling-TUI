use crate::event::{AppEvent, Command};
use iroh::endpoint::Connection;
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, endpoint::presets, protocol::Router,
};
use iroh_gossip::{
    api::Event,
    net::{GOSSIP_ALPN, Gossip},
};
use n0_future::StreamExt;
use starling::crypto::FlockCrypto;
use starling::event::{ChatMessage, GossipPayload};
use starling::presence::publish_presence;
use starling::roost::{ModRequest, RoostState, RoostWelcome};
use std::collections::{HashMap, HashSet};

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{Duration, timeout};

use zeroize::Zeroizing;

const HISTORY_IO_TIMEOUT: Duration = Duration::from_secs(30);

/// Moderation protocol ALPN: bi-stream where the client sends a `ModRequest`
/// and the roost replies `Result<(), String>`.
const MOD_ALPN: &[u8] = b"starling/mod/0";
/// Join handshake ALPN: the roost opens a uni stream and writes
/// `Result<RoostWelcome, String>`; the client accepts and reads.
const ROOST_JOIN_ALPN: &[u8] = b"starling/roost-join/0";
type HistoryAuthorizer =
    dyn Fn(EndpointId, &starling::protocol::SpaceId) -> bool + Send + Sync + 'static;
type HistoryChallengeCache = Arc<Mutex<HashSet<(EndpointId, [u8; 32])>>>;
pub type HistoryStore = Arc<dyn crate::history::HistoryBackend>;
pub type CancellationToken = tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct HistoryProto {
    store: crate::history_store::SledHistory,
    authorize: Arc<HistoryAuthorizer>,
    seen_challenges: HistoryChallengeCache,
}

impl std::fmt::Debug for HistoryProto {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HistoryProto")
            .finish_non_exhaustive()
    }
}

impl HistoryProto {
    async fn serve(&self, conn: Connection) -> anyhow::Result<()> {
        let remote_id = conn.remote_id();
        let (mut send, mut recv) = timeout(HISTORY_IO_TIMEOUT, conn.accept_bi())
            .await
            .map_err(|_| anyhow::anyhow!("timed out waiting for history request"))??;
        let (header, body) = timeout(
            HISTORY_IO_TIMEOUT,
            starling::protocol::read_frame(&mut recv),
        )
        .await
        .map_err(|_| anyhow::anyhow!("timed out reading history request"))??;
        anyhow::ensure!(
            header.kind == starling::history::FRAME_HISTORY_REQUEST_V1,
            "unexpected history frame kind"
        );
        let signed: starling::history::SignedHistoryRequest = postcard::from_bytes(&body)
            .map_err(|error| anyhow::anyhow!("invalid history request: {error}"))?;
        anyhow::ensure!(
            postcard::to_stdvec(&signed)? == body,
            "history request is not canonical"
        );
        signed.verify(&remote_id)?;
        let mut request = signed.request;
        {
            let mut seen = self
                .seen_challenges
                .lock()
                .map_err(|_| anyhow::anyhow!("history challenge lock poisoned"))?;
            anyhow::ensure!(
                seen.insert((remote_id, request.challenge)),
                "history challenge was replayed"
            );
            if seen.len() > starling::history::MAX_HISTORY_HASHES {
                seen.clear();
                seen.insert((remote_id, request.challenge));
            }
        }
        anyhow::ensure!(
            (self.authorize)(remote_id, &request.space),
            "history request denied: authorizer always rejects (membership persistence not yet implemented; see SRV-9/10)"
        );

        request.max_bytes = request
            .max_bytes
            .min((starling::protocol::MAX_BODY_BYTES - 4096) as u32);
        // All synchronous store access completes before the response write awaits.
        let response = starling::history::reconciliation_page(&self.store, &request)?;
        let encoded = response.encode()?;
        timeout(
            HISTORY_IO_TIMEOUT,
            starling::protocol::write_frame(
                &mut send,
                starling::history::FRAME_HISTORY_RESPONSE_V1,
                &encoded,
            ),
        )
        .await
        .map_err(|_| anyhow::anyhow!("timed out writing history response"))??;
        send.finish()?;
        Ok(())
    }
}

impl iroh::protocol::ProtocolHandler for HistoryProto {
    async fn accept(&self, conn: Connection) -> Result<(), iroh::protocol::AcceptError> {
        self.serve(conn).await.map_err(|error| {
            iroh::protocol::AcceptError::from_err(std::io::Error::other(error.to_string()))
        })
    }
}

use iroh_gossip::api::GossipSender;

struct FlockHandle {
    sender: GossipSender,
    crypto: FlockCrypto,
    cancel: CancellationToken,
}

// V1 runtime ownership lives alongside the V0 `FlockHandle` until command and
// event handling migrate to stable space identities.
#[allow(dead_code)]
pub struct NetworkRuntime {
    endpoint: Endpoint,
    router: Router,
    gossip: Gossip,
    spaces: HashMap<starling::protocol::SpaceId, SpaceHandle>,
    roosts: HashMap<starling::protocol::RoostId, RoostHandle>,
    calls: HashMap<starling::protocol::CallId, CallSession>,
    cancel: CancellationToken,
}

#[allow(dead_code)]
pub struct SpaceHandle {
    descriptor: SpaceDescriptor,
    sender: GossipSender,
    keys: Keyring,
    history: HistoryStore,
    members: (),
    cancel: CancellationToken,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    readiness: SpaceReadiness,
}

#[allow(dead_code)]
pub struct RoostHandle {
    control: AuthenticatedRoostControl,
    manifest: starling::roost::SignedManifestV1,
    members: (),
    history: HistoryStore,
    channels: HashMap<starling::protocol::ChannelId, SpaceHandle>,
    readiness: SpaceReadiness,
    cancel: CancellationToken,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

#[allow(dead_code)]
pub enum SpaceDescriptor {
    Flock(starling::descriptor::SignedFlockDescriptorV1),
    RoostChannel {
        roost: starling::protocol::RoostId,
        channel: starling::protocol::ChannelId,
        manifest_revision: u64,
    },
}

#[derive(Default)]
#[allow(dead_code)]
pub struct Keyring {
    current_epoch: u64,
    keys: HashMap<u64, Zeroizing<[u8; 32]>>,
}

#[allow(dead_code)]
impl Keyring {
    pub fn insert(&mut self, epoch: u64, key: [u8; 32]) {
        self.current_epoch = self.current_epoch.max(epoch);
        self.keys.insert(epoch, Zeroizing::new(key));
    }

    pub fn get(&self, epoch: u64) -> Option<&[u8; 32]> {
        self.keys.get(&epoch).map(|key| &**key)
    }

    pub fn retain_from(&mut self, oldest_epoch: u64) {
        self.keys.retain(|epoch, _| *epoch >= oldest_epoch);
    }
}

#[allow(dead_code)]
pub struct AuthenticatedRoostControl {
    pub roost: starling::protocol::RoostId,
    pub server: EndpointId,
    pub connection: Connection,
    pub manifest_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub enum SpaceReadiness {
    Restored,
    Resolving,
    Authenticating,
    AwaitingKeys,
    Subscribing,
    Reconciling,
    Ready,
    Revoked,
    NeedsUserAction,
    Retrying { attempt: u32 },
}

#[allow(dead_code)]
pub struct CallSession {
    pub call: starling::protocol::CallId,
    pub space: starling::protocol::SpaceId,
    pub owner: EndpointId,
    pub connections: HashMap<EndpointId, Connection>,
    pub cancel: CancellationToken,
    pub tasks: Vec<tokio::task::JoinHandle<()>>,
}

/// Drives one space through the explicit restore lifecycle.
#[allow(dead_code)]
pub async fn drive_restore(handle: &mut SpaceHandle, cancel: CancellationToken) {
    use SpaceReadiness::*;
    loop {
        if cancel.is_cancelled() {
            return;
        }
        handle.readiness = match handle.readiness {
            Retrying { attempt } => {
                let multiplier = 1_u64 << attempt.min(6);
                let backoff = Duration::from_millis(200_u64.saturating_mul(multiplier));
                tokio::select! {
                    () = cancel.cancelled() => return,
                    () = tokio::time::sleep(backoff) => Restored,
                }
            }
            ref state => match advance_readiness(state) {
                Some(next) => next,
                None => return,
            },
        };
    }
}

fn advance_readiness(state: &SpaceReadiness) -> Option<SpaceReadiness> {
    use SpaceReadiness::*;
    match state {
        Restored => Some(Resolving),
        Resolving => Some(Authenticating),
        Authenticating => Some(AwaitingKeys),
        AwaitingKeys => Some(Subscribing),
        Subscribing => Some(Reconciling),
        Reconciling => Some(Ready),
        Ready | Revoked | NeedsUserAction | Retrying { .. } => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    bootstrap: Option<String>,
    restore_codes: HashMap<starling::protocol::SpaceId, String>,
    mut cmd_rx: mpsc::UnboundedReceiver<Command>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    muted: Arc<AtomicBool>,
    my_node_id: EndpointId,
    name: String,
    input_device: Option<String>,
    camera_index: Option<u32>,
    pronouns: String,
) -> anyhow::Result<()> {
    #[cfg(feature = "audio")]
    #[allow(clippy::redundant_locals)]
    let input_device = input_device;
    #[cfg(not(feature = "audio"))]
    let _ = (&muted, &input_device);
    #[cfg(feature = "video")]
    #[allow(clippy::redundant_locals)]
    let camera_index = camera_index;
    #[cfg(not(feature = "video"))]
    let _ = &camera_index;
    let secret = starling::config::Profile::load_or_create_secret();
    // Phase 9: load a separate `crypto_box` DM keypair. It is kept distinct
    // from the ed25519 identity which is used only for signatures, so a
    // compromise-future work on the DM path never touches the permanent
    // identity key. The bytes published to peers on every profile broadcast.
    let dm_secret_bytes = starling::config::Profile::load_or_create_dm_secret_bytes();
    let my_dm_public_bytes = crypto_box::SecretKey::from_bytes(dm_secret_bytes)
        .public_key()
        .to_bytes()
        .to_vec();
    let mut builder = Endpoint::builder(presets::N0).secret_key(secret.clone());
    // Allow a community to point its endpoints at a self-hosted iroh-relay
    // (run beside their roost) without rebuilding. Relays only forward
    // ciphertext the E2E crypto has already sealed, so this drops the last
    // centralized dependency in the flight path.
    if let Ok(url) = std::env::var("STARLING_RELAY") {
        let relay: RelayUrl = url.parse()?;
        builder = builder.relay_mode(RelayMode::Custom(relay.into()));
    }
    let endpoint = builder.bind().await?;
    endpoint.online().await;

    starling::logger::warn(&format!("endpoint bound: node_id={my_node_id}"));
    let _ = evt_tx.send(AppEvent::Ticket(my_node_id));
    let _ = evt_tx.send(AppEvent::DmKey {
        endpoint: my_node_id,
        dm_pk: my_dm_public_bytes.clone(),
    });

    let gossip = Gossip::builder().spawn(endpoint.clone());
    let history: starling::sync::History = Default::default();
    let durable_history = crate::history_store::SledHistory::open(
        starling::config::Profile::config_dir().join("history-v1"),
    )?;
    // Flocks are open-membership: anyone holding the room code is authorized
    // to read history. Roost channel history is served by the roost itself,
    // which enforces membership server-side. So the client-side history
    // server authorizes everyone — the gossip topic already gates who can
    // reach this handler.
    let history_proto = HistoryProto {
        store: durable_history,
        authorize: Arc::new(|_, _| true),
        seen_challenges: Arc::new(Mutex::new(HashSet::new())),
    };

    #[allow(unused_mut)]
    let mut builder = Router::builder(endpoint.clone())
        .accept(GOSSIP_ALPN, gossip.clone())
        .accept(starling::history::HISTORY_V1_ALPN, history_proto);
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
    // V1 ALPNs sit alongside V0 so peers can negotiate via signed call
    // signals (`starling::call::SignedCallSignalV1`) before opening media.
    #[cfg(feature = "audio")]
    {
        builder = builder.accept(
            crate::call::v1::VOICE_V1_ALPN,
            VoiceV1Proto {
                evt_tx: evt_tx.clone(),
                muted: muted.clone(),
                input_device: input_device.clone(),
            },
        );
    }
    #[cfg(feature = "video")]
    {
        builder = builder.accept(
            crate::call::v1::VIDEO_V1_ALPN,
            VideoV1Proto {
                evt_tx: evt_tx.clone(),
            },
        );
    }
    // Keep the Router alive for the lifetime of the network task. Dropping it
    // would unregister all ALPN handlers and break incoming connections, which
    // also prevents outbound joins from negotiating an ALPN with the roost.
    let _router = builder
        .accept(
            starling::sync::SYNC_ALPN,
            starling::sync::SyncProto {
                history: history.clone(),
                members: Arc::new(Mutex::new(starling::membership::MembershipState::genesis(
                    starling::membership::MembershipScopeId::Flock(starling::protocol::FlockId(
                        [0; 32],
                    )),
                    my_node_id,
                ))),
            },
        )
        .spawn();

    let mut flocks: HashMap<String, FlockHandle> = HashMap::new();
    let mut spaces: HashMap<starling::protocol::SpaceId, FlockHandle> = HashMap::new();

    if let Some(code) = bootstrap
        && let Err(e) = join_by_code(
            &gossip,
            &endpoint,
            code,
            0,
            &mut flocks,
            &mut spaces,
            evt_tx.clone(),
            my_node_id,
            name.clone(),
            secret.clone(),
            dm_secret_bytes,
            my_dm_public_bytes.clone(),
            pronouns.clone(),
        )
        .await
    {
        let _ = evt_tx.send(AppEvent::Notice(format!("join failed: {e}")));
    }

    // Rejoin saved contexts that have persisted join codes.
    for (_space_id, code) in restore_codes {
        let since = 0; // Saved contexts start from scratch (history backfill handles old messages).
        if let Err(error) = join_by_code(
            &gossip,
            &endpoint,
            code,
            since,
            &mut flocks,
            &mut spaces,
            evt_tx.clone(),
            my_node_id,
            name.clone(),
            secret.clone(),
            dm_secret_bytes,
            my_dm_public_bytes.clone(),
            pronouns.clone(),
        )
        .await
        {
            let _ = evt_tx.send(AppEvent::Error(format!(
                "failed to rejoin saved context: {error}"
            )));
        }
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
            Command::SendContextText { space, body } => {
                if let Some(handle) = spaces.get(&space) {
                    let msg = ChatMessage {
                        id: uuid::Uuid::new_v4().to_string(),
                        author: name.clone(),
                        body,
                        ts: chrono::Utc::now().timestamp_millis(),
                    };
                    starling::net::broadcast_payload(
                        &handle.sender,
                        &handle.crypto,
                        &secret,
                        &GossipPayload::Chat(msg.clone()),
                    )
                    .await?;
                    let flock_label = match &space {
                        starling::protocol::SpaceId::Flock(_) => format!("{space:?}"),
                        starling::protocol::SpaceId::RoostChannel { roost, channel } => {
                            let roost_code = starling::net::encode_roost_code(
                                &iroh::EndpointId::from_bytes(&roost.0)
                                    .ok()
                                    .unwrap_or(my_node_id),
                            );
                            let channel_name = std::str::from_utf8(&channel.0)
                                .unwrap_or("?")
                                .trim_end_matches('\0');
                            format!("{roost_code}/{channel_name}")
                        }
                    };
                    let _ = evt_tx.send(AppEvent::Message {
                        flock: flock_label,
                        msg,
                        private: false,
                    });
                } else {
                    let _ = evt_tx.send(AppEvent::Notice(format!(
                        "Context {space:?} is not connected — join the space first"
                    )));
                }
            }
            Command::SelectContext(space) => {
                let _ = evt_tx.send(AppEvent::ContextStateChanged {
                    space,
                    state: crate::ui::ContextState::Reconciling,
                });
                let _ = evt_tx.send(AppEvent::ContextStateChanged {
                    space,
                    state: crate::ui::ContextState::Ready,
                });
            }
            Command::RestoreContexts(spaces) => {
                for space in spaces {
                    let _ = evt_tx.send(AppEvent::ContextStateChanged {
                        space,
                        state: crate::ui::ContextState::Restoring,
                    });
                }
            }
            Command::SendText { flock, body } => {
                if let Some(h) = flocks.get(&flock) {
                    let msg = ChatMessage {
                        id: uuid::Uuid::new_v4().to_string(),
                        author: name.clone(),
                        body,
                        ts: chrono::Utc::now().timestamp_millis(),
                    };
                    starling::net::broadcast_payload(
                        &h.sender,
                        &h.crypto,
                        &secret,
                        &GossipPayload::Chat(msg.clone()),
                    )
                    .await?;
                    let _ = evt_tx.send(AppEvent::Message {
                        flock: flock.clone(),
                        msg,
                        private: false,
                    });
                }
            }

            Command::SendChirp {
                flock,
                to,
                their_pk,
                body,
            } => {
                if let Some(h) = flocks.get(&flock) {
                    let Ok(their_pk) = crypto_box::PublicKey::try_from(their_pk.as_slice()) else {
                        let _ = evt_tx.send(AppEvent::Error(
                            "could not seal chirp: unknown DM public key".into(),
                        ));
                        continue;
                    };
                    let msg = ChatMessage {
                        id: uuid::Uuid::new_v4().to_string(),
                        author: name.clone(),
                        body: body.clone(),
                        ts: chrono::Utc::now().timestamp_millis(),
                    };
                    let my_dm_secret = crypto_box::SecretKey::from_bytes(dm_secret_bytes);
                    let plain = postcard::to_stdvec(&msg)?;
                    let sealed = starling::crypto::seal_chirp(&my_dm_secret, &their_pk, &plain)?;
                    starling::net::broadcast_payload(
                        &h.sender,
                        &h.crypto,
                        &secret,
                        &GossipPayload::Chirp { to, sealed },
                    )
                    .await?;
                    let _ = evt_tx.send(AppEvent::Message {
                        flock: flock.clone(),
                        msg,
                        private: true,
                    });
                }
            }

            Command::Join { code, since } => {
                if let Err(error) = join_by_code(
                    &gossip,
                    &endpoint,
                    code,
                    since,
                    &mut flocks,
                    &mut spaces,
                    evt_tx.clone(),
                    my_node_id,
                    name.clone(),
                    secret.clone(),
                    dm_secret_bytes,
                    my_dm_public_bytes.clone(),
                    pronouns.clone(),
                )
                .await
                {
                    let _ = evt_tx.send(AppEvent::Error(format!("join failed: {error}")));
                }
            }

            #[cfg(feature = "audio")]
            Command::StartCall(peers) => {
                if peers.is_empty() {
                    continue;
                }
                let (mic_tx, mut mic_rx) = mpsc::unbounded_channel();
                match crate::voice::start_capture(mic_tx, muted.clone(), input_device.as_deref()) {
                    Ok(stream) => {
                        _mic_stream = Some(stream);
                        let ep = endpoint.clone();
                        let tx = evt_tx.clone();
                        // Fan out mic frames to every peer in the call.
                        let (fan_tx, _) = broadcast::channel::<Vec<u8>>(16);
                        let forwarder_tx = fan_tx.clone();
                        tokio::spawn(async move {
                            while let Some(frame) = mic_rx.recv().await {
                                let _ = forwarder_tx.send(frame);
                            }
                        });
                        for peer in peers {
                            let ep = ep.clone();
                            let tx = tx.clone();
                            let fan_rx = fan_tx.subscribe();
                            tokio::spawn(async move {
                                if let Err(error) =
                                    crate::call::place_call(ep, peer, fan_rx, tx.clone()).await
                                {
                                    let _ = tx.send(AppEvent::Error(format!(
                                        "could not start call: {error}"
                                    )));
                                }
                            });
                        }
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
                match crate::video::start_camera(
                    camera_index.unwrap_or(0),
                    video_tx.clone(),
                    evt_tx.clone(),
                ) {
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

            Command::Ban { roost, target } => {
                let ep = endpoint.clone();
                let tx = evt_tx.clone();
                tokio::spawn(async move {
                    let Ok(conn) = ep.connect(EndpointAddr::from(roost), MOD_ALPN).await else {
                        return;
                    };
                    let Ok((mut send, mut recv)) = conn.open_bi().await else {
                        return;
                    };
                    let req = ModRequest::Ban(target);
                    let _ = send.write_all(&postcard::to_stdvec(&req).unwrap()).await;
                    let _ = send.finish();
                    if let Ok(bytes) = recv.read_to_end(1024).await
                        && let Ok(Err(reason)) = postcard::from_bytes::<Result<(), String>>(&bytes)
                    {
                        let _ = tx.send(AppEvent::Notice(reason));
                    }
                });
            }

            Command::Kick { roost, target } => {
                let ep = endpoint.clone();
                let tx = evt_tx.clone();
                tokio::spawn(async move {
                    let Ok(conn) = ep.connect(EndpointAddr::from(roost), MOD_ALPN).await else {
                        return;
                    };
                    let Ok((mut send, mut recv)) = conn.open_bi().await else {
                        return;
                    };
                    let req = ModRequest::Kick(target);
                    let _ = send.write_all(&postcard::to_stdvec(&req).unwrap()).await;
                    let _ = send.finish();
                    if let Ok(bytes) = recv.read_to_end(1024).await
                        && let Ok(Err(reason)) = postcard::from_bytes::<Result<(), String>>(&bytes)
                    {
                        let _ = tx.send(AppEvent::Notice(reason));
                    }
                });
            }

            Command::SetRole {
                roost,
                target,
                role_index,
            } => {
                let ep = endpoint.clone();
                let tx = evt_tx.clone();
                tokio::spawn(async move {
                    let Ok(conn) = ep.connect(EndpointAddr::from(roost), MOD_ALPN).await else {
                        return;
                    };
                    let Ok((mut send, mut recv)) = conn.open_bi().await else {
                        return;
                    };
                    let req = ModRequest::SetRole { target, role_index };
                    let _ = send.write_all(&postcard::to_stdvec(&req).unwrap()).await;
                    let _ = send.finish();
                    if let Ok(bytes) = recv.read_to_end(1024).await
                        && let Ok(Err(reason)) = postcard::from_bytes::<Result<(), String>>(&bytes)
                    {
                        let _ = tx.send(AppEvent::Notice(reason));
                    }
                });
            }

            Command::TransferOwnership { roost, target } => {
                let ep = endpoint.clone();
                let tx = evt_tx.clone();
                tokio::spawn(async move {
                    let Ok(conn) = ep.connect(EndpointAddr::from(roost), MOD_ALPN).await else {
                        return;
                    };
                    let Ok((mut send, mut recv)) = conn.open_bi().await else {
                        return;
                    };
                    let req = ModRequest::TransferOwnership(target);
                    let _ = send.write_all(&postcard::to_stdvec(&req).unwrap()).await;
                    let _ = send.finish();
                    if let Ok(bytes) = recv.read_to_end(1024).await
                        && let Ok(Err(reason)) = postcard::from_bytes::<Result<(), String>>(&bytes)
                    {
                        let _ = tx.send(AppEvent::Notice(reason));
                    }
                });
            }
            Command::Invite { roost, target } => {
                let ep = endpoint.clone();
                let tx = evt_tx.clone();
                tokio::spawn(async move {
                    let Ok(conn) = ep.connect(EndpointAddr::from(roost), MOD_ALPN).await else {
                        return;
                    };
                    let Ok((mut send, mut recv)) = conn.open_bi().await else {
                        return;
                    };
                    let req = ModRequest::Invite(target);
                    let _ = send.write_all(&postcard::to_stdvec(&req).unwrap()).await;
                    let _ = send.finish();
                    if let Ok(bytes) = recv.read_to_end(1024).await
                        && let Ok(Err(reason)) = postcard::from_bytes::<Result<(), String>>(&bytes)
                    {
                        let _ = tx.send(AppEvent::Notice(reason));
                    }
                });
            }

            Command::AddChannel { roost, channel } => {
                let ep = endpoint.clone();
                let tx = evt_tx.clone();
                tokio::spawn(async move {
                    let Ok(conn) = ep.connect(EndpointAddr::from(roost), MOD_ALPN).await else {
                        return;
                    };
                    let Ok((mut send, mut recv)) = conn.open_bi().await else {
                        return;
                    };
                    let req = ModRequest::AddChannel(channel);
                    let _ = send.write_all(&postcard::to_stdvec(&req).unwrap()).await;
                    let _ = send.finish();
                    if let Ok(bytes) = recv.read_to_end(1024).await
                        && let Ok(Err(reason)) = postcard::from_bytes::<Result<(), String>>(&bytes)
                    {
                        let _ = tx.send(AppEvent::Notice(reason));
                    }
                });
            }

            Command::RemoveChannel { roost, channel } => {
                let ep = endpoint.clone();
                let tx = evt_tx.clone();
                tokio::spawn(async move {
                    let Ok(conn) = ep.connect(EndpointAddr::from(roost), MOD_ALPN).await else {
                        return;
                    };
                    let Ok((mut send, mut recv)) = conn.open_bi().await else {
                        return;
                    };
                    let req = ModRequest::RemoveChannel(channel);
                    let _ = send.write_all(&postcard::to_stdvec(&req).unwrap()).await;
                    let _ = send.finish();
                    if let Ok(bytes) = recv.read_to_end(1024).await
                        && let Ok(Err(reason)) = postcard::from_bytes::<Result<(), String>>(&bytes)
                    {
                        let _ = tx.send(AppEvent::Notice(reason));
                    }
                });
            }

            Command::RenameRoost { roost, name } => {
                let ep = endpoint.clone();
                let tx = evt_tx.clone();
                tokio::spawn(async move {
                    let Ok(conn) = ep.connect(EndpointAddr::from(roost), MOD_ALPN).await else {
                        return;
                    };
                    let Ok((mut send, mut recv)) = conn.open_bi().await else {
                        return;
                    };
                    let req = ModRequest::Rename(name);
                    let _ = send.write_all(&postcard::to_stdvec(&req).unwrap()).await;
                    let _ = send.finish();
                    if let Ok(bytes) = recv.read_to_end(1024).await
                        && let Ok(Err(reason)) = postcard::from_bytes::<Result<(), String>>(&bytes)
                    {
                        let _ = tx.send(AppEvent::Notice(reason));
                    }
                });
            }

            Command::DeleteMessage { roost, channel, id } => {
                let ep = endpoint.clone();
                let tx = evt_tx.clone();
                tokio::spawn(async move {
                    let Ok(conn) = ep.connect(EndpointAddr::from(roost), MOD_ALPN).await else {
                        return;
                    };
                    let Ok((mut send, mut recv)) = conn.open_bi().await else {
                        return;
                    };
                    let req = ModRequest::DeleteMessage { channel, id };
                    let _ = send.write_all(&postcard::to_stdvec(&req).unwrap()).await;
                    let _ = send.finish();
                    if let Ok(bytes) = recv.read_to_end(1024).await
                        && let Ok(Err(reason)) = postcard::from_bytes::<Result<(), String>>(&bytes)
                    {
                        let _ = tx.send(AppEvent::Notice(reason));
                    }
                });
            }

            Command::CreateRoost { name } => {
                let tx = evt_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = starling::roost::server::create(&name) {
                        let _ = tx.send(AppEvent::Error(format!("roost '{name}' failed: {e}")));
                        return;
                    }
                    let (_console_tx, console_rx) = tokio::sync::mpsc::unbounded_channel();
                    match starling::roost::server::open(&name, true, console_rx).await {
                        Ok(()) => {
                            let _ = tx.send(AppEvent::Notice(format!("roost '{name}' started")));
                        }
                        Err(e) => {
                            let _ = tx.send(AppEvent::Error(format!("roost '{name}' failed: {e}")));
                        }
                    }
                });
            }
            Command::Quit => break,
            Command::Leave { code } => {
                // Remove the top-level handle and any derived roost channel
                // handles (keys of the form "{code}/{channel}").
                let prefix = format!("{code}/");
                flocks.retain(|key, handle| {
                    if key == &code || key.starts_with(&prefix) {
                        handle.cancel.cancel();
                        false
                    } else {
                        true
                    }
                });
            }
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

/// V1 voice protocol handler. The V1 path negotiates via signed call signals
/// (`crate::call::v1::SignedCallSignalV1`) and then runs a per-peer
/// [`crate::call::v1::MediaSession`]; this handler accepts the QUIC connection
/// and forwards datagrams to the UI while the session layer is wired up.
#[cfg(feature = "audio")]
#[derive(Debug)]
struct VoiceV1Proto {
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    muted: Arc<AtomicBool>,
    input_device: Option<String>,
}

#[cfg(feature = "audio")]
impl iroh::protocol::ProtocolHandler for VoiceV1Proto {
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

/// V1 video protocol handler. Mirrors V0 until per-peer video mixing lands.
#[cfg(feature = "video")]
#[derive(Debug)]
struct VideoV1Proto {
    evt_tx: mpsc::UnboundedSender<AppEvent>,
}

#[cfg(feature = "video")]
impl iroh::protocol::ProtocolHandler for VideoV1Proto {
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
    since: i64,
    flocks: &mut HashMap<String, FlockHandle>,
    spaces: &mut HashMap<starling::protocol::SpaceId, FlockHandle>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    my_id: EndpointId,
    name: String,
    secret: iroh::SecretKey,
    dm_secret_bytes: [u8; 32],
    my_dm_public_bytes: Vec<u8>,
    pronouns: String,
) -> anyhow::Result<()> {
    let decoded = starling::net::decode_typed_code(&code)
        .ok_or_else(|| anyhow::anyhow!("invalid or unsupported typed code"))?;
    match decoded.code_type {
        starling::net::CodeType::Flock => {
            let flock = starling::net::decode_flock_code(&decoded)
                .ok_or_else(|| anyhow::anyhow!("flock code has an invalid payload"))?;
            // Phase 9: high-entropy flock keys. The flock cipher is derived
            // from the 32-byte secret inside the typed code, NOT from the
            // displayed code's characters. The displayed code still binds the
            // gossip topic (so peers with the right code find each other),
            join_flock(
                gossip,
                code.clone(),
                vec![flock.opener],
                flock.secret,
                flocks,
                spaces,
                evt_tx.clone(),
                my_id,
                flock.name,
                name,
                secret.clone(),
                dm_secret_bytes,
                my_dm_public_bytes.clone(),
                pronouns.clone(),
            )
            .await?;
            if flock.opener != my_id {
                let (ep, tx) = (endpoint.clone(), evt_tx.clone());
                tokio::spawn(async move {
                    if let Err(error) =
                        crate::sync::backfill(ep, flock.opener, code, since, tx.clone()).await
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
            join_roost(
                gossip,
                endpoint,
                code,
                since,
                opener,
                flocks,
                spaces,
                evt_tx,
                my_id,
                name,
                secret,
                dm_secret_bytes,
                my_dm_public_bytes,
                pronouns,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn join_flock(
    gossip: &Gossip,
    code: String,
    boot: Vec<EndpointId>,
    flock_secret: [u8; 32],
    flocks: &mut HashMap<String, FlockHandle>,
    _spaces: &mut HashMap<starling::protocol::SpaceId, FlockHandle>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    my_id: EndpointId,
    flock_name: String,
    name: String,
    secret: iroh::SecretKey,
    dm_secret_bytes: [u8; 32],
    my_dm_public_bytes: Vec<u8>,
    pronouns: String,
) -> anyhow::Result<()> {
    if flocks.contains_key(&code) {
        return Ok(());
    }

    let topic = starling::net::topic_for(&format!("starling/flock/{code}"));
    let crypto = FlockCrypto::from_secret(&flock_secret);
    let (sender, mut receiver) = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        gossip.subscribe(topic, boot),
    )
    .await
    .map_err(|_| anyhow::anyhow!("timed out joining flock — peer unreachable"))??
    .split();

    let rx_space = starling::protocol::SpaceId::Flock(starling::protocol::FlockId(flock_secret));
    let identity_secret = secret.clone();
    let (
        rx_crypto,
        rx_code,
        rx_tx,
        rx_sender,
        rx_my_id,
        rx_name,
        rx_secret,
        rx_dm_secret,
        rx_dm_pk,
        rx_pronouns,
    ) = (
        FlockCrypto::from_secret(&flock_secret),
        code.clone(),
        evt_tx.clone(),
        sender.clone(),
        my_id,
        name,
        secret,
        dm_secret_bytes,
        my_dm_public_bytes,
        pronouns,
    );
    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    tokio::spawn(async move {
        let mut dm_pks: HashMap<EndpointId, Vec<u8>> = HashMap::new();
        let my_dm_secret = crypto_box::SecretKey::from_bytes(rx_dm_secret);
        loop {
            tokio::select! {
                () = task_cancel.cancelled() => break,
                event = receiver.next() => {
                    let Some(event) = event else { break };
                    match event {
                Ok(Event::Received(msg)) => {
                    let Some(envelope) = starling::net::receive_payload(&rx_crypto, &msg.content)
                        .ok()
                        .flatten() else { continue };
                    match envelope.payload {
                        GossipPayload::Chat(m) => {
                            let _ = rx_tx.send(AppEvent::Message {
                                flock: rx_code.clone(),
                                msg: m,
                                private: false,
                            });
                        }
                        GossipPayload::Profile { id, name, dm_pk, pronouns } => {
                            if id != envelope.author {
                                starling::logger::warn(&format!(
                                    "dropped spoofed profile: \
                                     claimed id {id} does not match verified author {}",
                                     envelope.author
                                ));
                                continue;
                            }
                            let _ = rx_tx.send(AppEvent::PeerNamed {
                                space: rx_space,
                                id,
                                name,
                                pronouns,
                            });
                            if !dm_pk.is_empty() {
                                dm_pks.insert(id, dm_pk.clone());
                                let _ = rx_tx.send(AppEvent::DmKey {
                                    endpoint: id,
                                    dm_pk,
                                });
                            }
                        }
                        GossipPayload::Status { id, status } => {
                            if id != envelope.author {
                                continue;
                            }
                            let _ = rx_tx.send(AppEvent::PeerStatus(id, status));
                        }
                        GossipPayload::Presence(lease) => {
                            let _ = rx_tx.send(AppEvent::PresenceLease(lease));
                        }
                        GossipPayload::Chirp { to, sealed } if to == rx_my_id => {
                            let Ok(their_pk) = crypto_box::PublicKey::try_from(
                                dm_pks.get(&envelope.author).cloned().unwrap_or_default().as_slice()
                            ) else { continue };
                            let Some(plain) = starling::crypto::open_chirp(
                                &my_dm_secret,
                                &their_pk,
                                &sealed,
                            ) else { continue };
                            if let Ok(m) = postcard::from_bytes::<ChatMessage>(&plain) {
                                let _ = rx_tx.send(AppEvent::Message {
                                    flock: rx_code.clone(),
                                    msg: m,
                                    private: true,
                                });
                            } else {
                                starling::logger::warn("dropped chirp: could not decode sealed body");
                            }
                        }
                        GossipPayload::Chirp { .. } => {
                            // Not addressed to us; relay only, cannot open.
                        }
                    }
                }

                Ok(Event::NeighborUp(id)) => {
                    let _ = rx_tx.send(AppEvent::PeerConnected {
                        space: rx_space,
                        id,
                    });
                    let payload = GossipPayload::Profile {
                        id: rx_my_id,
                        name: rx_name.clone(),
                        dm_pk: rx_dm_pk.clone(),
                        pronouns: rx_pronouns.clone(),
                    };
                    let _ = starling::net::broadcast_payload(
                        &rx_sender,
                        &rx_crypto,
                        &rx_secret,
                        &payload,
                    )
                    .await;
                }
                Ok(Event::NeighborDown(id)) => {
                    let _ = rx_tx.send(AppEvent::PeerConnectivityHintDown(id));
                }
                _ => {}
            }
                }
            }
        }
    });

    let (_, presence_changes) = mpsc::unbounded_channel();
    let presence_crypto = FlockCrypto::from_secret(&flock_secret);
    tokio::spawn(publish_presence(
        sender.clone(),
        presence_crypto,
        rx_space,
        identity_secret,
        presence_changes,
        cancel.clone(),
    ));
    flocks.insert(
        code.clone(),
        FlockHandle {
            sender,
            crypto,
            cancel,
        },
    );
    let _ = evt_tx.send(AppEvent::JoinedFlock {
        code,
        name: flock_name,
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn join_roost(
    gossip: &Gossip,
    endpoint: &Endpoint,
    code: String,
    since: i64,
    opener: EndpointId,
    flocks: &mut HashMap<String, FlockHandle>,
    spaces: &mut HashMap<starling::protocol::SpaceId, FlockHandle>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    my_id: EndpointId,
    name: String,
    secret: iroh::SecretKey,
    dm_secret_bytes: [u8; 32],
    my_dm_public_bytes: Vec<u8>,
    pronouns: String,
) -> anyhow::Result<()> {
    let control_key = format!("{code}/_control");

    // Phase 9: get the control channel secret from the join handshake BEFORE
    // subscribing, so the control channel can be encrypted with a high-entropy
    // secret rather than a public-derivable room code. A non-member who knows
    // the roost code can find the topic but cannot decrypt the member/ban list
    // that travels on it. Fall back to `from_room_code` for old servers that
    // don't include `control_secret` in the welcome.
    //
    // Identity-gated join handshake: the roost re-checks membership before
    // releasing per-channel secrets. Non-members never receive a welcome.
    starling::logger::warn(&format!(
        "join_roost: connecting to {opener} with ALPN {}",
        String::from_utf8_lossy(ROOST_JOIN_ALPN)
    ));
    let conn = endpoint
        .connect(EndpointAddr::from(opener), ROOST_JOIN_ALPN)
        .await?;
    let mut recv = conn.accept_uni().await?;
    let bytes = recv.read_to_end(65_536).await?;
    let welcome: Result<RoostWelcome, String> = postcard::from_bytes(&bytes)?;

    let welcome = match welcome {
        Ok(welcome) => welcome,
        Err(reason) => {
            let _ = evt_tx.send(AppEvent::Notice(format!("roost refused: {reason}")));
            return Ok(());
        }
    };

    let control_crypto = match welcome.control_secret {
        Some(secret) => FlockCrypto::from_secret(&secret),
        None =>
        {
            #[allow(deprecated)]
            FlockCrypto::from_room_code(&control_key)
        }
    };

    let topic = starling::net::topic_for(&format!("starling/roost/{control_key}"));
    let (_sender, mut receiver) = gossip.subscribe(topic, vec![opener]).await?.split();

    let state = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(event) = receiver.next().await {
            if let Ok(Event::Received(msg)) = event
                && let Some(plain) = control_crypto.decrypt(&msg.content)
                && let Ok(state) = postcard::from_bytes::<RoostState>(&plain)
            {
                return Ok(state);
            }
        }
        anyhow::bail!("roost control subscription ended before state arrived")
    })
    .await
    .map_err(|_| anyhow::anyhow!("roost server did not answer within 10 seconds"))??;

    for (channel, channel_secret) in &welcome.channels {
        join_roost_channel(
            gossip,
            &code,
            channel,
            *channel_secret,
            opener,
            flocks,
            spaces,
            evt_tx.clone(),
            my_id,
            name.clone(),
            secret.clone(),
            dm_secret_bytes,
            my_dm_public_bytes.clone(),
            pronouns.clone(),
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
                since,
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

    // Keep the control channel alive for ongoing RoostState broadcasts so the
    // TUI can refresh its view of permissions and roles as the roost mutates.
    let ctl_crypto = control_crypto;
    let ctl_code = code.clone();
    let ctl_tx = evt_tx.clone();
    tokio::spawn(async move {
        while let Some(event) = receiver.next().await {
            if let Ok(Event::Received(msg)) = event
                && let Some(plain) = ctl_crypto.decrypt(&msg.content)
                && let Ok(state) = postcard::from_bytes::<RoostState>(&plain)
            {
                let _ = ctl_tx.send(AppEvent::RoostUpdate {
                    code: ctl_code.clone(),
                    name: state.name,
                    channels: state.channels,
                    perms: state.perms,
                });
            }
        }
    });

    let _ = evt_tx.send(AppEvent::JoinedRoost {
        code,
        name: welcome.name,
        channels: welcome.channels.iter().map(|(c, _)| c.clone()).collect(),
        perms: state.perms,
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn join_roost_channel(
    gossip: &Gossip,
    roost_code: &str,
    channel: &str,
    secret: [u8; 32],
    opener: EndpointId,
    flocks: &mut HashMap<String, FlockHandle>,
    spaces: &mut HashMap<starling::protocol::SpaceId, FlockHandle>,
    evt_tx: mpsc::UnboundedSender<AppEvent>,
    my_id: EndpointId,
    name: String,
    identity_secret: iroh::SecretKey,
    dm_secret_bytes: [u8; 32],
    my_dm_public_bytes: Vec<u8>,
    pronouns: String,
) -> anyhow::Result<()> {
    let code = format!("{roost_code}/{channel}");
    if flocks.contains_key(&code) {
        return Ok(());
    }

    let topic = starling::net::topic_for(&format!("starling/roost/{code}"));
    starling::logger::info(&format!(
        "join_roost_channel: subscribing to '{code}' with bootstrap {opener}"
    ));
    let (sender, mut receiver) = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        gossip.subscribe(topic, vec![opener]),
    )
    .await
    .map_err(|_| anyhow::anyhow!("timed out subscribing to roost channel '{code}'"))??
    .split();
    let rx_crypto = FlockCrypto::from_secret(&secret);
    let rx_code = code.clone();
    let rx_tx = evt_tx.clone();
    let rx_sender = sender.clone();
    let rx_my_id = my_id;
    let rx_name = name.clone();
    let rx_dm_secret = dm_secret_bytes;
    let rx_pronouns = pronouns;
    let space_id = starling::protocol::SpaceId::RoostChannel {
        roost: starling::protocol::RoostId(*opener.as_bytes()),
        channel: channel_id_from_name(channel),
    };

    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let identity_secret2 = identity_secret.clone();
    tokio::spawn(async move {
        let rx_space = space_id;
        let mut dm_pks: HashMap<EndpointId, Vec<u8>> = HashMap::new();
        let my_dm_secret = crypto_box::SecretKey::from_bytes(rx_dm_secret);
        loop {
            tokio::select! {
                () = task_cancel.cancelled() => break,
                event = receiver.next() => {
                    let Some(event) = event else { break };
                    match event {
                Ok(Event::Received(msg)) => {
                    let Some(envelope) = starling::net::receive_payload(&rx_crypto, &msg.content)
                        .ok()
                        .flatten() else { continue };
                    match envelope.payload {
                        GossipPayload::Chat(m) => {
                            let _ = rx_tx.send(AppEvent::Message {
                                flock: rx_code.clone(),
                                msg: m,
                                private: false,
                            });
                        }
                        GossipPayload::Profile { id, name, dm_pk, pronouns } => {
                            if id != envelope.author {
                                starling::logger::warn(&format!(
                                    "roost channel: dropped spoofed profile \
                                     from claimed id {id} (verified {})",
                                     envelope.author
                                ));
                                continue;
                            }
                            let _ = rx_tx.send(AppEvent::PeerNamed {
                                space: rx_space,
                                id,
                                name,
                                pronouns,
                            });
                            if !dm_pk.is_empty() {
                                dm_pks.insert(id, dm_pk.clone());
                                let _ = rx_tx.send(AppEvent::DmKey {
                                    endpoint: id,
                                    dm_pk,
                                });
                            }
                        }
                        GossipPayload::Status { id, status } => {
                            if id != envelope.author {
                                continue;
                            }
                            let _ = rx_tx.send(AppEvent::PeerStatus(id, status));
                        }
                        GossipPayload::Presence(lease) => {
                            let _ = rx_tx.send(AppEvent::PresenceLease(lease));
                        }
                        GossipPayload::Chirp { to, sealed } if to == rx_my_id => {
                            let Ok(their_pk) = crypto_box::PublicKey::try_from(
                                dm_pks.get(&envelope.author).cloned().unwrap_or_default().as_slice()
                            ) else { continue };
                            let Some(plain) =
                                starling::crypto::open_chirp(&my_dm_secret, &their_pk, &sealed)
                            else { continue };
                            if let Ok(m) = postcard::from_bytes::<ChatMessage>(&plain) {
                                let _ = rx_tx.send(AppEvent::Message {
                                    flock: rx_code.clone(),
                                    msg: m,
                                    private: true,
                                });
                            } else {
                                starling::logger::warn(
                                    "dropped chirp: could not decode sealed body",
                                );
                            }
                        }
                        GossipPayload::Chirp { .. } => {
                            // Not addressed to us; relay-only.
                        }
                    }
                }
                Ok(Event::NeighborUp(id)) => {
                    starling::logger::info(&format!(
                        "roost channel '{rx_code}': peer up {id}"
                    ));
                    let _ = rx_tx.send(AppEvent::PeerConnected {
                        space: rx_space,
                        id,
                    });
                    let payload = GossipPayload::Profile {
                        id: rx_my_id,
                        name: rx_name.clone(),
                        dm_pk: my_dm_public_bytes.clone(),
                        pronouns: rx_pronouns.clone(),
                    };
                    let _ = starling::net::broadcast_payload(
                        &rx_sender,
                        &rx_crypto,
                        &identity_secret,
                        &payload,
                    )
                    .await;
                }
                Ok(Event::NeighborDown(id)) => {
                    starling::logger::info(&format!(
                        "roost channel '{rx_code}': peer down {id}"
                    ));
                    let _ = rx_tx.send(AppEvent::PeerConnectivityHintDown(id));
                }
                _ => {}
            }
                }
            }
        }
    });

    let (_, presence_changes) = mpsc::unbounded_channel();
    let presence_crypto = FlockCrypto::from_secret(&secret);
    tokio::spawn(publish_presence(
        sender.clone(),
        presence_crypto,
        space_id,
        identity_secret2,
        presence_changes,
        cancel.clone(),
    ));

    let flock_handle = FlockHandle {
        sender: sender.clone(),
        crypto: FlockCrypto::from_secret(&secret),
        cancel: cancel.clone(),
    };
    flocks.insert(code.clone(), flock_handle);
    // Track the deterministic SpaceId so V1 context sends can find this handle.
    spaces.insert(
        space_id,
        FlockHandle {
            sender,
            crypto: FlockCrypto::from_secret(&secret),
            cancel,
        },
    );
    Ok(())
}

/// Derive a deterministic [`ChannelId`] from a channel name (mirrors the server).
pub(crate) fn channel_id_from_name(name: &str) -> starling::protocol::ChannelId {
    let mut id = [0u8; 16];
    let bytes = name.as_bytes();
    let len = bytes.len().min(16);
    id[..len].copy_from_slice(&bytes[..len]);
    starling::protocol::ChannelId(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_follows_the_required_restore_order() {
        let mut state = SpaceReadiness::Restored;
        for expected in [
            SpaceReadiness::Resolving,
            SpaceReadiness::Authenticating,
            SpaceReadiness::AwaitingKeys,
            SpaceReadiness::Subscribing,
            SpaceReadiness::Reconciling,
            SpaceReadiness::Ready,
        ] {
            state = advance_readiness(&state).expect("non-terminal transition");
            assert_eq!(state, expected);
        }
        assert!(advance_readiness(&state).is_none());
        assert!(advance_readiness(&SpaceReadiness::Revoked).is_none());
        assert!(advance_readiness(&SpaceReadiness::NeedsUserAction).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn retry_backoff_is_bounded_and_cancellable() {
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            tokio::select! {
                () = task_cancel.cancelled() => true,
                () = tokio::time::sleep(Duration::from_millis(200 * (1 << 6))) => false,
            }
        });
        cancel.cancel();
        assert!(task.await.expect("retry task panicked"));
    }

    /// Verify that a chat message survives encrypt → broadcast → decrypt
    /// round-trip through the FlockCrypto and GossipPayload layers.
    #[test]
    fn message_encrypt_decrypt_round_trip() {
        let secret = iroh::SecretKey::generate();
        let crypto = starling::crypto::FlockCrypto::from_secret(&[42u8; 32]);

        let msg = starling::event::ChatMessage {
            id: "test-1".into(),
            author: "alice".into(),
            body: "hello from test".into(),
            ts: 1,
        };
        let payload = starling::event::GossipPayload::Chat(msg.clone());

        // Simulate broadcast: serialize → sign → encrypt.
        let payload_bytes = postcard::to_stdvec(&payload).unwrap();
        let signed = starling::event::Signed::sign(&secret, payload_bytes);
        let signed_bytes = postcard::to_stdvec(&signed).unwrap();
        let ciphertext = crypto.try_encrypt(&signed_bytes).unwrap();

        // Simulate receive: decrypt → deserialize → verify → deserialize payload.
        let decrypted = crypto.decrypt(&ciphertext).unwrap();
        let envelope: starling::event::Signed = postcard::from_bytes(&decrypted).unwrap();
        envelope.verify().unwrap();
        let received: starling::event::GossipPayload =
            postcard::from_bytes(&envelope.payload).unwrap();

        match received {
            starling::event::GossipPayload::Chat(m) => {
                assert_eq!(m.id, "test-1");
                assert_eq!(m.author, "alice");
                assert_eq!(m.body, "hello from test");
            }
            _ => panic!("expected Chat payload"),
        }
    }
}
