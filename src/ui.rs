use crate::event::{BirdStatus, ChatMessage};
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
    "Toggle Mute",
    "Toggle Video",
    "Call / Hang Up",
    "Profile",
    "Quit",
];

#[derive(Default)]
pub struct App {
    pub name: String,
    pub flocks: Vec<FlockView>,
    pub roosts: Vec<RoostView>,
    pub current_item: usize,
    pub input: String,
    pub peers: Vec<EndpointId>,
    pub selected_peer: usize,
    pub node_id: Option<String>,
    pub show_invite: bool,
    pub show_create_room: bool,
    pub show_join_room: bool,
    pub join_input: String,
    pub show_join_roost: bool,
    pub join_roost_input: String,
    pub in_call: bool,
    pub muted: bool,
    pub peer_names: HashMap<EndpointId, String>,
    pub peer_status: HashMap<EndpointId, BirdStatus>,
    #[allow(dead_code)]
    pub video_frame: Option<RgbImage>,
    #[allow(dead_code)]
    pub show_video: bool,
    pub show_menu: bool,
    pub menu_selection: usize,
    pub show_create_roost: bool,
    pub create_roost_input: String,
    pub quit_requested: bool,
}

impl App {
    pub fn rail_len(&self) -> usize {
        self.flocks.len() + self.roosts.len()
    }

    pub fn active(&mut self) -> Option<&mut FlockView> {
        if self.current_item < self.flocks.len() {
            self.flocks.get_mut(self.current_item)
        } else {
            None
        }
    }

    pub fn active_code(&self) -> Option<&str> {
        let i = self.current_item;
        if i < self.flocks.len() {
            self.flocks.get(i).map(|fv| fv.code.as_str())
        } else {
            self.roosts
                .get(i - self.flocks.len())
                .map(|rv| rv.code.as_str())
        }
    }

    pub fn active_roost(&self) -> Option<&str> {
        let i = self.current_item;
        if i >= self.flocks.len() {
            self.roosts
                .get(i - self.flocks.len())
                .map(|rv| rv.code.as_str())
        } else {
            None
        }
    }

    pub fn active_roost_name(&self) -> Option<String> {
        let i = self.current_item;
        if i >= self.flocks.len() {
            self.roosts.get(i - self.flocks.len()).map(|rv| {
                if rv.name.is_empty() {
                    rv.code.clone()
                } else {
                    rv.name.clone()
                }
            })
        } else {
            None
        }
    }

    pub fn bird_count(&self) -> usize {
        self.peers.len() + 1
    }

    pub fn select_next_peer(&mut self) {
        if !self.peers.is_empty() {
            self.selected_peer = (self.selected_peer + 1) % self.peers.len();
        }
    }

    #[allow(dead_code)]
    pub fn selected_peer_addr(&self) -> Option<EndpointAddr> {
        self.peers
            .get(self.selected_peer)
            .map(|id| EndpointAddr::from(*id))
    }

    pub fn peer_display_name(&self, id: &EndpointId) -> String {
        self.peer_names
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.fmt_short().to_string())
    }
}

pub fn toolbar_buttons() -> Vec<(&'static str, u16, u16)> {
    let labels = ["Create", "Join", "Menu", "Quit"];
    let mut x = 0u16;
    let mut result = Vec::new();
    for label in labels {
        let width = label.len() as u16 + 2;
        result.push((label, x, width));
        x += width + 1;
    }
    result
}

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .split(f.area());

    let header = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(chunks[0]);

    let active_code = app.active_code().unwrap_or("");
    let swatch_spans = color_swatches(active_code);
    if !swatch_spans.is_empty() {
        f.render_widget(Line::from(swatch_spans), header[0]);
    }

    if !active_code.is_empty() {
        let prefix = if app.active_roost().is_some() {
            " roost: "
        } else {
            " "
        };
        f.render_widget(
            Paragraph::new(format!("{prefix}{active_code}")).style(Style::new().fg(Color::DarkGray)),
            header[1],
        );
    }

    let middle = Layout::horizontal([
        Constraint::Length(14),
        Constraint::Min(1),
        Constraint::Length(24),
    ])
    .split(chunks[1]);

    let rail_split = Layout::vertical([
        Constraint::Ratio(1, 2),
        Constraint::Ratio(1, 2),
    ])
    .split(middle[0]);

    let flock_items: Vec<ListItem> = app
        .flocks
        .iter()
        .enumerate()
        .map(|(i, fv)| {
            let mark = if i == app.current_item { "> " } else { "  " };
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
        List::new(flock_items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" flocks ({flock_count}) ")),
        ),
        rail_split[0],
    );

    let roost_items: Vec<ListItem> = app
        .roosts
        .iter()
        .enumerate()
        .map(|(i, rv)| {
            let idx = app.flocks.len() + i;
            let mark = if idx == app.current_item { "> " } else { "  " };
            let unread = if rv.unread > 0 {
                format!(" ({})", rv.unread)
            } else {
                String::new()
            };
            let label = if rv.name.is_empty() {
                &rv.code[..10.min(rv.code.len())]
            } else {
                &rv.name[..10.min(rv.name.len())]
            };
            ListItem::new(format!("{mark}{label}{unread}"))
        })
        .collect();

    let roost_count = app.roosts.len();
    f.render_widget(
        List::new(roost_items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" roosts ({roost_count}) ")),
        ),
        rail_split[1],
    );

    let is_roost_selected = app.active_roost().is_some();
    if is_roost_selected {
        let display = app.active_roost_name().unwrap_or_default();
        let lines = vec![
            ListItem::new(Line::from(vec![
                Span::styled("Roost: ", Style::new().fg(Color::Rgb(111, 174, 157))),
                Span::raw(&display),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("Channels: ", Style::new().fg(Color::Rgb(111, 174, 157))),
                Span::raw("0"),
            ])),
        ];
        f.render_widget(
            List::new(lines).block(
                Block::default().borders(Borders::ALL).title(format!(
                    " {} . {} birds ",
                    display,
                    app.bird_count()
                )),
            ),
            middle[1],
        );
    } else {
        let active_msgs: &[ChatMessage] = app
            .flocks
            .get(app.current_item)
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

        let flock_label = app.active_code().unwrap_or("");

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
    }

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
            Some(BirdStatus::InCall) => "~",
            Some(BirdStatus::Idle) => "-",
            _ => "o",
        };
        let display = app.peer_display_name(id);
        peer_items.push(ListItem::new(format!("{prefix}{glyph} {display}")));
    }

    f.render_widget(
        List::new(peer_items).block(Block::default().borders(Borders::ALL).title(" birds ")),
        middle[2],
    );

    draw_button_bar(f, app, chunks[2]);

    f.render_widget(
        Paragraph::new(app.input.as_str())
            .block(Block::default().borders(Borders::ALL).title(" message ")),
        chunks[3],
    );

    if app.show_invite {
        draw_invite_popup(f, app);
    } else if app.show_create_room {
        draw_create_room_popup(f, app);
    } else if app.show_join_room {
        draw_join_room_popup(f, app);
    } else if app.show_join_roost {
        draw_join_roost_popup(f, app);
    } else if app.show_menu {
        draw_menu_popup(f, app);
    } else if app.show_create_roost {
        draw_create_roost_popup(f, app);
    }
}

fn status_text(app: &App) -> String {
    let total = app.rail_len();
    if total == 0 {
        String::new()
    } else if app.in_call {
        let nav = if total > 1 {
            format!(" . {}/{}", app.current_item + 1, total)
        } else {
            String::new()
        };
        format!("in call{}", if app.muted { " . muted" } else { " . live" }).to_string() + &nav
    } else {
        String::new()
    }
}

fn draw_button_bar(f: &mut Frame, app: &App, area: Rect) {
    let btns = toolbar_buttons();
    let mut spans = Vec::new();
    for (label, _x, _w) in &btns {
        let fg = Color::Rgb(111, 174, 157);
        spans.push(Span::styled("[", Style::new().fg(fg)));
        spans.push(Span::styled(*label, Style::new().fg(fg)));
        spans.push(Span::styled("]", Style::new().fg(fg)));
        spans.push(Span::raw(" "));
    }

    let status = status_text(app);
    if !status.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(status, Style::new().fg(Color::DarkGray)));
    }

    f.render_widget(Line::from(spans), area);
}

fn draw_menu_popup(f: &mut Frame, app: &App) {
    let area = f.area();
    let width = 28u16.min(area.width);
    let height = (MENU_ITEMS.len() as u16 + 2).min(area.height);
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );

    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default().borders(Borders::ALL).title(" Menu "),
        popup,
    );

    let inner = popup.inner(Margin {
        vertical: 0,
        horizontal: 2,
    });

    let mut items = Vec::new();
    for (i, item) in MENU_ITEMS.iter().enumerate() {
        let selected = i == app.menu_selection;
        let style = if selected {
            Style::new().fg(Color::Yellow).bold()
        } else {
            Style::new().fg(Color::White)
        };
        let prefix = if selected { "> " } else { "  " };
        items.push(ListItem::new(Line::from(Span::styled(
            format!("{prefix}{item}"),
            style,
        ))));
    }

    f.render_widget(
        List::new(items),
        inner,
    );
}

fn draw_create_roost_popup(f: &mut Frame, app: &App) {
    let area = f.area();
    let width = 50.min(area.width);
    let height = 8.min(area.height);
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );

    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default().borders(Borders::ALL).title(" Create Roost "),
        popup,
    );

    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new("Enter the roost name:").style(Style::new().fg(Color::White)),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(format!(" {}_", app.create_roost_input)).style(Style::new().fg(Color::Yellow)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(" Enter = create . Esc = cancel").style(Style::new().fg(Color::DarkGray)),
        chunks[2],
    );
}

fn draw_invite_popup(f: &mut Frame, app: &App) {
    let area = f.area();
    f.render_widget(Clear, area);

    let code = app.active_code().unwrap_or("");
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
        Paragraph::new("  Esc to close").style(Style::new().fg(Color::DarkGray)),
        chunks[9],
    );
}

fn draw_create_room_popup(f: &mut Frame, app: &App) {
    let area = f.area();
    let width = 50.min(area.width);
    let height = 8.min(area.height);
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
            .title(" Create Room "),
        popup,
    );

    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);

    let invite = app.node_id.as_deref().unwrap_or("waiting for endpoint...");
    f.render_widget(
        Paragraph::new(format!(
            "Your invite code: {}\n\nPress Enter to create, Esc to cancel.",
            invite
        ))
        .style(Style::new().fg(Color::White)),
        chunks[0],
    );
}

fn draw_join_room_popup(f: &mut Frame, app: &App) {
    let area = f.area();
    let width = 60.min(area.width);
    let height = 8.min(area.height);
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );

    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default().borders(Borders::ALL).title(" Join Room "),
        popup,
    );

    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new("Enter the room code:").style(Style::new().fg(Color::White)),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(format!(" {}_", app.join_input)).style(Style::new().fg(Color::Yellow)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(" Enter = join . Esc = cancel").style(Style::new().fg(Color::DarkGray)),
        chunks[2],
    );
}

fn draw_join_roost_popup(f: &mut Frame, app: &App) {
    let area = f.area();
    let width = 60.min(area.width);
    let height = 8.min(area.height);
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );

    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default().borders(Borders::ALL).title(" Join Roost "),
        popup,
    );

    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new("Enter the roost code:").style(Style::new().fg(Color::White)),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(format!(" {}_", app.join_roost_input)).style(Style::new().fg(Color::Yellow)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(" Enter = join . Esc = cancel").style(Style::new().fg(Color::DarkGray)),
        chunks[2],
    );
}

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
