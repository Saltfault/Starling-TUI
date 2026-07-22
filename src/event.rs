//! Shared types that cross the UI ↔ network boundary.

use iroh::{EndpointAddr, EndpointId};
use serde::{Deserialize, Serialize};

/// UI → Network: things the user does.
#[allow(dead_code)]
pub enum Command {
    /// Send a chat text message to a specific flock.
    SendText {
        flock: String,
        body: String,
    },
    JoinFlock {
        code: String,
    },
    /// Start a voice call with a peer.
    StartCall(EndpointAddr),
    /// End the current call.
    HangUp,
    StartVideo(EndpointAddr),
    StopVideo,
    /// Shut down the network layer.
    Quit,
}

/// Network → UI: things that happen.
#[allow(dead_code)]
#[derive(Debug)]
pub enum AppEvent {
    /// A chat message was received (or echoed back) in a flock.
    Message { flock: String, msg: ChatMessage },
    /// Successfully joined a flock.
    JoinedFlock { code: String },
    /// A gossip neighbor came online.
    PeerConnected(EndpointId),
    /// A gossip neighbor went offline.
    PeerDisconnected(EndpointId),
    /// A peer announced their display name.
    PeerNamed(EndpointId, String),
    /// The endpoint bound. Carries our own node ID (base-encoded).
    Ticket(String),
    /// A 20 ms Opus voice frame arrived.
    VoiceFrame(Vec<u8>),
    /// A JPEG video frame arrived.
    VideoFrame(Vec<u8>),
    /// A peer changed their presence status.
    PeerStatus(EndpointId, BirdStatus),
    /// A batch of recent chat messages (history sync).
    HistoryChunk(Vec<ChatMessage>),
}

/// A chat message that travels over the gossip layer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Unique message ID (UUID).
    pub id: String,
    /// Sender's display name.
    pub author: String,
    /// Message text.
    pub body: String,
    /// Unix millisecond timestamp.
    pub ts: i64,
}

/// Payload types for gossip messages (encrypted before broadcast).
#[derive(Clone, Serialize, Deserialize)]
pub enum GossipPayload {
    /// A chat message.
    Chat(ChatMessage),
    /// A peer announcing their display name.
    Profile { id: EndpointId, name: String },
    /// A peer announcing their current status.
    Status { id: EndpointId, status: BirdStatus },
}

/// A bird's current presence state.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum BirdStatus {
    Online,
    Idle,
    InCall,
}
