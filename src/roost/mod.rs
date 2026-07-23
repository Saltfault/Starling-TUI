//! Roost types and protocol helpers.
//!
//! This module defines the structs and protocols that the TUI uses to
//! interact with a Starling roost server. The actual roost server
//! (create, open, close, destroy, doctor) lives in `starling-server`.
//!
//! The TUI never starts a roost — it only reads roost state broadcast
//! on the control channel so it can render the channel rail.

use serde::{Deserialize, Serialize};

/// Metadata broadcast by a roost on the control channel so clients
/// can render the flock rail (the channel list on the left).
///
/// A joining bird receives this encrypted message when they subscribe
/// to the control topic. Later phases will sign this payload so
/// clients can verify it came from the roost's identity key.
#[derive(Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct RoostState {
    /// Human-friendly display name for this roost.
    pub name: String,
    /// Ordered list of channel names (e.g. `["general", "builds", ...]`).
    pub channels: Vec<String>,
}
