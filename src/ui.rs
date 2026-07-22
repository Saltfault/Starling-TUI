//! UI state and rendering.

use crate::event::{BirdStatus, ChatMessage};
use image::RgbImage;
use iroh::{EndpointAddr, EndpointId};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};
use std::collections::HashMap;

/// A single flock (room) with its message list.
#[derive(Default)]
pub struct FlockView {
    pub code: String,
    pub messages: Vec<ChatMessage>,
    pub unread: usize,
}

#[derive(Default)]
pub struct App {
    /// Display name shown to other peers.
    pub name: String,
    /// All joined flocks.
    pub flocks: Vec<FlockView>,
    /// Index into `flocks` of the currently selected flock.
    pub current_flock: usize,
    /// Text currently being typed in the message box.
    pub input: String,
    /// Connected remote peers, by EndpointId.
    pub peers: Vec<EndpointId>,
    /// Index into `peers` of the currently highlighted peer.
    pub selected_peer: usize,
    /// The room code / invite: the flock opener's full encoded node ID.
    /// This is both displayed in the header and what peers join with.
    pub node_id: Option<String>,
    /// Whether the invite popup is currently displayed.
    pub show_invite: bool,
    /// Whether an active voice call is in progress.
    #[allow(dead_code)]
    pub in_call: bool,
    /// Whether the local microphone is muted.
    #[allow(dead_code)]
    pub muted: bool,
    /// Maps peer EndpointId → display name (from profile announcements).
    pub peer_names: HashMap<EndpointId, String>,
    /// Maps peer EndpointId → current presence status.
    pub peer_status: HashMap<EndpointId, BirdStatus>,
    /// Latest decoded video frame (JPEG → RgbImage).
    #[allow(dead_code)]
    pub video_frame: Option<RgbImage>,
    /// Whether the video pane is currently shown.
    #[allow(dead_code)]
    pub show_video: bool,
}

impl App {
    /// Return a mutable reference to the currently active flock view.
    pub fn active(&mut self) -> Option<&mut FlockView> {
        self.flocks.get_mut(self.current_flock)
    }

    /// Total birds in the room: the local user plus connected peers.
    pub fn bird_count(&self) -> usize {
        self.peers.len() + 1
    }

    /// Cycle selection to the next peer, wrapping around.
    pub fn select_next_peer(&mut self) {
        if !self.peers.is_empty() {
            self.selected_peer = (self.selected_peer + 1) % self.peers.len();
        }
    }

    /// Address of the currently selected peer, if any.
    #[allow(dead_code)]
    pub fn selected_peer_addr(&self) -> Option<EndpointAddr> {
        self.peers
            .get(self.selected_peer)
            .map(|id| EndpointAddr::from(*id))
    }

    /// Get the display name for a peer, or fall back to the short node ID.
    pub fn peer_display_name(&self, id: &EndpointId) -> String {
        self.peer_names
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.fmt_short().to_string())
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .split(f.area());

    // ── Header: color swatches + full node code ───────────────────────
    let header = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(chunks[0]);

    let full_code = app.node_id.as_deref().unwrap_or("");
    let swatch_spans = color_swatches(full_code);
    if !swatch_spans.is_empty() {
        f.render_widget(Line::from(swatch_spans), header[0]);
    }

    f.render_widget(
        Paragraph::new(format!(" {}", full_code)).style(Style::new().fg(Color::DarkGray)),
        header[1],
    );

    // ── Flock rail + Messages + Birds panel ────────────────────────────
    let middle = Layout::horizontal([
        Constraint::Length(14),
        Constraint::Min(1),
        Constraint::Length(24),
    ])
    .split(chunks[1]);

    // ── Flock rail (left) ────────────────────────────────────────────
    let rail_items: Vec<ListItem> = app
        .flocks
        .iter()
        .enumerate()
        .map(|(i, fv)| {
            let mark = if i == app.current_flock { "> " } else { "  " };
            let unread = if fv.unread > 0 {
                format!(" ({})", fv.unread)
            } else {
                String::new()
            };
            let label = &fv.code[..10.min(fv.code.len())];
            ListItem::new(format!("{mark}{label}{unread}"))
        })
        .collect();

    let flock_count = app.flocks.len();
    f.render_widget(
        List::new(rail_items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" flocks ({flock_count}) ")),
        ),
        middle[0],
    );

    // ── Messages (centre) ────────────────────────────────────────────
    let active_msgs: &[ChatMessage] = app
        .flocks
        .get(app.current_flock)
        .map(|fv| fv.messages.as_slice())
        .unwrap_or(&[]);

    let items: Vec<ListItem> = active_msgs
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

    let flock_label = app
        .flocks
        .get(app.current_flock)
        .map(|fv| fv.code.as_str())
        .unwrap_or("");

    // When video is showing, split the message area into messages (60%)
    // and video (40%). Otherwise the messages take the full width.
    #[cfg(feature = "video")]
    let msg_area = if app.show_video {
        let panes = Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(middle[1]);
        if let Some(img) = &app.video_frame {
            let inner = panes[1].inner(Margin {
                vertical: 1,
                horizontal: 1,
            });
            let lines = crate::video::frame_to_lines(img, inner.width, inner.height);
            f.render_widget(
                Block::default().borders(Borders::ALL).title(" video "),
                panes[1],
            );
            f.render_widget(Paragraph::new(lines), inner);
        }
        panes[0]
    } else {
        middle[1]
    };
    #[cfg(not(feature = "video"))]
    let msg_area = middle[1];

    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(format!(
            " {} . {} birds ",
            flock_label,
            app.bird_count()
        ))),
        msg_area,
    );

    // ── Birds panel (right) ──────────────────────────────────────────
    let mut peer_items: Vec<ListItem> = Vec::new();
    peer_items.push(ListItem::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{} (you)", app.name),
            Style::new().fg(Color::Yellow).bold(),
        ),
    ])));

    for (i, id) in app.peers.iter().enumerate() {
        let prefix = if i == app.selected_peer { "> " } else { "  " };
        let glyph = match app.peer_status.get(id) {
            Some(BirdStatus::InCall) => "🔊",
            Some(BirdStatus::Idle) => "◌",
            _ => "●",
        };
        let display = app.peer_display_name(id);
        peer_items.push(ListItem::new(format!("{prefix}{glyph} {display}")));
    }

    f.render_widget(
        List::new(peer_items).block(Block::default().borders(Borders::ALL).title(" birds ")),
        middle[2],
    );

    // ── Status ────────────────────────────────────────────────────────
    let flock_info = if app.flocks.len() > 1 {
        format!(
            " flock {}/{} . Alt+Up/Down to switch",
            app.current_flock + 1,
            app.flocks.len()
        )
    } else {
        String::new()
    };
    let status = if app.in_call {
        format!(
            " in call . {} . Ctrl+K to hang up{}",
            if app.muted { "muted" } else { "live" },
            flock_info
        )
    } else if !flock_info.is_empty() {
        format!(
            " idle . Ctrl+K to call . Tab to cycle . Ctrl+M to mute . i = invite{}",
            flock_info
        )
    } else {
        " idle . Ctrl+K to call . Tab to cycle . Ctrl+M to mute . i = invite".into()
    };
    f.render_widget(
        Paragraph::new(status).style(Style::new().fg(Color::Rgb(111, 174, 157))),
        chunks[2],
    );

    // ── Input ─────────────────────────────────────────────────────────
    f.render_widget(
        Paragraph::new(app.input.as_str())
            .block(Block::default().borders(Borders::ALL).title(" message ")),
        chunks[3],
    );

    // ── Invite popup ──────────────────────────────────────────────────
    if app.show_invite {
        draw_invite_popup(f, app);
    }
}

fn draw_invite_popup(f: &mut Frame, app: &App) {
    let area = f.area();
    f.render_widget(Clear, area);

    let code = app.node_id.as_deref().unwrap_or("connecting...");
    let swatch_spans = color_swatches(code);

    let swatch_line_len = swatch_spans.len();
    let code_len = code.len();
    let content_width = swatch_line_len.max(code_len).max(40) + 4;
    let width = content_width.min(area.width as usize) as u16;
    let height = 12.min(area.height);

    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );

    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default().borders(Borders::ALL).title(" Invite "),
        popup,
    );

    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });

    let chunks = Layout::vertical(vec![Constraint::Length(1); 10]).split(inner);

    f.render_widget(Line::from(swatch_spans), chunks[1]);

    let mid = code.len() / 2;
    let (code1, code2) = if code.len() > 40 {
        let split = code[mid..].find('-').map(|i| mid + i).unwrap_or(mid);
        (&code[..split], &code[split..])
    } else {
        (code, "")
    };

    f.render_widget(
        Paragraph::new(code1).style(Style::new().fg(Color::Green)),
        chunks[3],
    );
    if !code2.is_empty() {
        f.render_widget(
            Paragraph::new(code2).style(Style::new().fg(Color::Green)),
            chunks[4],
        );
    }

    f.render_widget(Paragraph::new("They join with:"), chunks[6]);
    f.render_widget(
        Paragraph::new("  starling join <code>").style(Style::new().fg(Color::Yellow)),
        chunks[7],
    );
    f.render_widget(
        Paragraph::new("  Press i or Esc to close").style(Style::new().fg(Color::DarkGray)),
        chunks[9],
    );
}

/// Build the color-swatch line for a node ID: pairs of ▀▄ glyphs (each
/// rendered full-bright over a dimmed copy) separated by spaces, one per
/// 6-hex color group in the code. Returns an empty vec when no colors
/// have been parsed yet.
///
/// All spans are built from string literals, so they carry a `'static`
/// lifetime and don't borrow from `code`.
fn color_swatches(code: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (r, g, b) in parse_color_code(code) {
        let full = Color::Rgb(r, g, b);
        let half = Color::Rgb(r / 2, g / 2, b / 2);
        spans.push(Span::styled("\u{2580}", Style::new().fg(full).bg(half)));
        spans.push(Span::styled("\u{2584}", Style::new().fg(full).bg(half)));
        spans.push(Span::raw(" "));
    }
    spans
}

/// Parse the 6-hex color groups out of a node ID (e.g. "BIRD-aabbcc-...").
fn parse_color_code(code: &str) -> Vec<(u8, u8, u8)> {
    let mut colors = Vec::new();
    for group in code.split('-') {
        if group == "BIRD" || group.len() != 6 {
            continue;
        }
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&group[0..2], 16),
            u8::from_str_radix(&group[2..4], 16),
            u8::from_str_radix(&group[4..6], 16),
        ) {
            colors.push((r, g, b));
        }
    }
    colors
}
