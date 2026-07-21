//! Shared types that cross the UI ↔ network boundary.
//!
//! [`Command`] flows from the UI to the network task (user actions).
//! [`AppEvent`] flows from the network task to the UI (things that happen).
//! [`ChatMessage`] is the payload that travels over the gossip layer.

use iroh::EndpointAddr;
use iroh::EndpointId;
use serde::{Deserialize, Serialize};

/// UI → Network: things the user does.
pub enum Command {
    /// Broadcast a text message over gossip.
    SendText(String),
    /// Open a direct QUIC stream to `EndpointAddr` and start sending voice.
    StartCall(EndpointAddr),
    /// Tear down the current call (drops the mic capture stream).
    HangUp,
    /// Shut down the network task and exit.
    Quit,
}

/// Network → UI: things that happen in the network.
#[derive(Debug)]
pub enum AppEvent {
    /// A chat message was received (or echoed back from our own broadcast).
    Message(ChatMessage),
    /// A gossip neighbor came online.
    PeerConnected(EndpointId),
    /// A gossip neighbor went offline.
    PeerDisconnected(EndpointId),
    /// A 20 ms Opus voice frame arrived from a remote peer.
    VoiceFrame(Vec<u8>),
}

/// A chat message that travels over the gossip layer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Unique identifier (UUID v4) so duplicates can be deduped.
    pub id: String,
    /// Display name of the sender.
    pub author: String,
    /// Message body (plain text).
    pub body: String,
    /// Unix-epoch timestamp in milliseconds (UTC).
    pub ts: i64,
}
