use iroh::EndpointId;
#[cfg(any(feature = "audio", feature = "video"))]
use iroh::EndpointAddr;

pub enum Command {
    SendText {
        flock: String,
        body: String,
    },
    JoinFlock {
        code: String,
    },
    JoinRoost {
        code: String,
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
    Message { flock: String, msg: starling::event::ChatMessage },
    JoinedFlock { code: String },
    JoinedRoost { code: String, name: String, channels: Vec<String> },
    RoostUpdate { code: String, name: String, channels: Vec<String> },
    PeerConnected(EndpointId),
    PeerDisconnected(EndpointId),
    PeerNamed(EndpointId, String),
    Ticket(String),
    #[cfg(feature = "audio")]
    VoiceFrame(Vec<u8>),
    #[cfg(feature = "video")]
    VideoFrame(Vec<u8>),
    PeerStatus(EndpointId, starling::event::BirdStatus),
    HistoryChunk(Vec<starling::event::ChatMessage>),
}
