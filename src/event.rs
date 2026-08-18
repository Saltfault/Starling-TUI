use iroh::EndpointId;

pub enum Command {
    #[allow(dead_code)]
    SendContextText {
        space: starling::protocol::SpaceId,
        body: String,
    },
    SelectContext(starling::protocol::SpaceId),
    RestoreContexts(Vec<starling::protocol::SpaceId>),
    SendText {
        flock: String,
        body: String,
    },
    /// Send a sealed 1:1 chirp through a shared flock. The body is encrypted
    /// to `to`'s published DM public key with `crypto_box`; the flock relays
    /// opaque bytes it can't read, and only the addressee's DM private key
    /// opens it. `their_pk` is looked up from the local copy of peers'
    /// profile-announced DM public keys, populated by the typed-chat [
    /// `AppEvent::DmKey`] handler.
    SendChirp {
        flock: String,
        to: EndpointId,
        their_pk: Vec<u8>,
        body: String,
    },
    Join {
        code: String,
        since: i64,
    },
    CreateRoost {
        name: String,
    },
    #[cfg(feature = "audio")]
    StartCall(Vec<EndpointId>),
    #[cfg(feature = "audio")]
    HangUp,
    #[cfg(feature = "video")]
    StartVideo(Vec<EndpointId>),
    #[cfg(feature = "video")]
    StopVideo,
    Quit,
    Leave {
        code: String,
    },
    Ban {
        roost: EndpointId,
        target: EndpointId,
    },
    Kick {
        roost: EndpointId,
        target: EndpointId,
    },
    SetRole {
        roost: EndpointId,
        target: EndpointId,
        role_index: Option<usize>,
    },
    TransferOwnership {
        roost: EndpointId,
        target: EndpointId,
    },
    Invite {
        roost: EndpointId,
        target: EndpointId,
    },
    AddChannel {
        roost: EndpointId,
        channel: String,
    },
    RemoveChannel {
        roost: EndpointId,
        channel: String,
    },
    RenameRoost {
        roost: EndpointId,
        name: String,
    },
    DeleteMessage {
        roost: EndpointId,
        channel: String,
        id: String,
    },
}

#[derive(Debug)]
pub enum AppEvent {
    ContextStateChanged {
        space: starling::protocol::SpaceId,
        state: crate::ui::ContextState,
    },
    Message {
        flock: String,
        msg: starling::event::ChatMessage,
        /// `true` when the message was delivered as a sealed chirp and the
        /// body was decrypted to its addressee — rendered with a 🔒 to make
        /// the difference between a public broadcast and a private chirp
        /// obvious at a glance.
        private: bool,
    },
    /// A peer published a [`GossipPayload::Profile`] carrying their `crypto_box`
    /// DM public key. The endpoint is already authenticated as the signed
    /// envelope author by the receive loop, so main.rs can store this
    /// `Vec<u8>` directly without verifying a `claimed id` against a name.
    DmKey {
        endpoint: EndpointId,
        dm_pk: Vec<u8>,
    },
    JoinedFlock {
        code: String,
        name: String,
    },
    JoinedRoost {
        code: String,
        name: String,
        channels: Vec<String>,
        perms: starling::roost::perms::PermState,
    },
    RoostUpdate {
        code: String,
        name: String,
        channels: Vec<String>,
        perms: starling::roost::perms::PermState,
    },
    PeerConnected {
        space: starling::protocol::SpaceId,
        id: EndpointId,
    },

    PeerConnectivityHintDown(EndpointId),
    PeerNamed {
        space: starling::protocol::SpaceId,
        id: EndpointId,
        name: String,
        pronouns: String,
    },
    Ticket(EndpointId),
    Error(String),
    #[cfg(feature = "audio")]
    VoiceFrame {
        peer: EndpointId,
        bytes: Vec<u8>,
    },
    #[cfg(feature = "audio")]
    CallStarted(EndpointId),
    #[cfg(feature = "audio")]
    CallEnded(EndpointId),
    #[cfg(feature = "video")]
    LocalVideoFrame(Vec<u8>),
    #[cfg(feature = "video")]
    LocalVideoFailed(String),
    #[cfg(feature = "video")]
    RemoteVideoFrame {
        peer: EndpointId,
        jpeg: Vec<u8>,
    },
    #[cfg(feature = "video")]
    RemoteVideoStopped(EndpointId),
    PeerStatus(EndpointId, starling::event::BirdStatus),
    PresenceLease(starling::presence::SignedPresenceLeaseV1),
    HistoryChunk {
        flock: String,
        messages: Vec<starling::event::ChatMessage>,
    },
    /// A short status-line flash for moderation verdicts / join refusals.
    Notice(String),
}
