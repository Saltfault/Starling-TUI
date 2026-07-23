use iroh::{EndpointAddr, EndpointId};

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
    StartCall(EndpointAddr),
    HangUp,
    StartVideo(EndpointAddr),
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
    VoiceFrame(Vec<u8>),
    VideoFrame(Vec<u8>),
    PeerStatus(EndpointId, starling::event::BirdStatus),
    HistoryChunk(Vec<starling::event::ChatMessage>),
}
