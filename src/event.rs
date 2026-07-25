use iroh::EndpointId;

pub enum Command {
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
    Join {
        code: String,
    },
    UpdateProfile {
        name: String,
        input_device: Option<String>,
    },
    #[cfg(feature = "audio")]
    StartCall(EndpointId),
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
    },
    JoinedFlock {
        code: String,
        name: String,
    },
    JoinedRoost {
        code: String,
        name: String,
        channels: Vec<String>,
    },
    #[allow(dead_code)]
    RoostUpdate {
        code: String,
        name: String,
        channels: Vec<String>,
    },
    PeerConnected(EndpointId),

    PeerConnectivityHintDown(EndpointId),
    PeerNamed(EndpointId, String),
    Ticket(EndpointId),
    Error(String),
    #[cfg(feature = "audio")]
    VoiceFrame(Vec<u8>),
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
}
