#[cfg(any(feature = "audio", feature = "video"))]
use iroh::EndpointAddr;
use iroh::EndpointId;

pub enum Command {
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
    StartCall(EndpointAddr),
    #[cfg(feature = "audio")]
    HangUp,
    #[cfg(feature = "video")]
    StartVideo(EndpointAddr),
    #[cfg(feature = "video")]
    StopVideo,
    Quit,
}

#[derive(Debug)]
pub enum AppEvent {
    Message {
        flock: String,
        msg: starling::event::ChatMessage,
    },
    JoinedFlock {
        code: String,
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
    PeerDisconnected(EndpointId),
    PeerNamed(EndpointId, String),
    Ticket(EndpointId),
    Error(String),
    #[cfg(feature = "audio")]
    VoiceFrame(Vec<u8>),
    #[cfg(feature = "video")]
    VideoFrame(Vec<u8>),
    PeerStatus(EndpointId, starling::event::BirdStatus),
    HistoryChunk {
        flock: String,
        messages: Vec<starling::event::ChatMessage>,
    },
}
