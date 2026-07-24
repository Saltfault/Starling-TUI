use starling::event::{BirdStatus, ChatMessage};
use image::RgbImage;
use iroh::{EndpointAddr, EndpointId};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};
use std::collections::{HashMap, HashSet};

const GREEN: Color = Color::Rgb(111, 174, 157);
const ORANGE: Color = Color::Rgb(244, 138, 82);
const YELLOW: Color = Color::Rgb(224, 210, 103);
const DIM: Color = Color::Rgb(95, 104, 98);
const CHAN: Color = Color::Rgb(154, 163, 157);
const INVITE: Color = Color::Rgb(78, 201, 143);
#[derive(Default)]
pub struct FlockView {
    pub code: String,
    pub name: String,
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
    "Settings",
    "Quit",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Flock(usize),
    Channel(usize, usize),
}

impl Default for Selection {
    fn default() -> Self {
        Selection::Flock(0)
    }
}

pub struct App {
    pub name: String,
    pub pronouns: String,
    pub flocks: Vec<FlockView>,
    pub roosts: Vec<RoostView>,
    pub selection: Selection,
    pub expanded: HashSet<usize>,
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
    pub text_color: Color,
    pub bg_color: Option<Color>,
    pub border_color: Color,
}

impl Default for App {
    fn default() -> Self {
        Self {
            name: String::new(),
            pronouns: String::new(),
            flocks: Vec::new(),
            roosts: Vec::new(),
            selection: Selection::default(),
            expanded: HashSet::new(),
            input: String::new(),
            peers: Vec::new(),
            selected_peer: 0,
            node_id: None,
            show_invite: false,
            show_create_room: false,
            show_join_room: false,
            join_input: String::new(),
            show_join_roost: false,
            join_roost_input: String::new(),
            in_call: false,
            muted: false,
            peer_names: HashMap::new(),
            peer_status: HashMap::new(),
            video_frame: None,
            show_video: false,
            show_menu: false,
            menu_selection: 0,
            show_create_roost: false,
            create_roost_input: String::new(),
            quit_requested: false,
            text_color: Color::Rgb(207, 214, 210),
            bg_color: None,
            border_color: Color::Rgb(51, 59, 55),
        }
    }
}

impl App {
    pub fn active_code(&self) -> Option<&str> {
        match self.selection {
            Selection::Flock(i) => self.flocks.get(i).map(|f| f.code.as_str()),
            Selection::Channel(ri, _) => self.roosts.get(ri).map(|r| r.code.as_str()),
        }
    }

    pub fn active_messages(&self) -> &[ChatMessage] {
        match self.selection {
            Selection::Flock(i) => self
                .flocks
                .get(i)
                .map(|f| f.messages.as_slice())
                .unwrap_or(&[]),
            Selection::Channel(ri, ci) => self
                .roosts
                .get(ri)
                .and_then(|r| r.channels.get(ci))
                .map(|c| c.messages.as_slice())
                .unwrap_or(&[]),
        }
    }

    pub fn active_title(&self) -> String {
        match self.selection {
            Selection::Flock(i) => self
                .flocks
                .get(i)
                .map(|f| f.code[..16.min(f.code.len())].to_string())
                .unwrap_or_default(),
            Selection::Channel(ri, ci) => self
                .roosts
                .get(ri)
                .map(|r| {
                    let rn = if r.name.is_empty() { &r.code } else { &r.name };
                    let cn = r.channels.get(ci).map(|c| c.name.as_str()).unwrap_or("");
                    format!("{rn} #{cn}")
                })
                .unwrap_or_default(),
        }
    }

    pub fn toggle_expand(&mut self, ri: usize) {
        if !self.expanded.remove(&ri) {
            self.expanded.insert(ri);
        }
    }

    pub fn bird_count(&self) -> usize {
        self.peers.len() + 1
    }

    #[allow(dead_code)]
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

pub fn hex_to_color(hex: &str) -> Option<Color> {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    Some(Color::Rgb(
        u8::from_str_radix(&h[0..2], 16).ok()?,
        u8::from_str_radix(&h[2..4], 16).ok()?,
        u8::from_str_radix(&h[4..6], 16).ok()?,
    ))
}

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();

    if let Some(bg) = app.bg_color {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::raw("")]))
                .style(Style::new().bg(bg)),
            area,
        );
    }

    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .split(area);

    draw_header(f, app, chunks[0]);

    let middle = Layout::horizontal([
        Constraint::Length(26),
        Constraint::Min(1),
        Constraint::Length(24),
    ])
    .split(chunks[1]);

    let rail = Layout::vertical([Constraint::Percentage(33), Constraint::Min(1)]).split(middle[0]);
    draw_flocks(f, app, rail[0]);
    draw_roosts(f, app, rail[1]);

    draw_messages(f, app, middle[1]);
    draw_birds(f, app, middle[2]);

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
    } else if app.show_create_roost {
        draw_create_roost_popup(f, app);
    } else if app.show_menu {
        draw_menu_popup(f, app);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let header = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
    let code = app.active_code().unwrap_or("");
    let swatches = color_swatches(code);
    if !swatches.is_empty() {
        f.render_widget(Line::from(swatches), header[0]);
    }
    if !code.is_empty() {
        f.render_widget(
            Paragraph::new(format!(" {code}")).style(Style::new().fg(DIM)),
            header[1],
        );
    }
}

fn draw_flocks(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .flocks
        .iter()
        .enumerate()
        .map(|(i, fv)| {
            let sel = app.selection == Selection::Flock(i);
            let mark = if sel { "> " } else { "  " };
            let unread = if fv.unread > 0 {
                format!(" ({})", fv.unread)
            } else {
                String::new()
            };
            let dot = flock_dot(&fv.code);
            let label = &fv.code[..12.min(fv.code.len())];
            ListItem::new(Line::from(vec![
                Span::styled(mark, Style::new().fg(YELLOW)),
                Span::styled("\u{25AE} ", Style::new().fg(dot)),
                Span::styled(
                    label.to_string(),
                    Style::new().fg(if sel { YELLOW } else { app.text_color }),
                ),
                Span::styled(unread, Style::new().fg(YELLOW)),
            ]))
        })
        .collect();

    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(app.border_color))
                .title(Span::styled(
                    format!(" flocks ({}) ", app.flocks.len()),
                    Style::new().fg(GREEN),
                )),
        ),
        area,
    );
}

fn draw_roosts(f: &mut Frame, app: &App, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();
    for (i, rv) in app.roosts.iter().enumerate() {
        let expanded = app.expanded.contains(&i);
        let head_sel = matches!(app.selection, Selection::Channel(ri, _) if ri == i);
        let caret = if expanded { "\u{25BE} " } else { "\u{25B8} " };
        let unread = if rv.unread > 0 {
            format!(" ({})", rv.unread)
        } else {
            String::new()
        };
        let dot = flock_dot(&rv.code);
        let name = if rv.name.is_empty() {
            &rv.code[..12.min(rv.code.len())]
        } else {
            &rv.name[..]
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(caret, Style::new().fg(DIM)),
            Span::styled("\u{25AE} ", Style::new().fg(dot)),
            Span::styled(
                name.to_string(),
                Style::new()
                    .fg(if head_sel { YELLOW } else { app.text_color })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(unread, Style::new().fg(YELLOW)),
        ])));

        if expanded {
            for (ci, ch) in rv.channels.iter().enumerate() {
                let sel = app.selection == Selection::Channel(i, ci);
                let cu = if ch.unread > 0 {
                    format!(" ({})", ch.unread)
                } else {
                    String::new()
                };
                items.push(ListItem::new(Line::from(vec![
                    Span::raw("    "),
                    Span::styled("#", Style::new().fg(if sel { YELLOW } else { DIM })),
                    Span::styled(
                        format!(" {}", ch.name),
                        Style::new().fg(if sel { YELLOW } else { CHAN }),
                    ),
                    Span::styled(cu, Style::new().fg(YELLOW)),
                ])));
            }
        }
    }

    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(app.border_color))
                .title(Span::styled(
                    format!(" roosts ({}) ", app.roosts.len()),
                    Style::new().fg(GREEN),
                )),
        ),
        area,
    );
}

fn draw_messages(f: &mut Frame, app: &App, area: Rect) {
    #[cfg(feature = "video")]
    let area = if app.show_video {
        let panes = Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);
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
        area
    };

    let items: Vec<ListItem> = app
        .active_messages()
        .iter()
        .map(|m| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{}: ", m.author),
                    Style::new().fg(ORANGE).add_modifier(Modifier::BOLD),
                ),
                Span::styled(m.body.clone(), Style::new().fg(app.text_color)),
            ]))
        })
        .collect();

    let title = format!(" {} . {} birds ", app.active_title(), app.bird_count());
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(app.border_color))
                .title(Span::styled(title, Style::new().fg(app.text_color))),
        ),
        area,
    );
}

fn draw_birds(f: &mut Frame, app: &App, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();
    items.push(ListItem::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{} (you)", app.name),
            Style::new().fg(YELLOW).add_modifier(Modifier::BOLD),
        ),
    ])));

    for (i, id) in app.peers.iter().enumerate() {
        let sel = i == app.selected_peer;
        let mark = if sel { "> " } else { "  " };
        let (glyph, gc) = match app.peer_status.get(id) {
            Some(BirdStatus::InCall) => ("~", ORANGE),
            Some(BirdStatus::Idle) => ("-", DIM),
            _ => ("o", GREEN),
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(mark, Style::new().fg(YELLOW)),
            Span::styled(format!("{glyph} "), Style::new().fg(gc)),
            Span::styled(
                app.peer_display_name(id),
                Style::new().fg(if sel { YELLOW } else { app.text_color }),
            ),
        ])));
    }

    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(app.border_color))
                .title(Span::styled(" birds ", Style::new().fg(GREEN))),
        ),
        area,
    );
}

fn status_text(app: &App) -> String {
    if app.in_call {
        format!("in call{}", if app.muted { " . muted" } else { " . live" })
    } else {
        String::new()
    }
}

fn draw_button_bar(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    for (label, _x, _w) in toolbar_buttons() {
        spans.push(Span::styled("[", Style::new().fg(GREEN)));
        spans.push(Span::styled(label, Style::new().fg(GREEN)));
        spans.push(Span::styled("]", Style::new().fg(GREEN)));
        spans.push(Span::raw(" "));
    }
    let status = status_text(app);
    if !status.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(status, Style::new().fg(DIM)));
    }
    f.render_widget(Line::from(spans), area);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    )
}

fn draw_menu_popup(f: &mut Frame, app: &App) {
    let popup = centered(f.area(), 28, MENU_ITEMS.len() as u16 + 2);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(app.border_color))
            .title(Span::styled(" Menu ", Style::new().fg(GREEN))),
        popup,
    );
    let inner = popup.inner(Margin {
        vertical: 0,
        horizontal: 2,
    });
    let items: Vec<ListItem> = MENU_ITEMS
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let sel = i == app.menu_selection;
            let style = if sel {
                Style::new().fg(YELLOW).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(app.text_color)
            };
            let prefix = if sel { "> " } else { "  " };
            ListItem::new(Line::from(Span::styled(format!("{prefix}{item}"), style)))
        })
        .collect();
    f.render_widget(List::new(items), inner);
}

fn draw_create_room_popup(f: &mut Frame, app: &App) {
    let popup = centered(f.area(), 56, 8);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(app.border_color))
            .title(Span::styled(" Create Room ", Style::new().fg(GREEN))),
        popup,
    );
    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(inner);
    let invite = app.node_id.as_deref().unwrap_or("waiting for endpoint...");
    f.render_widget(
        Paragraph::new("Your invite code:").style(Style::new().fg(app.text_color)),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(invite).style(Style::new().fg(INVITE)),
        rows[1],
    );
    f.render_widget(
        Paragraph::new("Press Enter to create, Esc to cancel.").style(Style::new().fg(DIM)),
        rows[3],
    );
}

fn draw_join_room_popup(f: &mut Frame, app: &App) {
    draw_input_popup(
        f,
        " Join Room ",
        "Enter the room code:",
        &app.join_input,
        " Enter = join . Esc = cancel",
        app.text_color,
        app.border_color,
    );
}

fn draw_join_roost_popup(f: &mut Frame, app: &App) {
    draw_input_popup(
        f,
        " Join Roost ",
        "Enter the roost code:",
        &app.join_roost_input,
        " Enter = join . Esc = cancel",
        app.text_color,
        app.border_color,
    );
}

fn draw_create_roost_popup(f: &mut Frame, app: &App) {
    draw_input_popup(
        f,
        " Create Roost ",
        "Enter the roost name:",
        &app.create_roost_input,
        " Enter = create . Esc = cancel",
        app.text_color,
        app.border_color,
    );
}

fn draw_input_popup(f: &mut Frame, title: &str, prompt: &str, value: &str, hint: &str, text_color: Color, border_color: Color) {
    let popup = centered(f.area(), 60, 8);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(border_color))
            .title(Span::styled(title.to_string(), Style::new().fg(GREEN))),
        popup,
    );
    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(inner);
    f.render_widget(Paragraph::new(prompt).style(Style::new().fg(text_color)), rows[0]);
    f.render_widget(
        Paragraph::new(format!(" {value}_")).style(Style::new().fg(YELLOW)),
        rows[1],
    );
    f.render_widget(Paragraph::new(hint).style(Style::new().fg(DIM)), rows[2]);
}

fn draw_invite_popup(f: &mut Frame, app: &App) {
    let area = f.area();
    let code = app.active_code().unwrap_or("");
    let swatches = color_swatches(code);
    let content_width = swatches.len().max(code.len()).max(40) + 6;
    let popup = centered(area, content_width as u16, 12);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(app.border_color))
            .title(Span::styled(" Invite ", Style::new().fg(GREEN))),
        popup,
    );
    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let rows = Layout::vertical(vec![Constraint::Length(1); 10]).split(inner);

    f.render_widget(Line::from(swatches), rows[1]);

    let (c1, c2) = if code.len() > 40 {
        let mid = code.len() / 2;
        let split = code[mid..].find('-').map(|i| mid + i).unwrap_or(mid);
        (&code[..split], &code[split..])
    } else {
        (code, "")
    };
    f.render_widget(Paragraph::new(c1).style(Style::new().fg(INVITE)), rows[3]);
    if !c2.is_empty() {
        f.render_widget(Paragraph::new(c2).style(Style::new().fg(INVITE)), rows[4]);
    }
    f.render_widget(Paragraph::new("They join with:").style(Style::new().fg(app.text_color)), rows[6]);
    f.render_widget(
        Paragraph::new("  starling join <code>").style(Style::new().fg(YELLOW)),
        rows[7],
    );
    f.render_widget(
        Paragraph::new("  Esc to close").style(Style::new().fg(DIM)),
        rows[9],
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

fn flock_dot(code: &str) -> Color {
    parse_color_code(code)
        .first()
        .map(|&(r, g, b)| Color::Rgb(r, g, b))
        .unwrap_or(GREEN)
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
