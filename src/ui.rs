use starling::event::{BirdStatus, ChatMessage};
use image::RgbImage;
use iroh::{EndpointAddr, EndpointId};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};
use std::collections::HashMap;

#[derive(Default)]
pub struct FlockView {
    pub code: String,
    pub messages: Vec<ChatMessage>,
    pub unread: usize,
}

#[derive(Default)]
pub struct RoostView {
    pub code: String,
    pub name: String,
    pub channels: Vec<FlockView>,
    pub unread: usize,
}

pub const MENU_ITEMS: &[&str] = &[
    "Create Room",
    "Join Flock",
    "Join Roost",
    "Create Roost",
    "Invite",
    "Next Peer",
    "Toggle Mute",
    "Toggle Video",
    "Call / Hang Up",
    "Profile",
    "Quit",
];

#[derive(Default)]
pub struct App {
    pub flocks: Vec<FlockView>,
    pub roosts: Vec<RoostView>,
    pub peer_names: HashMap<EndpointId, String>,
    pub peer_status: HashMap<EndpointId, BirdStatus>,
    pub peers: Vec<EndpointId>,
    pub selected_peer: usize,
    pub input: String,
    pub join_input: String,
    pub join_roost_input: String,
    pub create_roost_input: String,
    pub show_join_room: bool,
    pub show_join_roost: bool,
    pub show_create_room: bool,
    pub show_create_roost: bool,
    pub show_invite: bool,
    pub show_menu: bool,
    pub show_video: bool,
    pub menu_selection: usize,
    pub current_item: usize,
    pub node_id: Option<String>,
    pub video_frame: Option<RgbImage>,
    pub name: String,
    #[cfg(feature = "audio")]
    pub in_call: bool,
    #[cfg(feature = "audio")]
    pub muted: bool,
    pub quit_requested: bool,
}

impl App {
    pub fn active(&self) -> Option<&FlockView> {
        if self.current_item < self.flocks.len() {
            Some(&self.flocks[self.current_item])
        } else {
            let roost_idx = self.current_item - self.flocks.len();
            self.roosts.get(roost_idx).and_then(|r| r.channels.first())
        }
    }

    pub fn active_code(&self) -> Option<&str> {
        self.active().map(|fv| fv.code.as_str())
    }

    pub fn rail_len(&self) -> usize {
        self.flocks.len() + self.roosts.len().min(1)
    }

    pub fn select_next_peer(&mut self) {
        if !self.peers.is_empty() {
            self.selected_peer = (self.selected_peer + 1) % self.peers.len();
        }
    }

    pub fn selected_peer_addr(&self) -> Option<EndpointAddr> {
        self.peers.get(self.selected_peer).map(|id| EndpointAddr::from(*id))
    }
}

pub fn toolbar_buttons() -> Vec<(&'static str, u16, u16)> {
    let labels = ["Create", "Join", "Menu", "Quit"];
    let widths: [u16; 4] = [8, 6, 6, 6];
    let mut x = 1u16;
    let mut buttons = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        buttons.push((*label, x, widths[i]));
        x += widths[i] + 1;
    }
    buttons
}

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .split(area);

    draw_header(f, chunks[0], app);
    draw_middle(f, chunks[1], app);
    draw_button_bar(f, chunks[2]);
    draw_input(f, chunks[3], app);

    if app.show_menu {
        draw_menu_popup(f, area, app);
    }
    if app.show_invite {
        draw_invite_popup(f, area, app);
    }
    if app.show_create_room {
        draw_create_room_popup(f, area, app);
    }
    if app.show_join_room {
        draw_join_popup(f, area, "Join Flock", &app.join_input);
    }
    if app.show_join_roost {
        draw_join_popup(f, area, "Join Roost", &app.join_roost_input);
    }
    if app.show_create_roost {
        draw_create_roost_popup(f, area, app);
    }
    if app.show_video {
        draw_video_modal(f, area, app);
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let title = " Starling TUI ";
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let node_info = app.node_id.as_deref().unwrap_or("connecting…");
    let peer_count = app.peers.len();
    let status = format!(" peers: {peer_count} | id: {node_info}");
    let header = format!("{title}  {version}  |{status}");
    f.render_widget(
        Paragraph::new(header).style(Style::new().fg(Color::Cyan).bold()),
        area,
    );
}

fn draw_middle(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::horizontal([Constraint::Length(14), Constraint::Min(1)]).split(area);
    draw_rail(f, chunks[0], app);
    draw_chat(f, chunks[1], app);
}

fn draw_rail(f: &mut Frame, area: Rect, app: &App) {
    let rail_h = area.height.saturating_sub(2);
    let flocks_h = rail_h / 2;
    let roosts_h = rail_h - flocks_h;

    let rail_chunks = Layout::vertical([
        Constraint::Length(flocks_h + 1),
        Constraint::Length(roosts_h + 1),
    ])
    .split(area);

    // Flocks header + list
    let flocks_block = Block::default()
        .borders(Borders::TOP)
        .title(" Flocks ");
    let flocks_inner = flocks_block.inner(rail_chunks[0]);
    f.render_widget(flocks_block, rail_chunks[0]);

    let flock_items: Vec<ListItem> = app
        .flocks
        .iter()
        .enumerate()
        .map(|(i, fv)| {
            let prefix = if i == app.current_item { ">" } else { " " };
            let unread = if fv.unread > 0 {
                format!(" ({})", fv.unread)
            } else {
                String::new()
            };
            ListItem::new(format!("{prefix} {} {}", fv.code, unread))
        })
        .collect();
    f.render_widget(
        List::new(flock_items).style(Style::new().fg(Color::White)),
        flocks_inner,
    );

    // Roosts header + list
    let roosts_block = Block::default()
        .borders(Borders::TOP)
        .title(" Roosts ");
    let roosts_inner = roosts_block.inner(rail_chunks[1]);
    f.render_widget(roosts_block, rail_chunks[1]);

    let roost_items: Vec<ListItem> = app
        .roosts
        .iter()
        .enumerate()
        .map(|(i, rv)| {
            let idx = app.flocks.len() + i;
            let prefix = if idx == app.current_item { ">" } else { " " };
            let unread = if rv.unread > 0 {
                format!(" ({})", rv.unread)
            } else {
                String::new()
            };
            ListItem::new(format!("{prefix} {}{}", rv.name, unread))
        })
        .collect();
    f.render_widget(
        List::new(roost_items).style(Style::new().fg(Color::White)),
        roosts_inner,
    );
}

fn draw_chat(f: &mut Frame, area: Rect, app: &App) {
    if let Some(fv) = app.active() {
        let messages: Vec<ListItem> = fv
            .messages
            .iter()
            .rev()
            .take(area.height as usize)
            .rev()
            .map(|m| {
                let author_style = if m.author == app.name {
                    Style::new().fg(Color::Green).bold()
                } else {
                    Style::new().fg(Color::Yellow).bold()
                };
                let ts = chrono::DateTime::from_timestamp_millis(m.ts)
                    .map(|dt| dt.format("%H:%M").to_string())
                    .unwrap_or_default();
                let line = format!("{ts} {:<12} {}", m.author, m.body);
                ListItem::new(line).style(author_style)
            })
            .collect();
        f.render_widget(List::new(messages), area);
    }
}

fn draw_button_bar(f: &mut Frame, area: Rect) {
    let btns = toolbar_buttons();
    let mut spans = Vec::new();
    for (i, (label, _, _)) in btns.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            format!("[{label}]"),
            Style::new().fg(Color::Cyan).bold(),
        ));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().bg(Color::DarkGray)),
        area,
    );
}

fn draw_input(f: &mut Frame, area: Rect, app: &App) {
    let input_style = if app.show_create_room || app.show_join_room || app.show_join_roost || app.show_create_roost {
        Style::new().fg(Color::DarkGray)
    } else {
        Style::default()
    };
    let cursor = if app.input.is_empty() {
        " Type a message…"
    } else {
        ""
    };
    f.render_widget(
        Paragraph::new(format!(" {}{}_", app.input, cursor)).style(input_style),
        area,
    );
}

fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let w = area.width * percent_x / 100;
    let h = area.height * percent_y / 100;
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w.min(area.width),
        h.min(area.height),
    )
}

fn draw_menu_popup(f: &mut Frame, area: Rect, app: &App) {
    let popup = centered_rect(area, 40, 60);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default().borders(Borders::ALL).title(" Menu "),
        popup,
    );

    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let items: Vec<ListItem> = MENU_ITEMS
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let style = if i == app.menu_selection {
                Style::new().fg(Color::Black).bg(Color::White)
            } else {
                Style::new().fg(Color::White)
            };
            ListItem::new(*text).style(style)
        })
        .collect();
    f.render_widget(List::new(items), inner);
}

fn draw_invite_popup(f: &mut Frame, area: Rect, app: &App) {
    let popup = centered_rect(area, 50, 20);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default().borders(Borders::ALL).title(" Invite "),
        popup,
    );

    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let invite_code = app.active_code().unwrap_or("");
    let lines = vec![
        Line::raw("Share this code with a friend:"),
        Line::raw(""),
        Line::styled(invite_code, Style::new().fg(Color::Cyan).bold()),
        Line::raw(""),
        Line::raw("They join with: starling join <code>"),
        Line::raw(""),
        Line::styled(" Esc = close ", Style::new().fg(Color::DarkGray)),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_create_room_popup(f: &mut Frame, area: Rect, app: &App) {
    let popup = centered_rect(area, 50, 20);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(" Create Room "),
        popup,
    );

    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let code = app.node_id.as_deref().unwrap_or("(no node id)");
    let lines = vec![
        Line::raw("Create a room with your node ID?"),
        Line::raw(""),
        Line::styled(format!(" node: {code}"), Style::new().fg(Color::Cyan)),
        Line::raw(""),
        Line::raw(" Enter = create . Esc = cancel "),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_join_popup(f: &mut Frame, area: Rect, title: &str, input: &str) {
    let popup = centered_rect(area, 50, 25);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default().borders(Borders::ALL).title(title),
        popup,
    );

    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let lines = vec![
        Line::raw("Enter the invite code:"),
        Line::raw(""),
        Line::styled(format!(" {}_", input), Style::new().fg(Color::Yellow)),
        Line::raw(""),
        Line::raw(" Enter = join . Esc = cancel "),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_create_roost_popup(f: &mut Frame, area: Rect, app: &App) {
    let popup = centered_rect(area, 50, 25);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(" Create Roost "),
        popup,
    );

    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let lines = vec![
        Line::raw("Name your roost:"),
        Line::raw(""),
        Line::styled(
            format!(" {}_", app.create_roost_input),
            Style::new().fg(Color::Yellow),
        ),
        Line::raw(""),
        Line::raw(" Enter = create . Esc = cancel "),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_video_modal(f: &mut Frame, area: Rect, app: &App) {
    let popup = centered_rect(area, 60, 50);
    f.render_widget(Clear, popup);
    let title = format!(
        " Video — {} ",
        app.selected_peer_addr()
            .map(|a| starling::net::encode_node_id(&a.id))
            .as_deref()
            .unwrap_or("no peer")
    );
    f.render_widget(
        Block::default().borders(Borders::ALL).title(title),
        popup,
    );

    if let Some(img) = &app.video_frame {
        let inner = popup.inner(Margin {
            vertical: 1,
            horizontal: 2,
        });
        let lines = frame_to_lines(img, inner.width, inner.height);
        f.render_widget(Paragraph::new(lines), inner);
    }
}

fn frame_to_lines(img: &image::RgbImage, cols: u16, rows: u16) -> Vec<Line<'static>> {
    let small =
        image::imageops::resize(img, cols as u32, (rows * 2) as u32, image::imageops::FilterType::Triangle);
    (0..rows)
        .map(|cy| {
            Line::from(
                (0..cols)
                    .map(|cx| {
                        let top = small.get_pixel(cx as u32, (cy * 2) as u32);
                        let bot = small.get_pixel(cx as u32, (cy * 2 + 1) as u32);
                        Span::styled(
                            "\u{2580}",
                            Style::new()
                                .fg(Color::Rgb(top[0], top[1], top[2]))
                                .bg(Color::Rgb(bot[0], bot[1], bot[2])),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}
