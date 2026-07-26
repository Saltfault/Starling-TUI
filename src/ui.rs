use image::RgbImage;
use iroh::EndpointId;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};
use sha2::{Digest, Sha256};
use starling::event::{BirdStatus, ChatMessage};
use starling::protocol::{RoostId, SpaceId};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

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

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub struct MemberProfile {
    pub endpoint: EndpointId,
    pub name: String,
    pub pronouns: String,
}

#[derive(Clone, Debug)]
pub struct LiveLease {
    pub deadline: tokio::time::Instant,
    pub sequence: u64,
}

impl LiveLease {
    fn is_live_at(&self, now: tokio::time::Instant) -> bool {
        self.deadline > now
    }
}

/// Presence for one selected space. Member profiles survive lease expiry.
#[derive(Default)]
pub struct ContextPresence {
    pub members: HashMap<EndpointId, MemberProfile>,
    pub live: HashMap<EndpointId, LiveLease>,
    ordered_ids: Vec<EndpointId>,
}

impl ContextPresence {
    pub fn set_profile(&mut self, profile: MemberProfile) {
        self.members.insert(profile.endpoint, profile);
    }

    pub fn apply_verified_lease(
        &mut self,
        endpoint: EndpointId,
        lease: LiveLease,
        now: tokio::time::Instant,
    ) {
        if lease.is_live_at(now) {
            let replace = self
                .live
                .get(&endpoint)
                .is_none_or(|current| lease.sequence > current.sequence);
            if replace {
                self.live.insert(endpoint, lease);
                if !self.ordered_ids.contains(&endpoint) {
                    self.ordered_ids.push(endpoint);
                }
            }
        }
    }

    pub fn live_ids(&self, now: tokio::time::Instant) -> Vec<EndpointId> {
        self.ordered_ids
            .iter()
            .copied()
            .filter(|endpoint| {
                self.live
                    .get(endpoint)
                    .is_some_and(|lease| lease.is_live_at(now))
            })
            .collect()
    }

    pub fn expire(&mut self, now: tokio::time::Instant) {
        self.live.retain(|_, lease| lease.is_live_at(now));
        self.ordered_ids
            .retain(|endpoint| self.live.contains_key(endpoint));
    }
}

#[derive(Default)]
pub struct ScopedPresence {
    pub contexts: HashMap<starling::protocol::SpaceId, ContextPresence>,
}

impl ScopedPresence {
    pub fn context_mut(&mut self, space: starling::protocol::SpaceId) -> &mut ContextPresence {
        self.contexts.entry(space).or_default()
    }

    pub fn neighbor_down(&mut self, _endpoint: EndpointId) {
        // Connectivity is only a hint. Signed leases remain authoritative.
    }
}

#[derive(Default)]
pub struct FlockView {
    pub code: String,
    pub name: String,
    /// `private` marks sealed 1:1 chirps so the renderer can prepend a 🔒;
    /// legacy broadcasts and history backfill come through as `private = false`.
    pub messages: Vec<MessageView>,
    pub unread: usize,
}

#[derive(Default)]
pub struct RoostView {
    pub code: String,
    pub name: String,
    pub channels: Vec<FlockView>,
    pub unread: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatMessageView {
    pub event_hash: [u8; 32],
    pub sender: EndpointId,
    pub author: String,
    pub body: String,
    pub ts: i64,
}

/// A chat message ready to render in a flock view, optionally flagged as a
/// private chirp so the renderer can mark it with a 🔒. Private chirps live
/// in the same logical thread so the reader can see them in order.
#[derive(Clone, Debug)]
pub struct MessageView {
    pub msg: ChatMessage,
    pub private: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextView {
    pub id: SpaceId,
    pub title: String,
    pub roost: Option<RoostId>,
    pub base_invite_display: Option<String>,
    pub messages: Vec<ChatMessageView>,
    pub unread: usize,
    pub state: ContextState,
    pub secret: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub enum ContextState {
    AwaitingKeys,
    Reconciling,
    Ready,
    Revoked,
    NeedsUserAction,
    Restoring,
}

pub const MENU_ITEMS: &[&str] = &[
    "Create a Flock",
    "Edit a Flock",
    "Join",
    "Profile",
    "Settings",
    "Delete All Data",
    "Quit",
];

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
    Menu,
    #[cfg(feature = "audio")]
    Call,
    #[cfg(feature = "audio")]
    Mute,
    #[cfg(feature = "video")]
    Video,
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

const NOTICE_DURATION: Duration = Duration::from_secs(4);

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
    pub create_flock_secret: Option<[u8; 32]>,
    pub create_flock_name: String,
    pub show_create_room: bool,
    pub show_join_room: bool,
    pub join_input: String,
    #[allow(dead_code)]
    pub joining: Option<String>,
    pub show_edit_flock: bool,
    pub edit_flock_code: String,
    pub edit_flock_name: String,
    pub in_call: bool,
    pub muted: bool,
    pub peer_names: HashMap<EndpointId, String>,
    pub peer_status: HashMap<EndpointId, BirdStatus>,
    /// Received `crypto_box` DM public keys, keyed by their authenticated
    /// endpoint id (the [`Signed::author`] of the profile announcement, not
    /// the untrusted `id` field inside the payload). `/chirp` looks up the
    /// recipient through this table; main() feeds it from `AppEvent::DmKey`.
    pub peer_dm_keys: HashMap<EndpointId, Vec<u8>>,
    #[allow(dead_code)]
    pub local_video_frame: Option<RgbImage>,
    #[allow(dead_code)]
    pub remote_video_frames: HashMap<EndpointId, RgbImage>,
    #[allow(dead_code)]
    pub show_video: bool,
    pub show_menu: bool,
    pub menu_selection: usize,
    pub show_delete_confirm: bool,
    pub delete_confirm_input: String,
    pub flock_scroll: SpringScroll,
    pub roost_scroll: SpringScroll,
    pub bird_scroll: SpringScroll,
    pub scroll_focus: ScrollPanel,
    pub quit_requested: bool,
    pub skip_save_on_exit: bool,
    pub error_message: Option<String>,
    pub palette: Palette,
    pub contexts: HashMap<SpaceId, ContextView>,
    pub context_order: Vec<SpaceId>,
    pub active: Option<SpaceId>,
    pub presence: ScopedPresence,
    pub status_notice: Option<String>,
    pub status_notice_expires_at: Option<Instant>,
    /// The client's own effective permissions in the active roost. UX-only; the
    /// roost re-checks every privileged action.
    pub my_perms: starling::roost::perms::Perm,
    /// Peer endpoint id -> top role color, for coloring names in the bird list.
    pub peer_roles: HashMap<EndpointId, (u8, u8, u8)>,
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
            create_flock_secret: None,
            create_flock_name: String::new(),
            show_create_room: false,
            show_join_room: false,
            join_input: String::new(),
            joining: None,
            show_edit_flock: false,
            edit_flock_code: String::new(),
            edit_flock_name: String::new(),
            in_call: false,
            muted: false,
            peer_names: HashMap::new(),
            peer_status: HashMap::new(),
            peer_dm_keys: HashMap::new(),
            local_video_frame: None,
            remote_video_frames: HashMap::new(),
            show_video: false,
            show_menu: false,
            menu_selection: 0,
            show_delete_confirm: false,
            delete_confirm_input: String::new(),
            flock_scroll: SpringScroll::default(),
            roost_scroll: SpringScroll::default(),
            bird_scroll: SpringScroll::default(),
            scroll_focus: ScrollPanel::Flocks,
            quit_requested: false,
            skip_save_on_exit: false,
            error_message: None,
            palette: Palette::default(),
            contexts: HashMap::new(),
            context_order: Vec::new(),
            active: None,
            presence: ScopedPresence::default(),
            status_notice: None,
            status_notice_expires_at: None,
            my_perms: starling::roost::perms::Perm::empty(),
            peer_roles: HashMap::new(),
        }
    }
}

impl App {
    pub fn insert_context(&mut self, context: ContextView) {
        let id = context.id;
        if !self.contexts.contains_key(&id) {
            self.context_order.push(id);
        }
        self.contexts.insert(id, context);
        if self.active.is_none() {
            self.select_context(id);
        }
    }

    pub fn ordered_contexts(&self) -> impl Iterator<Item = &ContextView> {
        self.context_order
            .iter()
            .filter_map(|id| self.contexts.get(id))
    }

    pub fn select_context(&mut self, id: SpaceId) -> bool {
        let Some(context) = self.contexts.get_mut(&id) else {
            return false;
        };
        context.unread = 0;
        self.active = Some(id);
        true
    }

    pub fn active_context(&self) -> Option<&ContextView> {
        self.active.and_then(|id| self.contexts.get(&id))
    }

    pub fn active_context_messages(&self) -> Option<&[ChatMessageView]> {
        self.active_context()
            .map(|context| context.messages.as_slice())
    }

    /// Returns the newest message timestamp for a flock or roost channel
    /// identified by its display code. For roost codes (without a channel
    /// suffix), returns the max across all channels in that roost.
    pub fn newest_ts(&self, code: &str) -> Option<i64> {
        for flock in &self.flocks {
            if flock.code == code {
                return flock.messages.iter().map(|m| m.msg.ts).max();
            }
        }
        for roost in &self.roosts {
            if roost.code == code {
                return roost
                    .channels
                    .iter()
                    .flat_map(|ch| ch.messages.iter().map(|m| m.msg.ts))
                    .max();
            }
            for channel in &roost.channels {
                if channel.code == code {
                    return channel.messages.iter().map(|m| m.msg.ts).max();
                }
            }
        }
        None
    }

    pub fn active_base_invite(&self) -> Option<&str> {
        let context = self.active_context()?;
        if let Some(roost) = context.roost {
            self.contexts.values().find_map(|candidate| {
                let belongs_to_roost = candidate.roost == Some(roost)
                    || matches!(candidate.id, SpaceId::RoostChannel { roost: id, .. } if id == roost);
                (candidate.id != context.id && belongs_to_roost)
                    .then_some(candidate.base_invite_display.as_deref())
                    .flatten()
                    .filter(|invite| !invite.is_empty() && !invite.contains('/'))
            })
        } else {
            context
                .base_invite_display
                .as_deref()
                .filter(|invite| !invite.is_empty() && !invite.contains('/'))
        }
    }

    pub fn show_status_notice(&mut self, message: impl Into<String>, now: Instant) {
        self.status_notice = Some(message.into());
        self.status_notice_expires_at = Some(now + NOTICE_DURATION);
    }

    /// Refresh the client's view of its own permissions and peer role colors
    /// from a roost's `PermState`. UX-only; the roost re-checks every action.
    pub fn apply_roost_perms(&mut self, perms: &starling::roost::perms::PermState) {
        if let Some(my_id) = self.node_id {
            self.my_perms = perms.effective(&my_id);
        }
        self.peer_roles.clear();
        for (&id, role_indices) in &perms.members {
            let color = role_indices
                .iter()
                .filter_map(|&i| perms.roles.get(i))
                .max_by_key(|role| role.position)
                .map(|role| role.color)
                .unwrap_or((150, 150, 150));
            self.peer_roles.insert(id, color);
        }
        if let Some(owner) = perms.owner {
            self.peer_roles.insert(owner, (255, 215, 0));
        }
    }

    pub fn expire_status_notice(&mut self, now: Instant) -> bool {
        if self
            .status_notice_expires_at
            .is_some_and(|expires_at| now >= expires_at)
        {
            self.status_notice = None;
            self.status_notice_expires_at = None;
            true
        } else {
            false
        }
    }

    pub fn visible_status_notice(&self, now: Instant) -> Option<&str> {
        self.status_notice.as_deref().filter(|_| {
            self.status_notice_expires_at
                .is_none_or(|expires_at| now < expires_at)
        })
    }

    pub fn live_member_count(&self, space: starling::protocol::SpaceId) -> usize {
        self.presence
            .contexts
            .get(&space)
            .map(|p| p.live_ids(tokio::time::Instant::now()).len())
            .unwrap_or(0)
    }

    pub fn active_code(&self) -> Option<&str> {
        if self.active_context().is_some() {
            return self.active_base_invite();
        }
        match self.selection {
            Selection::Flock(i) => self.flocks.get(i).map(|f| f.code.as_str()),
            Selection::Channel(ri, ci) => self
                .roosts
                .get(ri)
                .and_then(|r| r.channels.get(ci))
                .map(|c| c.code.as_str()),
        }
    }

    pub fn active_messages(&self) -> &[MessageView] {
        if self.active_context().is_some() {
            return &[];
        }
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
        if let Some(context) = self.active_context() {
            return context.title.clone();
        }
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
        self.active = None;
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

    pub fn selected_peer_id(&self) -> Option<EndpointId> {
        self.peers.get(self.selected_peer).copied()
    }

    /// Decode the currently-selected roost's invite code into its opener
    /// `EndpointId`. Used to route moderation commands to the right roost.
    /// Returns `None` when a flock (not a roost) is selected or the code is
    /// not a valid roost code.
    pub fn selected_roost_endpoint_id(&self) -> Option<EndpointId> {
        let ri = match self.selection {
            Selection::Channel(ri, _) => ri,
            Selection::Flock(_) => return None,
        };
        let roost = self.roosts.get(ri)?;
        let decoded = starling::net::decode_typed_code(&roost.code)?;
        starling::net::typed_code_node_id(&decoded)
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
                1 + if self.expanded.contains(&index) {
                    roost.channels.len()
                } else {
                    0
                }
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
        let context_count = self
            .context_order
            .iter()
            .filter(|id| self.contexts.contains_key(id))
            .count();
        // Don't count legacy flocks that already have a typed context entry
        // so the scroll region matches what draw_flocks actually renders.
        let deduped_flocks = self
            .flocks
            .iter()
            .filter(|fv| {
                !self
                    .contexts
                    .values()
                    .any(|ctx| ctx.secret.as_deref() == Some(fv.code.as_str()))
            })
            .count();
        self.flock_scroll
            .set_max((deduped_flocks + context_count).saturating_sub(flock_viewport));
        self.roost_scroll
            .set_max(self.roost_row_count().saturating_sub(roost_viewport));
        self.bird_scroll
            .set_max(self.bird_count().saturating_sub(bird_viewport));
    }

    pub fn advance_scroll(&mut self, dt: f32) -> bool {
        self.flock_scroll.advance(dt) | self.roost_scroll.advance(dt) | self.bird_scroll.advance(dt)
    }
}

pub fn toolbar_buttons(app: &App) -> Vec<(ToolbarAction, &'static str, u16, u16)> {
    let labels: Vec<(ToolbarAction, &'static str)> = std::iter::once((
        ToolbarAction::Menu,
        if app.show_menu { "Close" } else { "Menu" },
    ))
    .chain({
        #[cfg(feature = "audio")]
        {
            vec![
                (
                    ToolbarAction::Call,
                    if app.in_call { "Hang up" } else { "Call" },
                ),
                (
                    ToolbarAction::Mute,
                    if app.muted { "Unmute" } else { "Mute" },
                ),
            ]
            .into_iter()
        }
        #[cfg(not(feature = "audio"))]
        {
            std::iter::empty()
        }
    })
    .chain({
        #[cfg(feature = "video")]
        {
            vec![(
                ToolbarAction::Video,
                if app.show_video {
                    "Video off"
                } else {
                    "Video on"
                },
            )]
            .into_iter()
        }
        #[cfg(not(feature = "video"))]
        {
            std::iter::empty()
        }
    })
    .collect();
    let mut x = 0u16;
    labels
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
    } else if app.show_edit_flock {
        draw_edit_flock_popup(f, app);
    } else if app.show_join_room {
        draw_join_room_popup(f, app);
    } else if app.show_delete_confirm {
        draw_delete_confirm_popup(f, app);
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
            Paragraph::new(format!(" starling://join/{code}"))
                .style(Style::new().fg(app.palette.invite)),
            header[1],
        );
    }
    if let Some(ref code) = app.joining {
        let line = Line::from(vec![
            Span::raw(" Joining "),
            Span::styled(code.as_str(), Style::new().fg(app.palette.accent)),
            Span::raw("..."),
        ]);
        f.render_widget(line, header[1]);
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
    let typed_items = app.ordered_contexts().map(|context| {
        let selected = app.active == Some(context.id);
        let mark = if selected { "> " } else { "  " };
        let unread = if context.unread > 0 {
            format!(" ({})", context.unread)
        } else {
            String::new()
        };
        ListItem::new(Line::from(vec![
            Span::styled(mark, Style::new().fg(app.palette.selection)),
            Span::styled("\u{25AE} ", Style::new().fg(app.palette.accent)),
            Span::styled(
                context.title.clone(),
                Style::new().fg(if selected {
                    app.palette.selection
                } else {
                    app.palette.text
                }),
            ),
            Span::styled(unread, Style::new().fg(app.palette.selection)),
        ]))
    });
    // Skip legacy flocks that already have a typed context entry so the
    // same flock does not appear twice in the sidebar.
    let typed_secrets: std::collections::HashSet<&str> = app
        .contexts
        .values()
        .filter_map(|ctx| ctx.secret.as_deref())
        .collect();
    let legacy_items = app
        .flocks
        .iter()
        .enumerate()
        .filter(|(_, fv)| !typed_secrets.contains(fv.code.as_str()))
        .map(|(i, fv)| {
            let sel = app.selection == Selection::Flock(i);
            let mark = if sel { "> " } else { "  " };
            let unread = if fv.unread > 0 {
                format!(" ({})", fv.unread)
            } else {
                String::new()
            };
            let dot = flock_dot(&fv.code, app.palette.accent);
            let label = if fv.name.is_empty() {
                &fv.code[..12.min(fv.code.len())]
            } else {
                fv.name.as_str()
            };
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
        });
    let items: Vec<ListItem> = typed_items.chain(legacy_items).collect();
    let displayed_count = items.len();
    let items = window_list_items(items, app.flock_scroll);

    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(app.palette.border))
                .title(Span::styled(
                    format!(" flocks ({}) ", displayed_count),
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

fn draw_typed_messages(f: &mut Frame, app: &App, area: Rect) -> bool {
    let Some(messages) = app.active_context_messages() else {
        return false;
    };
    let items = messages.iter().map(|message| {
        ListItem::new(Line::from(vec![
            Span::styled(
                format!("{}: ", message.author),
                Style::new()
                    .fg(app.palette.author)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(message.body.clone(), Style::new().fg(app.palette.text)),
        ]))
    });
    let title = format!(" {} . {} birds ", app.active_title(), app.bird_count());
    f.render_widget(
        List::new(items.collect::<Vec<_>>()).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(app.palette.border))
                .title(Span::styled(title, Style::new().fg(app.palette.text))),
        ),
        area,
    );
    true
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

    if draw_typed_messages(f, app, area) {
        return;
    }

    let items: Vec<ListItem> = app
        .active_messages()
        .iter()
        .map(|m| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    if m.private { "⚷ " } else { "" },
                    Style::new().fg(app.palette.selection),
                ),
                Span::styled(
                    format!("{}: ", m.msg.author),
                    Style::new()
                        .fg(app.palette.author)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(m.msg.body.clone(), Style::new().fg(app.palette.text)),
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
        let (r, g, b) = app.peer_roles.get(id).copied().unwrap_or((150, 150, 150));
        let name_color = if sel {
            app.palette.selection
        } else {
            Color::Rgb(r, g, b)
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(mark, Style::new().fg(app.palette.selection)),
            Span::styled(format!("{glyph} "), Style::new().fg(gc)),
            Span::styled(app.peer_display_name(id), Style::new().fg(name_color)),
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
    if let Some(notice) = app.visible_status_notice(Instant::now()) {
        notice.to_string()
    } else if app.in_call {
        format!("in call{}", if app.muted { " . muted" } else { " . live" })
    } else if let Some(active) = app.active {
        format!("{} live", app.live_member_count(active))
    } else {
        String::new()
    }
}

fn draw_button_bar(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    for (action, label, _x, _w) in toolbar_buttons(app) {
        let enabled = match action {
            #[cfg(feature = "audio")]
            ToolbarAction::Call => app.in_call || app.selected_peer_id().is_some(),
            _ => true,
        };
        let color = if enabled {
            app.palette.accent
        } else {
            app.palette.dim
        };
        spans.push(Span::styled("[", Style::new().fg(color)));
        spans.push(Span::styled(label, Style::new().fg(color)));
        spans.push(Span::styled("]", Style::new().fg(color)));
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

pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
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
    let popup = centered(f.area(), 60, 8);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(app.palette.border))
            .title(Span::styled(
                " Create a Flock ",
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
        Paragraph::new("Flock name:").style(Style::new().fg(app.palette.text)),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(format!(" {}_", app.create_flock_name))
            .style(Style::new().fg(app.palette.selection)),
        rows[1],
    );
    f.render_widget(
        Paragraph::new("Press Enter to create, Esc to cancel.")
            .style(Style::new().fg(app.palette.dim)),
        rows[2],
    );
}

fn draw_edit_flock_popup(f: &mut Frame, app: &App) {
    let popup = centered(f.area(), 60, 10);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(app.palette.border))
            .title(Span::styled(
                " Edit Flock ",
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
    f.render_widget(
        Paragraph::new("Flock name:").style(Style::new().fg(app.palette.text)),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(format!(" {}_", app.edit_flock_name))
            .style(Style::new().fg(app.palette.selection)),
        rows[1],
    );
    f.render_widget(
        Paragraph::new("Enter = save . Delete = delete flock . Esc = cancel")
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

fn draw_delete_confirm_popup(f: &mut Frame, app: &App) {
    let popup = centered(f.area(), 60, 8);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(app.palette.border))
            .title(Span::styled(
                " Delete all data ",
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
    f.render_widget(
        Paragraph::new("This will erase your identity, profile, roosts, and history.")
            .style(Style::new().fg(app.palette.text)),
        rows[0],
    );
    f.render_widget(
        Paragraph::new("Type DELETE to confirm:").style(Style::new().fg(app.palette.text)),
        rows[1],
    );
    f.render_widget(
        Paragraph::new(format!(" {}_", app.delete_confirm_input))
            .style(Style::new().fg(app.palette.selection)),
        rows[2],
    );
    f.render_widget(
        Paragraph::new(" Enter = confirm . Esc = cancel").style(Style::new().fg(app.palette.dim)),
        rows[3],
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
    use std::time::Duration;

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

    fn context(
        id: SpaceId,
        title: &str,
        base_invite: &str,
        parent_roost: Option<RoostId>,
    ) -> ContextView {
        ContextView {
            id,
            title: title.into(),
            roost: parent_roost,
            base_invite_display: Some(base_invite.into()),
            messages: Vec::new(),
            unread: 0,
            state: ContextState::Ready,
            secret: None,
        }
    }

    #[test]
    fn typed_contexts_keep_insertion_order_when_replaced() {
        let first = SpaceId::Flock(starling::protocol::FlockId::random());
        let second = SpaceId::Flock(starling::protocol::FlockId::random());
        let mut app = App::default();
        app.insert_context(context(first, "First", "first-invite", None));
        app.insert_context(context(second, "Second", "second-invite", None));
        app.insert_context(context(first, "Renamed", "first-invite", None));

        assert_eq!(app.context_order, vec![first, second]);
        assert_eq!(
            app.ordered_contexts()
                .map(|context| context.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Renamed", "Second"]
        );
    }

    fn append_message(app: &mut App, id: SpaceId, message: ChatMessageView) {
        let context = app.contexts.get_mut(&id).expect("context exists");
        context.messages.push(message);
        if app.active != Some(id) {
            context.unread += 1;
        }
    }

    #[test]
    fn switching_contexts_reuses_state_and_clears_unread() {
        let first = SpaceId::Flock(starling::protocol::FlockId::random());
        let second = SpaceId::Flock(starling::protocol::FlockId::random());
        let author = iroh::SecretKey::generate().public();
        let mut app = App::default();
        app.insert_context(context(first, "First", "first-invite", None));
        app.insert_context(context(second, "Second", "second-invite", None));
        append_message(
            &mut app,
            second,
            ChatMessageView {
                event_hash: [1; 32],
                sender: author,
                author: "Wren".into(),
                body: "hello".into(),
                ts: 0,
            },
        );

        assert_eq!(app.contexts[&second].unread, 1);
        assert!(app.select_context(second));
        assert_eq!(app.contexts[&second].unread, 0);
        assert_eq!(app.contexts[&second].messages.len(), 1);
        assert_eq!(app.context_order, vec![first, second]);
    }

    #[test]
    fn messages_only_increment_unread_for_inactive_contexts() {
        let active = SpaceId::Flock(starling::protocol::FlockId::random());
        let inactive = SpaceId::Flock(starling::protocol::FlockId::random());
        let author = iroh::SecretKey::generate().public();
        let message = ChatMessageView {
            event_hash: [2; 32],
            sender: author,
            author: "Wren".into(),
            body: "hello".into(),
            ts: 0,
        };
        let mut app = App::default();
        app.insert_context(context(active, "Active", "active-invite", None));
        app.insert_context(context(inactive, "Inactive", "inactive-invite", None));

        append_message(&mut app, active, message.clone());
        append_message(&mut app, inactive, message);

        assert_eq!(app.contexts[&active].unread, 0);
        assert_eq!(app.contexts[&inactive].unread, 1);
    }

    #[test]
    fn roost_channel_uses_parent_base_invite_not_internal_route() {
        let roost = RoostId::random();
        let parent = SpaceId::RoostChannel {
            roost,
            channel: starling::protocol::ChannelId::random(),
        };
        let channel = SpaceId::RoostChannel {
            roost,
            channel: starling::protocol::ChannelId::random(),
        };
        let mut app = App::default();
        app.insert_context(context(parent, "Roost", "public-roost-invite", None));
        app.insert_context(context(channel, "general", "roost/general", Some(roost)));
        app.select_context(channel);

        assert_eq!(app.active_base_invite(), Some("public-roost-invite"));
        assert_ne!(app.active_base_invite(), Some("roost/general"));
    }

    #[test]
    fn status_notice_expires_at_its_deadline() {
        let now = Instant::now();
        let mut app = App::default();
        app.show_status_notice("Invite copied", now);

        assert_eq!(app.visible_status_notice(now), Some("Invite copied"));
        assert!(!app.expire_status_notice(now + NOTICE_DURATION - Duration::from_millis(1)));
        assert!(app.expire_status_notice(now + NOTICE_DURATION));
        assert_eq!(app.visible_status_notice(now + NOTICE_DURATION), None);
        assert_eq!(app.status_notice_expires_at, None);
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

    #[test]
    fn presence_leases_are_isolated_by_space() {
        let now = tokio::time::Instant::now();
        let member = iroh::SecretKey::generate().public();
        let first = starling::protocol::SpaceId::Flock(starling::protocol::FlockId::random());
        let second = starling::protocol::SpaceId::Flock(starling::protocol::FlockId::random());
        let mut presence = ScopedPresence::default();
        presence.context_mut(first).apply_verified_lease(
            member,
            LiveLease {
                deadline: now + Duration::from_secs(30),
                sequence: 1,
            },
            now,
        );
        presence.context_mut(second).apply_verified_lease(
            member,
            LiveLease {
                deadline: now + Duration::from_secs(60),
                sequence: 1,
            },
            now,
        );
        presence
            .context_mut(first)
            .expire(now + Duration::from_secs(31));
        assert!(
            presence.contexts[&first]
                .live_ids(now + Duration::from_secs(31))
                .is_empty()
        );
        assert_eq!(presence.contexts[&second].live_ids(now), vec![member]);
        presence.neighbor_down(member);
        assert_eq!(presence.contexts[&second].live_ids(now), vec![member]);
    }

    #[test]
    fn expiry_removes_live_state_but_retains_profiles_and_order() {
        let now = tokio::time::Instant::now();
        let first = iroh::SecretKey::generate().public();
        let second = iroh::SecretKey::generate().public();
        let mut presence = ContextPresence::default();
        presence.set_profile(MemberProfile {
            endpoint: first,
            name: "Wren".into(),
            pronouns: "they/them".into(),
        });
        presence.apply_verified_lease(
            first,
            LiveLease {
                deadline: now + Duration::from_secs(1),
                sequence: 1,
            },
            now,
        );
        presence.apply_verified_lease(
            second,
            LiveLease {
                deadline: now + Duration::from_secs(10),
                sequence: 1,
            },
            now,
        );
        assert_eq!(presence.live_ids(now), vec![first, second]);
        presence.expire(now + Duration::from_secs(2));
        assert_eq!(
            presence.live_ids(now + Duration::from_secs(2)),
            vec![second]
        );
        assert_eq!(
            presence
                .members
                .get(&first)
                .map(|profile| profile.name.as_str()),
            Some("Wren")
        );
    }
}
