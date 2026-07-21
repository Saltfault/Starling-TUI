//! UI state and rendering. The [`App`] struct holds all mutable state that
//! the terminal loop reads and writes. The [`draw`] function renders it.
//!
//! The app has two phases:
//!
//! 1. **Name entry** — a centered popup asks for the bird's display name.
//!    Rendered when `app.name` is empty.
//! 2. **Chat** — the full chat UI with messages, a birds panel (peer list),
//!    call status, and text input. Rendered once `app.name` is set.
//!
//! This module is purely presentational — it never touches the network or
//! audio directly. State changes happen in `main.rs` in response to keyboard
//! input or [`AppEvent`](crate::event::AppEvent)s.

use crate::event::ChatMessage;
use iroh::{EndpointAddr, EndpointId};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

/// All mutable UI state. Updated by `main.rs` in response to keyboard input
/// and network events; read by [`draw`] every frame.
#[derive(Default)]
pub struct App {
    // ── Name entry phase ──────────────────────────────────────────────
    /// The bird's confirmed display name. When empty, the UI shows the
    /// name-entry popup instead of the chat.
    pub name: String,
    /// Buffer for the name-entry input field.
    pub name_input: String,

    // ── Chat phase ────────────────────────────────────────────────────
    /// Chat messages received (and echoed from our own broadcasts).
    pub messages: Vec<ChatMessage>,
    /// Current text input buffer (for chat messages, not name entry).
    pub input: String,
    /// Connected peer IDs (from gossip neighbor-up/down events).
    pub peers: Vec<EndpointId>,
    /// Index into `peers` for the currently selected peer (for calling).
    pub selected_peer: usize,
    /// Room code shown in the header (e.g. "BIRD324524").
    pub invite: Option<String>,
    /// Whether we are currently in a call.
    pub in_call: bool,
    /// Whether the mic is muted (display state; the actual gate is an
    /// `Arc<AtomicBool>` in `main.rs`).
    pub muted: bool,
}

impl App {
    /// Number of connected birds.
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Cycle the selected peer to the next one in the list (wraps around).
    /// Does nothing if no peers are connected.
    pub fn select_next_peer(&mut self) {
        if !self.peers.is_empty() {
            self.selected_peer = (self.selected_peer + 1) % self.peers.len();
        }
    }

    /// Return the [`EndpointAddr`] of the currently selected peer, if any.
    ///
    /// The `EndpointAddr` is constructed from just the `EndpointId`; iroh's
    /// discovery system resolves the actual address when we connect.
    pub fn selected_peer_addr(&self) -> Option<EndpointAddr> {
        self.peers
            .get(self.selected_peer)
            .map(|id| EndpointAddr::from(*id))
    }
}

// ── Top-level draw dispatcher ───────────────────────────────────────────

/// Render the app state to the terminal.
///
/// Shows the name-entry popup if `app.name` is empty, otherwise shows the
/// full chat UI.
pub fn draw(f: &mut Frame, app: &App) {
    if app.name.is_empty() {
        draw_name_entry(f, app);
    } else {
        draw_chat(f, app);
    }
}

// ── Name entry ──────────────────────────────────────────────────────────

/// Render the name-entry popup — a centered box asking the bird for their
/// display name before joining the murmuration.
fn draw_name_entry(f: &mut Frame, app: &App) {
    let area = f.area();

    // Clear the whole screen first.
    f.render_widget(Clear, area);

    // Center a popup box.
    let width = 48.min(area.width);
    let height = 9.min(area.height);
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );

    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(" Welcome to Starling "),
        popup,
    );

    // Layout inside the popup.
    let inner = Layout::vertical([
        Constraint::Length(1), // blank
        Constraint::Length(1), // subtitle
        Constraint::Length(1), // blank
        Constraint::Length(1), // name input
        Constraint::Length(1), // blank
        Constraint::Length(1), // hint
    ])
    .margin(1)
    .split(popup);

    f.render_widget(Paragraph::new("Join the murmuration."), inner[1]);

    f.render_widget(
        Paragraph::new(format!(" Name: {}_", app.name_input)).style(Style::new().fg(Color::Yellow)),
        inner[3],
    );

    f.render_widget(
        Paragraph::new(" Press Enter to continue ").style(Style::new().fg(Color::DarkGray)),
        inner[5],
    );
}

// ── Chat UI ─────────────────────────────────────────────────────────────

/// Render the full chat UI: header, messages + birds panel, status, input.
fn draw_chat(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header: room code
        Constraint::Min(1),    // messages + birds panel
        Constraint::Length(1), // call status
        Constraint::Length(3), // input
    ])
    .split(f.area());

    // ── Header: room code ──────────────────────────────────────────────
    let invite = app.invite.as_deref().unwrap_or("waiting for endpoint...");
    f.render_widget(
        Paragraph::new(format!(" flock: {} ", invite)).style(Style::new().fg(Color::DarkGray)),
        chunks[0],
    );

    // ── Messages + Birds panel (horizontal split) ──────────────────────
    let middle = Layout::horizontal([
        Constraint::Min(1),     // messages
        Constraint::Length(24), // birds panel
    ])
    .split(chunks[1]);

    // Messages list
    let items: Vec<ListItem> = app
        .messages
        .iter()
        .map(|m| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{}: ", m.author),
                    Style::new().fg(Color::Rgb(244, 138, 82)).bold(),
                ),
                Span::raw(&m.body),
            ]))
        })
        .collect();

    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" #global · {} birds ", app.peer_count())),
        ),
        middle[0],
    );

    // Birds panel — shows all connected peers, with the selected one highlighted
    let peer_items: Vec<ListItem> = app
        .peers
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let prefix = if i == app.selected_peer { "▶ " } else { "  " };
            ListItem::new(format!("{prefix}{}", id.fmt_short()))
        })
        .collect();

    let peer_list = if peer_items.is_empty() {
        List::new(vec![ListItem::new("  no birds yet")])
            .block(Block::default().borders(Borders::ALL).title(" birds "))
    } else {
        List::new(peer_items).block(Block::default().borders(Borders::ALL).title(" birds "))
    };
    f.render_widget(peer_list, middle[1]);

    // ── Status: call state + keybindings ───────────────────────────────
    let status = if app.in_call {
        format!(
            "🔊 in call · {} · Ctrl+K to hang up",
            if app.muted { "muted" } else { "live" }
        )
    } else {
        "○ idle · Ctrl+K to call · Tab to cycle · Ctrl+M to mute".into()
    };
    f.render_widget(
        Paragraph::new(status).style(Style::new().fg(Color::Rgb(111, 174, 157))),
        chunks[2],
    );

    // ── Input ──────────────────────────────────────────────────────────
    f.render_widget(
        Paragraph::new(app.input.as_str())
            .block(Block::default().borders(Borders::ALL).title(" message ")),
        chunks[3],
    );
}
