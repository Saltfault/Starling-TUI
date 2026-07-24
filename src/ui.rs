use image::RgbImage;
use iroh::{EndpointAddr, EndpointId};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};
use starling::event::{BirdStatus, ChatMessage};
use std::collections::{HashMap, HashSet};

const DEFAULT_ACCENT: Color = Color::Rgb(111, 174, 157);
const DEFAULT_AUTHOR: Color = Color::Rgb(244, 138, 82);
const DEFAULT_SELECTION: Color = Color::Rgb(224, 210, 103);
const DEFAULT_DIM: Color = Color::Rgb(95, 104, 98);
const DEFAULT_CHANNEL: Color = Color::Rgb(154, 163, 157);
const DEFAULT_INVITE: Color = Color::Rgb(78, 201, 143);

pub struct Palette {
    pub text: Color,
    pub background: Option<Color>,
    pub border: Color,
    pub accent: Color,
    pub author: Color,
    pub selection: Color,
    pub dim: Color,
    pub channel: Color,
    pub invite: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            text: Color::Rgb(207, 214, 210),
            background: None,
            border: Color::Rgb(51, 59, 55),
            accent: DEFAULT_ACCENT,
            author: DEFAULT_AUTHOR,
            selection: DEFAULT_SELECTION,
            dim: DEFAULT_DIM,
            channel: DEFAULT_CHANNEL,
            invite: DEFAULT_INVITE,
        }
    }
}

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
    "Profile",
    "Settings",
    "Quit",
];

#[derive(Clone, Copy)]
pub enum ToolbarAction {
    Create,
    Join,
    Menu,
    #[cfg(feature = "audio")]
    Call,
    #[cfg(feature = "audio")]
    Mute,
    #[cfg(feature = "video")]
    Video,
    Quit,
}

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
    pub quit_requested: bool,
    pub error_message: Option<String>,
    pub palette: Palette,
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
            quit_requested: false,
            error_message: None,
            palette: Palette::default(),
        }
    }
}

impl App {
    pub fn active_code(&self) -> Option<&str> {
        match self.selection {
            Selection::Flock(i) => self.flocks.get(i).map(|f| f.code.as_str()),
            Selection::Channel(ri, ci) => self
                .roosts
                .get(ri)
                .and_then(|r| r.channels.get(ci))
                .map(|c| c.code.as_str()),
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

    pub fn select(&mut self, selection: Selection) {
        self.selection = selection;
        match selection {
            Selection::Flock(i) => {
                if let Some(flock) = self.flocks.get_mut(i) {
                    flock.unread = 0;
                }
            }
            Selection::Channel(ri, ci) => {
                if let Some(roost) = self.roosts.get_mut(ri) {
                    if let Some(channel) = roost.channels.get_mut(ci) {
                        channel.unread = 0;
                    }
                    roost.unread = roost.channels.iter().map(|channel| channel.unread).sum();
                }
            }
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

pub fn toolbar_buttons(_app: &App) -> Vec<(ToolbarAction, &'static str, u16, u16)> {
    let mut buttons = vec![
        (ToolbarAction::Create, "Create"),
        (ToolbarAction::Join, "Join"),
        (ToolbarAction::Menu, "Menu"),
    ];
    #[cfg(feature = "audio")]
    {
        buttons.push((
            ToolbarAction::Call,
            if _app.in_call { "Hang up" } else { "Call" },
        ));
        buttons.push((
            ToolbarAction::Mute,
            if _app.muted { "Unmute" } else { "Mute" },
        ));
    }
    #[cfg(feature = "video")]
    buttons.push((
        ToolbarAction::Video,
        if _app.show_video {
            "Video off"
        } else {
            "Video on"
        },
    ));
    buttons.push((ToolbarAction::Quit, "Quit"));

    let mut x = 0u16;
    buttons
        .into_iter()
        .map(|(action, label)| {
            let width = label.len() as u16 + 2;
            let button = (action, label, x, width);
            x += width + 1;
            button
        })
        .collect()
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

    if let Some(bg) = app.palette.background {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::raw("")])).style(Style::new().bg(bg)),
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

    if app.show_create_room {
        draw_create_room_popup(f, app);
    } else if app.show_join_room {
        draw_join_room_popup(f, app);
    } else if app.show_join_roost {
        draw_join_roost_popup(f, app);
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
            Paragraph::new(format!(" {code}")).style(Style::new().fg(app.palette.dim)),
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
            let dot = flock_dot(&fv.code, app.palette.accent);
            let label = &fv.code[..12.min(fv.code.len())];
            ListItem::new(Line::from(vec![
                Span::styled(mark, Style::new().fg(app.palette.selection)),
                Span::styled("\u{25AE} ", Style::new().fg(dot)),
                Span::styled(
                    label.to_string(),
                    Style::new().fg(if sel {
                        app.palette.selection
                    } else {
                        app.palette.text
                    }),
                ),
                Span::styled(unread, Style::new().fg(app.palette.selection)),
            ]))
        })
        .collect();

    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(app.palette.border))
                .title(Span::styled(
                    format!(" flocks ({}) ", app.flocks.len()),
                    Style::new().fg(app.palette.accent),
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
        let dot = flock_dot(&rv.code, app.palette.accent);
        let name = if rv.name.is_empty() {
            &rv.code[..12.min(rv.code.len())]
        } else {
            &rv.name[..]
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(caret, Style::new().fg(app.palette.dim)),
            Span::styled("\u{25AE} ", Style::new().fg(dot)),
            Span::styled(
                name.to_string(),
                Style::new()
                    .fg(if head_sel {
                        app.palette.selection
                    } else {
                        app.palette.text
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(unread, Style::new().fg(app.palette.selection)),
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
                    Span::styled(
                        "#",
                        Style::new().fg(if sel {
                            app.palette.selection
                        } else {
                            app.palette.dim
                        }),
                    ),
                    Span::styled(
                        format!(" {}", ch.name),
                        Style::new().fg(if sel {
                            app.palette.selection
                        } else {
                            app.palette.channel
                        }),
                    ),
                    Span::styled(cu, Style::new().fg(app.palette.selection)),
                ])));
            }
        }
    }

    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(app.palette.border))
                .title(Span::styled(
                    format!(" roosts ({}) ", app.roosts.len()),
                    Style::new().fg(app.palette.accent),
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
                    Style::new()
                        .fg(app.palette.author)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(m.body.clone(), Style::new().fg(app.palette.text)),
            ]))
        })
        .collect();

    let title = format!(" {} . {} birds ", app.active_title(), app.bird_count());
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(app.palette.border))
                .title(Span::styled(title, Style::new().fg(app.palette.text))),
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
            Style::new()
                .fg(app.palette.selection)
                .add_modifier(Modifier::BOLD),
        ),
    ])));

    for (i, id) in app.peers.iter().enumerate() {
        let sel = i == app.selected_peer;
        let mark = if sel { "> " } else { "  " };
        let (glyph, gc) = match app.peer_status.get(id) {
            Some(BirdStatus::InCall) => ("~", app.palette.author),
            Some(BirdStatus::Idle) => ("-", app.palette.dim),
            _ => ("o", app.palette.accent),
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(mark, Style::new().fg(app.palette.selection)),
            Span::styled(format!("{glyph} "), Style::new().fg(gc)),
            Span::styled(
                app.peer_display_name(id),
                Style::new().fg(if sel {
                    app.palette.selection
                } else {
                    app.palette.text
                }),
            ),
        ])));
    }

    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(app.palette.border))
                .title(Span::styled(" birds ", Style::new().fg(app.palette.accent))),
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
    for (_action, label, _x, _w) in toolbar_buttons(app) {
        spans.push(Span::styled("[", Style::new().fg(app.palette.accent)));
        spans.push(Span::styled(label, Style::new().fg(app.palette.accent)));
        spans.push(Span::styled("]", Style::new().fg(app.palette.accent)));
        spans.push(Span::raw(" "));
    }
    let status = app
        .error_message
        .clone()
        .unwrap_or_else(|| status_text(app));
    if !status.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(status, Style::new().fg(app.palette.dim)));
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
            .border_style(Style::new().fg(app.palette.border))
            .title(Span::styled(" Menu ", Style::new().fg(app.palette.accent))),
        popup,
    );
    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let items: Vec<ListItem> = MENU_ITEMS
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let sel = i == app.menu_selection;
            let style = if sel {
                Style::new()
                    .fg(app.palette.selection)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(app.palette.text)
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
            .border_style(Style::new().fg(app.palette.border))
            .title(Span::styled(
                " Create Room ",
                Style::new().fg(app.palette.accent),
            )),
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
        Paragraph::new("Your invite code:").style(Style::new().fg(app.palette.text)),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(invite).style(Style::new().fg(app.palette.invite)),
        rows[1],
    );
    f.render_widget(
        Paragraph::new("Press Enter to create, Esc to cancel.")
            .style(Style::new().fg(app.palette.dim)),
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
        app,
    );
}

fn draw_join_roost_popup(f: &mut Frame, app: &App) {
    draw_input_popup(
        f,
        " Join Roost ",
        "Enter the roost code:",
        &app.join_roost_input,
        " Enter = join . Esc = cancel",
        app,
    );
}

fn draw_input_popup(f: &mut Frame, title: &str, prompt: &str, value: &str, hint: &str, app: &App) {
    let popup = centered(f.area(), 60, 8);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(app.palette.border))
            .title(Span::styled(
                title.to_string(),
                Style::new().fg(app.palette.accent),
            )),
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
    f.render_widget(
        Paragraph::new(prompt).style(Style::new().fg(app.palette.text)),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(format!(" {value}_")).style(Style::new().fg(app.palette.selection)),
        rows[1],
    );
    f.render_widget(
        Paragraph::new(hint).style(Style::new().fg(app.palette.dim)),
        rows[2],
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

fn flock_dot(code: &str, fallback: Color) -> Color {
    parse_color_code(code)
        .first()
        .map(|&(r, g, b)| Color::Rgb(r, g, b))
        .unwrap_or(fallback)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_selection_uses_channel_route_and_clears_unread() {
        let mut app = App::default();
        app.roosts.push(RoostView {
            code: "roost".into(),
            name: "Nest".into(),
            channels: vec![FlockView {
                code: "roost/general".into(),
                name: "general".into(),
                messages: vec![],
                unread: 3,
            }],
            unread: 3,
        });

        app.select(Selection::Channel(0, 0));

        assert_eq!(app.active_code(), Some("roost/general"));
        assert_eq!(app.roosts[0].channels[0].unread, 0);
        assert_eq!(app.roosts[0].unread, 0);
    }

    #[test]
    fn empty_background_and_invalid_colors_are_rejected() {
        assert_eq!(hex_to_color("#102030"), Some(Color::Rgb(16, 32, 48)));
        assert_eq!(hex_to_color(""), None);
        assert_eq!(hex_to_color("#GGGGGG"), None);
    }
}
