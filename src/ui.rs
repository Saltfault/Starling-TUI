use image::RgbImage;
use iroh::EndpointId;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};
use sha2::{Digest, Sha256};
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

pub const MENU_ITEMS: &[&str] = &["Create Room", "Join", "Settings", "Quit"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScrollPanel {
    Flocks,
    Roosts,
    Birds,
}

#[derive(Clone, Copy, Default)]
pub struct SpringScroll {
    pub current: f32,
    target: f32,
    velocity: f32,
    max: f32,
}

impl SpringScroll {
    pub fn set_max(&mut self, max: usize) {
        self.max = max as f32;
        self.target = self.target.clamp(0.0, self.max);
    }

    pub fn scroll(&mut self, delta: f32) {
        let desired = self.target + delta;
        self.target = desired.clamp(0.0, self.max);
        if desired < 0.0 || desired > self.max {
            self.current += (desired - self.target) * 0.45;
        }
    }

    pub fn advance(&mut self, dt: f32) -> bool {
        let stiffness = 95.0;
        let damping = 19.5;
        self.velocity += (self.target - self.current) * stiffness * dt;
        self.velocity *= (-damping * dt).exp();
        self.current += self.velocity * dt;

        if (self.current - self.target).abs() < 0.01 && self.velocity.abs() < 0.01 {
            self.current = self.target;
            self.velocity = 0.0;
            false
        } else {
            true
        }
    }

    pub fn row_index(&self, visible_row: usize) -> Option<usize> {
        let index = self.current.round() as isize + visible_row as isize;
        (index >= 0).then_some(index as usize)
    }

    fn rounded_offset(&self) -> isize {
        self.current.round() as isize
    }
}

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
    pub node_id: Option<EndpointId>,
    pub create_flock_code: Option<String>,
    pub show_create_room: bool,
    pub show_join_room: bool,
    pub join_input: String,
    pub in_call: bool,
    pub muted: bool,
    pub peer_names: HashMap<EndpointId, String>,
    pub peer_status: HashMap<EndpointId, BirdStatus>,
    #[allow(dead_code)]
    pub local_video_frame: Option<RgbImage>,
    #[allow(dead_code)]
    pub remote_video_frames: HashMap<EndpointId, RgbImage>,
    #[allow(dead_code)]
    pub show_video: bool,
    pub show_menu: bool,
    pub menu_selection: usize,
    pub flock_scroll: SpringScroll,
    pub roost_scroll: SpringScroll,
    pub bird_scroll: SpringScroll,
    pub scroll_focus: ScrollPanel,
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
            create_flock_code: None,
            show_create_room: false,
            show_join_room: false,
            join_input: String::new(),
            in_call: false,
            muted: false,
            peer_names: HashMap::new(),
            peer_status: HashMap::new(),
            local_video_frame: None,
            remote_video_frames: HashMap::new(),
            show_video: false,
            show_menu: false,
            menu_selection: 0,
            flock_scroll: SpringScroll::default(),
            roost_scroll: SpringScroll::default(),
            bird_scroll: SpringScroll::default(),
            scroll_focus: ScrollPanel::Flocks,
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
    pub fn selected_peer_id(&self) -> Option<EndpointId> {
        self.peers.get(self.selected_peer).copied()
    }

    pub fn peer_display_name(&self, id: &EndpointId) -> String {
        self.peer_names
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.fmt_short().to_string())
    }

    pub fn roost_row_count(&self) -> usize {
        self.roosts
            .iter()
            .enumerate()
            .map(|(index, roost)| {
                1 + self
                    .expanded
                    .contains(&index)
                    .then_some(roost.channels.len())
                    .unwrap_or(0)
            })
            .sum()
    }

    pub fn scroll_mut(&mut self, panel: ScrollPanel) -> &mut SpringScroll {
        match panel {
            ScrollPanel::Flocks => &mut self.flock_scroll,
            ScrollPanel::Roosts => &mut self.roost_scroll,
            ScrollPanel::Birds => &mut self.bird_scroll,
        }
    }

    pub fn update_scroll_bounds(
        &mut self,
        flock_viewport: usize,
        roost_viewport: usize,
        bird_viewport: usize,
    ) {
        self.flock_scroll
            .set_max(self.flocks.len().saturating_sub(flock_viewport));
        self.roost_scroll
            .set_max(self.roost_row_count().saturating_sub(roost_viewport));
        self.bird_scroll
            .set_max(self.bird_count().saturating_sub(bird_viewport));
    }

    pub fn advance_scroll(&mut self, dt: f32) -> bool {
        self.flock_scroll.advance(dt) | self.roost_scroll.advance(dt) | self.bird_scroll.advance(dt)
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

fn window_list_items<'a>(items: Vec<ListItem<'a>>, scroll: SpringScroll) -> Vec<ListItem<'a>> {
    let offset = scroll.rounded_offset();
    if offset < 0 {
        std::iter::repeat_with(|| ListItem::new(""))
            .take((-offset) as usize)
            .chain(items)
            .collect()
    } else {
        items.into_iter().skip(offset as usize).collect()
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
    let items = window_list_items(items, app.flock_scroll);

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

    let items = window_list_items(items, app.roost_scroll);
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

#[cfg(feature = "video")]
fn draw_video_grid(f: &mut Frame, app: &App, area: Rect) {
    let mut tiles: Vec<(String, Option<&RgbImage>)> = Vec::new();
    if app.show_video {
        tiles.push((
            format!("{} (you)", app.name),
            app.local_video_frame.as_ref(),
        ));
    }
    for (peer, frame) in &app.remote_video_frames {
        tiles.push((app.peer_display_name(peer), Some(frame)));
    }
    if tiles.is_empty() {
        return;
    }

    let mut columns = 1usize;
    while columns * columns < tiles.len() {
        columns += 1;
    }
    let rows = tiles.len().div_ceil(columns);
    let row_areas = Layout::vertical(vec![Constraint::Ratio(1, rows as u32); rows]).split(area);
    for (row, row_area) in row_areas.iter().enumerate() {
        let column_areas = Layout::horizontal(vec![Constraint::Ratio(1, columns as u32); columns])
            .split(*row_area);
        for (column, tile_area) in column_areas.iter().enumerate() {
            let index = row * columns + column;
            let Some((name, frame)) = tiles.get(index) else {
                continue;
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(app.palette.border))
                .title(Span::styled(
                    format!(" {name} "),
                    Style::new().fg(app.palette.accent),
                ));
            let inner = block.inner(*tile_area);
            f.render_widget(block, *tile_area);
            if let Some(frame) = frame {
                let lines = crate::video::frame_to_lines(frame, inner.width, inner.height);
                f.render_widget(Paragraph::new(lines), inner);
            } else {
                f.render_widget(
                    Paragraph::new("camera starting...").style(Style::new().fg(app.palette.dim)),
                    inner,
                );
            }
        }
    }
}

fn draw_messages(f: &mut Frame, app: &App, area: Rect) {
    #[cfg(feature = "video")]
    let area = if app.show_video || !app.remote_video_frames.is_empty() {
        let panes = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        draw_video_grid(f, app, panes[1]);
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

    let items = window_list_items(items, app.bird_scroll);
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
    let popup = centered(f.area(), 72, 12);
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
        Constraint::Length(4),
        Constraint::Min(1),
    ])
    .split(inner);
    let invite = app
        .create_flock_code
        .as_deref()
        .unwrap_or("waiting for endpoint...");
    f.render_widget(
        Paragraph::new("Your invite code:").style(Style::new().fg(app.palette.text)),
        rows[0],
    );
    f.render_widget(Line::from(color_swatches(invite)), rows[1]);
    f.render_widget(
        Paragraph::new(invite)
            .style(Style::new().fg(app.palette.invite))
            .wrap(Wrap { trim: false }),
        rows[2],
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
        " Join ",
        "Enter a flock or roost code:",
        &app.join_input,
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
    let compact = code.trim();
    if !compact.is_empty()
        && !compact.contains('-')
        && compact.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return compact
            .as_bytes()
            .chunks_exact(6)
            .filter_map(|group| {
                let group = std::str::from_utf8(group).ok()?;
                Some((
                    u8::from_str_radix(&group[0..2], 16).ok()?,
                    u8::from_str_radix(&group[2..4], 16).ok()?,
                    u8::from_str_radix(&group[4..6], 16).ok()?,
                ))
            })
            .collect();
    }

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
    if colors.is_empty() && !code.is_empty() {
        colors.extend(
            Sha256::digest(code.as_bytes())
                .chunks_exact(3)
                .take(6)
                .map(|chunk| (chunk[0], chunk[1], chunk[2])),
        );
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
    fn overscroll_springs_back_to_the_clamped_edge() {
        let mut scroll = SpringScroll::default();
        scroll.set_max(4);
        scroll.scroll(-3.0);
        assert!(scroll.current < 0.0);

        for _ in 0..100 {
            scroll.advance(0.05);
        }

        assert_eq!(scroll.current, 0.0);
        assert_eq!(scroll.row_index(0), Some(0));
    }

    #[test]
    fn scroll_offset_maps_visible_rows_to_content() {
        let mut scroll = SpringScroll::default();
        scroll.set_max(10);
        scroll.current = 4.0;

        assert_eq!(scroll.row_index(0), Some(4));
        assert_eq!(scroll.row_index(3), Some(7));
    }

    #[test]
    fn hexadecimal_codes_map_directly_to_colors() {
        let colors = parse_color_code("001122AABBCC");
        assert_eq!(colors, vec![(0x00, 0x11, 0x22), (0xAA, 0xBB, 0xCC)]);
    }

    #[test]
    fn empty_background_and_invalid_colors_are_rejected() {
        assert_eq!(hex_to_color("#102030"), Some(Color::Rgb(16, 32, 48)));
        assert_eq!(hex_to_color(""), None);
        assert_eq!(hex_to_color("#GGGGGG"), None);
    }
}
