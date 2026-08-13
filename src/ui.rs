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
    pub hover: Color,
    pub active: Color,
    pub focus_ring: Color,
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
            hover: Color::Rgb(131, 194, 177),
            active: Color::Rgb(126, 189, 172),
            focus_ring: Color::Rgb(180, 230, 214),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
    pub ordered_ids: Vec<EndpointId>,
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
pub enum ContextState {
    #[allow(dead_code)]
    AwaitingKeys,
    #[allow(dead_code)]
    Reconciling,
    Ready,
    #[allow(dead_code)]
    Revoked,
    #[allow(dead_code)]
    NeedsUserAction,
    Restoring,
}

pub const MENU_ITEMS: &[&str] = &[
    "Create a Flock",
    "Create a Roost",
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
}

#[derive(Clone, Copy)]
pub enum ToolbarAction {
    Menu,
    Leave,
    #[cfg(feature = "audio")]
    Call,
    #[cfg(feature = "audio")]
    Mute,
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
    pub show_create_roost: bool,
    pub show_add_channel: bool,
    pub add_channel_input: String,
    pub create_roost_input: String,
    pub show_join_room: bool,
    pub join_input: String,
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
    pub local_video_frame: Option<RgbImage>,
    pub remote_video_frames: HashMap<EndpointId, RgbImage>,
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
    pub show_bird_profile: bool,
    pub bird_profile_peer: Option<EndpointId>,
    pub show_context_menu: bool,
    pub show_role_submenu: bool,
    pub context_menu_target: Option<ContextMenuTarget>,
    pub context_menu_selection: usize,
    pub context_menu_items: Vec<ContextMenuItem>,
    pub role_submenu_target: Option<EndpointId>,
    pub role_submenu_selection: usize,
    pub v2_view: V2View,
    pub profile_panel: LocalProfilePanel,
    pub settings_open: bool,
    pub accent_input: String,
    pub settings_tab: SettingsTab,
    pub selected_dm: Option<EndpointId>,
    pub icon_style: IconStyle,
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
            show_create_roost: false,
            show_add_channel: false,
            add_channel_input: String::new(),
            create_roost_input: String::new(),
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
            show_role_submenu: false,
            show_context_menu: false,
            context_menu_target: None,
            context_menu_selection: 0,
            role_submenu_target: None,
            role_submenu_selection: 0,
            context_menu_items: Vec::new(),
            status_notice: None,
            status_notice_expires_at: None,
            show_bird_profile: false,
            bird_profile_peer: None,
            my_perms: starling::roost::perms::Perm::empty(),
            peer_roles: HashMap::new(),
            v2_view: V2View::Home,
            profile_panel: LocalProfilePanel::default(),
            settings_open: false,
            settings_tab: SettingsTab::default(),
            accent_input: "#6FAE9D".to_string(),
            selected_dm: None,
            icon_style: IconStyle::from_env(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum V2View {
    #[default]
    Home,
    Space,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProfileField {
    #[default]
    Name,
    Avatar,
    Banner,
    AboutMe,
    Pronouns,
    Motd,
    CustomStatus,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SettingsTab {
    #[default]
    Account,
    Voice,
    Appearance,
    Notifications,
    Keybinds,
}

#[derive(Clone, Debug, Default)]
pub struct LocalProfilePanel {
    pub open: bool,
    pub editing: bool,
    pub field: ProfileField,
    pub avatar_label: String,
    pub banner: String,
    pub about_me: String,
    pub pronouns: String,
    pub motd: String,
    pub custom_status: String,
    pub draft_name: String,
    pub draft_avatar_label: String,
    pub draft_banner: String,
    pub draft_about_me: String,
    pub draft_pronouns: String,
    pub draft_motd: String,
    pub draft_custom_status: String,
}

// Keep the preference in App so every renderer uses one consistent glyph policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum IconStyle {
    NerdFont,
    #[default]
    Unicode,
    Ascii,
}

impl IconStyle {
    pub fn from_env() -> Self {
        match std::env::var("STARLING_ICON_STYLE").ok().as_deref() {
            Some("nerd") | Some("nerdfont") => Self::NerdFont,
            Some("ascii") => Self::Ascii,
            Some("unicode") => Self::Unicode,
            _ => Self::Unicode,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum TerminalIcon {
    Home,
    Text,
    Voice,
    Call,
    Video,
    Members,
}

impl TerminalIcon {
    fn nerd_font(self) -> Option<&'static str> {
        Some(match self {
            Self::Home => "\u{f015}",
            Self::Text => "\u{f075}",
            Self::Voice => "\u{f130}",
            Self::Call => "\u{f095}",
            Self::Video => "\u{f03d}",
            Self::Members => "\u{f0c0}",
        })
    }

    fn unicode(self) -> Option<&'static str> {
        Some(match self {
            Self::Home => "⌂",
            Self::Text => "▤",
            Self::Voice => "♫",
            Self::Call => "☎",
            Self::Video => "▣",
            Self::Members => "♟",
        })
    }

    fn ascii(self) -> &'static str {
        match self {
            Self::Home => "[H]",
            Self::Text => "[T]",
            Self::Voice => "[V]",
            Self::Call => "[C]",
            Self::Video => "[D]",
            Self::Members => "[M]",
        }
    }

    fn glyph(self, style: IconStyle) -> &'static str {
        match style {
            IconStyle::NerdFont => self
                .nerd_font()
                .or_else(|| self.unicode())
                .unwrap_or_else(|| self.ascii()),
            IconStyle::Unicode => self.unicode().unwrap_or_else(|| self.ascii()),
            IconStyle::Ascii => self.ascii(),
        }
    }
}

fn icon_span(icon: TerminalIcon, style: IconStyle, color: Color) -> Span<'static> {
    Span::styled(format!("{} ", icon.glyph(style)), Style::new().fg(color))
}

fn initials(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "?".to_string();
    }
    let mut chars = trimmed.chars();
    let first = chars.next().unwrap().to_uppercase().to_string();
    let second = trimmed
        .split_whitespace()
        .nth(1)
        .and_then(|w| w.chars().next())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    format!("{first}{second}")
}

fn author_color(name: &str) -> Color {
    const PALETTE: [Color; 8] = [
        Color::Rgb(88, 101, 242),
        Color::Rgb(35, 165, 90),
        Color::Rgb(240, 178, 50),
        Color::Rgb(242, 63, 67),
        Color::Rgb(235, 69, 158),
        Color::Rgb(0, 175, 244),
        Color::Rgb(255, 115, 250),
        Color::Rgb(128, 132, 142),
    ];
    let hash = name
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    PALETTE[(hash as usize) % PALETTE.len()]
}

fn format_ts(ts: i64) -> String {
    use chrono::TimeZone;
    let secs = if ts > 10_000_000_000 { ts / 1000 } else { ts };
    chrono::Utc
        .timestamp_opt(secs, 0)
        .single()
        .map(|dt| dt.format("%H:%M").to_string())
        .unwrap_or_default()
}

pub enum Popup {
    DeleteConfirm,
    CreateRoom,
    CreateRoost,
    AddChannel,
    EditFlock,
    JoinRoom,
    Menu,
    BirdProfile,
    ContextMenu,
    RoleSubmenu,
    None,
}

/// What was right-clicked to open the context menu.
#[derive(Clone, Debug)]
pub enum ContextMenuTarget {
    Roost(usize),
    RoostChannel(usize, usize),
    Bird(EndpointId),
}

/// What the context menu triggers when selected.
#[derive(Clone, Debug)]
pub enum ContextMenuAction {
    AddChannel,
    RemoveChannel,
    Invite,
    Kick,
    Ban,
    SetRole,
    RemoveRoles,
    TransferOwnership,
    DeleteMessage,
}

/// One entry in the context menu.
#[derive(Clone, Debug)]
pub struct ContextMenuItem {
    pub label: String,
    pub action: ContextMenuAction,
    pub enabled: bool,
}

impl App {
    /// Resolve the single active popup using the same precedence as the old
    /// `if`-cascade: delete-confirm wins over create-room, which wins over
    /// edit-flock, join-room, and the menu, in that order. Only the highest-
    pub fn active_popup(&self) -> Popup {
        if self.show_role_submenu {
            Popup::RoleSubmenu
        } else if self.show_context_menu {
            Popup::ContextMenu
        } else if self.show_delete_confirm {
            Popup::DeleteConfirm
        } else if self.show_add_channel {
            Popup::AddChannel
        } else if self.show_create_roost {
            Popup::CreateRoost
        } else if self.show_create_room {
            Popup::CreateRoom
        } else if self.show_edit_flock {
            Popup::EditFlock
        } else if self.show_join_room {
            Popup::JoinRoom
        } else if self.show_menu {
            Popup::Menu
        } else if self.show_bird_profile {
            Popup::BirdProfile
        } else {
            Popup::None
        }
    }

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

    #[cfg(test)]
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

    /// Remove the active flock/roost from the in-memory state. Returns the
    /// base join code (so the caller can send [`Command::Leave`] to tear down
    /// the gossip subscription) and a display title for a status message, or
    /// `None` when no context is active or it has no join code to leave by.
    pub fn leave_active_context(&mut self) -> Option<(String, String)> {
        let active = self.active?;
        let context = self.contexts.get(&active)?;
        let code = context.base_invite_display.clone()?;
        let title = context.title.clone();
        let is_roost = context.roost.is_some();

        // Remove every context sharing this base join code: a roost stores
        // one context per channel, all keyed by the same roost code.
        let leaving: Vec<SpaceId> = self
            .contexts
            .iter()
            .filter(|(_, c)| c.base_invite_display.as_deref() == Some(code.as_str()))
            .map(|(id, _)| *id)
            .collect();
        for id in &leaving {
            self.context_order.retain(|x| x != id);
            self.contexts.remove(id);
            self.presence.contexts.remove(id);
        }
        if is_roost {
            self.roosts.retain(|r| r.code != code);
        } else {
            self.flocks.retain(|f| f.code != code);
        }

        // Move the active selection to the next remaining context, if any.
        self.active = self.context_order.first().copied();
        self.selection = Selection::default();
        Some((code, title))
    }

    pub fn active_context(&self) -> Option<&ContextView> {
        self.active.and_then(|id| self.contexts.get(&id))
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

    /// The `flocks` routing key to send to for the active context. This differs
    /// from [`active_code`] (which returns the base invite for display/copy):
    /// a roost channel routes on its full `{roost_code}/{channel}` key, which
    /// the net layer stores under the same string, while a flock routes on its
    /// join code. Using `active_code()` for a roost channel returns the base
    /// roost code, which is not a `flocks` key, so the send would be silently
    /// dropped and the outgoing message would never be echoed back for display.
    pub fn active_send_code(&self) -> Option<String> {
        let context = self.active_context()?;
        if context.roost.is_some() {
            Some(context.title.clone())
        } else {
            context.base_invite_display.clone()
        }
    }

    pub fn active_messages(&self) -> &[MessageView] {
        // For typed contexts, delegate to the legacy FlockView where
        // messages are actually stored (roost channels live in
        // roosts[].channels; flocks live in flocks).
        if let Some(ctx) = self.active_context() {
            if ctx.roost.is_some()
                && let Some(channel) =
                    flattened_channels(self).find(|channel| channel.code == ctx.title)
            {
                return &channel.messages;
            } else if let Some(secret) = &ctx.secret
                && let Some(fv) = self.flocks.iter().find(|f| f.code == *secret)
            {
                return &fv.messages;
            }
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
            if context.roost.is_some() {
                // Render roost channels as "RoostName #channel" rather than the
                // raw `{roost_code}/{channel}` routing key, so a channel does
                // not look like a flock-code entry in the message header.
                if let Some(roost_code) = context.base_invite_display.as_deref()
                    && let Some(rv) = self.roosts.iter().find(|r| r.code == roost_code)
                {
                    let rn = if rv.name.is_empty() {
                        &rv.code
                    } else {
                        &rv.name
                    };
                    if let Some(ch) = rv.channels.iter().find(|c| c.code == context.title) {
                        return format!("{rn} #{}", ch.name);
                    }
                    return format!("{rn} #{}", context.title);
                }
            }
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
        self.v2_view = V2View::Space;
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

    pub fn open_home(&mut self) {
        self.v2_view = V2View::Home;
        self.active = None;
    }

    pub fn toggle_expand(&mut self, ri: usize) {
        if !self.expanded.remove(&ri) {
            self.expanded.insert(ri);
        }
    }

    pub fn active_peers(&self) -> Vec<EndpointId> {
        self.active_context()
            .and_then(|ctx| self.presence.contexts.get(&ctx.id))
            .map(|presence| presence.ordered_ids.clone())
            .unwrap_or_default()
    }

    pub fn bird_count(&self) -> usize {
        self.active_peers().len() + 1
    }

    pub fn select_next_peer(&mut self) {
        let peers = self.active_peers();
        if !peers.is_empty() {
            self.selected_peer = (self.selected_peer + 1) % peers.len();
        }
    }

    pub fn selected_peer_id(&self) -> Option<EndpointId> {
        self.active_peers().get(self.selected_peer).copied()
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

    /// Build the context menu items for a given target
    pub fn build_context_menu(&mut self, target: ContextMenuTarget) {
        self.context_menu_target = Some(target.clone());
        self.context_menu_selection = 0;
        self.context_menu_items = Vec::new();

        let perms = self.my_perms;

        match target {
            ContextMenuTarget::Roost(ri) => {
                if let Some(rv) = self.roosts.get(ri) {
                    if perms.contains(starling::roost::perms::Perm::MANAGE_CHANS) {
                        self.context_menu_items.push(ContextMenuItem {
                            label: "Add Channel".into(),
                            action: ContextMenuAction::AddChannel,
                            enabled: true,
                        });
                        self.context_menu_items.push(ContextMenuItem {
                            label: "Remove Channel".into(),
                            action: ContextMenuAction::RemoveChannel,
                            enabled: !rv.channels.is_empty(),
                        });
                    }
                    if perms.contains(starling::roost::perms::Perm::INVITE) {
                        self.context_menu_items.push(ContextMenuItem {
                            label: "Invite Bird".into(),
                            action: ContextMenuAction::Invite,
                            enabled: true,
                        });
                    }
                }
            }
            ContextMenuTarget::RoostChannel(_ri, _ci) => {
                if perms.contains(starling::roost::perms::Perm::MANAGE_MSGS) {
                    self.context_menu_items.push(ContextMenuItem {
                        label: "Delete Message".into(),
                        action: ContextMenuAction::DeleteMessage,
                        enabled: true,
                    });
                }
            }
            ContextMenuTarget::Bird(endpoint) => {
                let is_self = self.node_id == Some(endpoint);
                if is_self {
                    return;
                }
                if perms.contains(starling::roost::perms::Perm::KICK) {
                    self.context_menu_items.push(ContextMenuItem {
                        label: "Kick".into(),
                        action: ContextMenuAction::Kick,
                        enabled: true,
                    });
                }
                if perms.contains(starling::roost::perms::Perm::BAN) {
                    self.context_menu_items.push(ContextMenuItem {
                        label: "Ban".into(),
                        action: ContextMenuAction::Ban,
                        enabled: true,
                    });
                }
                if perms.contains(starling::roost::perms::Perm::MANAGE_ROLES) {
                    self.context_menu_items.push(ContextMenuItem {
                        label: "Set Role".into(),
                        action: ContextMenuAction::SetRole,
                        enabled: !self.my_perms.is_empty(),
                    });
                    self.context_menu_items.push(ContextMenuItem {
                        label: "Remove All Roles".into(),
                        action: ContextMenuAction::RemoveRoles,
                        enabled: true,
                    });
                }
                if perms.contains(starling::roost::perms::Perm::ADMIN) {
                    self.context_menu_items.push(ContextMenuItem {
                        label: "Transfer Ownership".into(),
                        action: ContextMenuAction::TransferOwnership,
                        enabled: true,
                    });
                }
            }
        }
    }

    /// Get the roost endpoint for the current context menu target
    pub fn context_menu_roost_endpoint(&self) -> Option<EndpointId> {
        let target = self.context_menu_target.as_ref()?;
        match target {
            ContextMenuTarget::Roost(ri) => {
                let rv = self.roosts.get(*ri)?;
                starling::net::decode_typed_code(&rv.code)
                    .and_then(|t| starling::net::typed_code_node_id(&t))
            }
            ContextMenuTarget::RoostChannel(ri, _) => {
                let rv = self.roosts.get(*ri)?;
                starling::net::decode_typed_code(&rv.code)
                    .and_then(|t| starling::net::typed_code_node_id(&t))
            }
            ContextMenuTarget::Bird(_) => {
                let ctx = self.active_context()?;
                let roost_id = ctx.roost?;
                Some(EndpointId::from_bytes(&roost_id.0).ok()?)
            }
        }
    }
}

pub fn toolbar_buttons(app: &App) -> Vec<(ToolbarAction, &'static str, u16, u16)> {
    let labels: Vec<(ToolbarAction, &'static str)> = std::iter::once((
        ToolbarAction::Menu,
        if app.show_menu { "Close" } else { "Menu" },
    ))
    .chain(std::iter::once((ToolbarAction::Leave, "Leave")))
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
            std::iter::empty()
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

pub fn shade_color(color: Color, percent: i16) -> Color {
    let Color::Rgb(r, g, b) = color else {
        return color;
    };
    let shade = |value: u8| -> u8 { (i16::from(value) + percent).clamp(0, 255) as u8 };
    Color::Rgb(shade(r), shade(g), shade(b))
}

pub fn apply_accent_color(app: &mut App, hex: &str) -> bool {
    let Some(accent) = hex_to_color(hex) else {
        return false;
    };
    app.palette.accent = accent;
    app.palette.selection = shade_color(accent, 35);
    app.palette.invite = shade_color(accent, 15);
    app.palette.border = shade_color(accent, -45);
    app.palette.hover = shade_color(accent, 20);
    app.palette.active = shade_color(accent, 10);
    app.palette.focus_ring = shade_color(accent, 55);
    app.accent_input = hex.to_ascii_uppercase();
    true
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

    let columns = Layout::horizontal([
        Constraint::Length(12),
        Constraint::Length(28),
        Constraint::Min(1),
        Constraint::Length(27),
    ])
    .split(chunks[1]);

    draw_server_rail(f, app, columns[0]);
    draw_sidebar(f, app, columns[1]);
    draw_chat(f, app, columns[2]);
    if matches!(app.v2_view, V2View::Space) {
        draw_members(f, app, columns[3]);
    }
    draw_button_bar(f, app, chunks[2]);
    f.render_widget(
        Paragraph::new(app.input.as_str())
            .block(Block::default().borders(Borders::ALL).title(" message ")),
        chunks[3],
    );
    if app.in_call {
        draw_call_overlay(f, app);
    }
    if app.profile_panel.open {
        draw_profile_modal(f, app);
    }
    if app.settings_open {
        draw_settings_modal(f, app);
    }
    if app.show_role_submenu {
        draw_role_submenu(f, app);
    } else if app.show_context_menu {
        draw_context_menu(f, app);
    } else if app.show_add_channel {
        draw_add_channel_popup(f, app);
    } else if app.show_create_roost {
        draw_create_roost_popup(f, app);
    } else if app.show_create_room {
        draw_create_room_popup(f, app);
    } else if app.show_edit_flock {
        draw_edit_flock_popup(f, app);
    } else if app.show_join_room {
        draw_join_room_popup(f, app);
    } else if app.show_delete_confirm {
        draw_delete_confirm_popup(f, app);
    } else if app.show_menu {
        draw_menu_popup(f, app);
    } else if app.show_bird_profile {
        draw_bird_profile_popup(f, app);
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

fn draw_server_rail(f: &mut Frame, app: &App, area: Rect) {
    let mut items = Vec::new();
    let home_style = if matches!(app.v2_view, V2View::Home) {
        Style::new()
            .fg(app.palette.selection)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(app.palette.text)
    };
    items.push(ListItem::new(Line::from(vec![
        Span::styled("> ", Style::new().fg(app.palette.accent)),
        icon_span(TerminalIcon::Home, app.icon_style, app.palette.accent),
        Span::styled("HOME", home_style),
    ])));

    for (index, flock) in app.flocks.iter().enumerate() {
        let selected = matches!(app.selection, Selection::Flock(i) if i == index)
            && matches!(app.v2_view, V2View::Space);
        let label = if flock.name.is_empty() {
            "FLOCK"
        } else {
            flock.name.as_str()
        };
        let unread = if flock.unread > 0 {
            format!(" <{}>", flock.unread)
        } else {
            String::new()
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                if selected { "> " } else { "  " },
                Style::new().fg(app.palette.accent),
            ),
            Span::styled(
                format!("[{}]", label),
                Style::new().fg(if selected {
                    app.palette.selection
                } else {
                    app.palette.text
                }),
            ),
            Span::styled(
                unread,
                Style::new()
                    .fg(Color::Rgb(242, 63, 67))
                    .add_modifier(Modifier::BOLD),
            ),
        ])));
    }

    for (index, roost) in app.roosts.iter().enumerate() {
        let selected = matches!(app.selection, Selection::Channel(ri, _) if ri == index)
            && matches!(app.v2_view, V2View::Space);
        let label = if roost.name.is_empty() {
            "ROOST"
        } else {
            roost.name.as_str()
        };
        let unread = if roost.unread > 0 {
            format!(" <{}>", roost.unread)
        } else {
            String::new()
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                if selected { "> " } else { "  " },
                Style::new().fg(app.palette.accent),
            ),
            Span::styled(
                format!("[{}]", label),
                Style::new().fg(if selected {
                    app.palette.selection
                } else {
                    app.palette.text
                }),
            ),
            Span::styled(
                unread,
                Style::new()
                    .fg(Color::Rgb(242, 63, 67))
                    .add_modifier(Modifier::BOLD),
            ),
        ])));
    }
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(" servers ")),
        area,
    );
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let mut items = vec![ListItem::new(Span::styled(
        if matches!(app.v2_view, V2View::Home) {
            " DIRECT MESSAGES "
        } else {
            " CHANNELS "
        },
        Style::new().fg(app.palette.dim),
    ))];

    if matches!(app.v2_view, V2View::Home) {
        for peer in app
            .peers
            .iter()
            .take(area.height.saturating_sub(3) as usize)
        {
            let name = app.peer_display_name(peer);
            let selected = app.selected_dm == Some(*peer);
            let status_color = match app.peer_status.get(peer) {
                Some(BirdStatus::Online) => Color::Rgb(35, 165, 90),
                Some(BirdStatus::Idle) => Color::Rgb(240, 178, 50),
                Some(BirdStatus::InCall) => app.palette.accent,
                None => app.palette.dim,
            };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{} ", initials(&name)),
                    Style::new().fg(status_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    name,
                    Style::new().fg(if selected {
                        app.palette.active
                    } else {
                        app.palette.text
                    }),
                ),
            ])));
        }
    } else {
        match app.selection {
            Selection::Flock(_) => items.push(ListItem::new(Span::styled(
                " [TEXT] general ",
                Style::new().fg(app.palette.selection),
            ))),
            Selection::Channel(ri, ci) => {
                if let Some(roost) = app.roosts.get(ri) {
                    if !roost.channels.is_empty() {
                        items.push(ListItem::new(Span::styled(
                            " TEXT CHANNELS ",
                            Style::new()
                                .fg(app.palette.dim)
                                .add_modifier(Modifier::BOLD),
                        )));
                    }
                    for (index, channel) in roost.channels.iter().enumerate() {
                        let selected = index == ci;
                        let mut spans = vec![icon_span(
                            TerminalIcon::Text,
                            app.icon_style,
                            app.palette.channel,
                        )];
                        spans.push(Span::styled(
                            channel.name.clone(),
                            Style::new().fg(if selected {
                                app.palette.selection
                            } else {
                                app.palette.text
                            }),
                        ));
                        if channel.unread > 0 {
                            spans.push(Span::styled(
                                format!(" ({})", channel.unread),
                                Style::new().fg(app.palette.selection),
                            ));
                        }
                        items.push(ListItem::new(Line::from(spans)));
                    }
                }
            }
        }
    }
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(" sidebar ")),
        chunks[0],
    );
    let footer = Line::from(vec![
        Span::styled(
            format!("{} ", initials(&app.name)),
            Style::new()
                .fg(app.palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            app.name.clone(),
            Style::new()
                .fg(app.palette.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            if app.muted { "[UNMUTE]" } else { "[MUTE]" },
            Style::new().fg(if app.muted {
                Color::Rgb(242, 63, 67)
            } else {
                app.palette.dim
            }),
        ),
        Span::raw(" "),
        Span::styled("[SETTINGS]", Style::new().fg(app.palette.dim)),
    ]);
    f.render_widget(footer, chunks[1]);
}

fn flattened_channels(app: &App) -> impl Iterator<Item = &FlockView> {
    app.roosts.iter().flat_map(|roost| roost.channels.iter())
}

fn draw_chat(f: &mut Frame, app: &App, area: Rect) {
    let title = match app.v2_view {
        V2View::Home => app
            .selected_dm
            .map(|peer| format!("DM: {}", app.peer_display_name(&peer)))
            .unwrap_or_else(|| "Direct messages".to_string()),
        V2View::Space => app.active_title(),
    };
    let topic = match app.v2_view {
        V2View::Home => if app.selected_dm.is_some() {
            "Direct conversation"
        } else {
            "Select a peer from the sidebar"
        }
        .to_string(),
        V2View::Space => "Messages and presence".to_string(),
    };
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            title,
            Style::new()
                .fg(app.palette.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(topic, Style::new().fg(app.palette.dim)),
    ]))
    .block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(header, Rect::new(area.x, area.y, area.width, 2));

    let rows = match app.v2_view {
        V2View::Home => vec![ListItem::new(Span::styled(
            if app.selected_dm.is_some() {
                "No direct messages yet."
            } else {
                "Select a peer to begin a direct message."
            },
            Style::new().fg(app.palette.dim),
        ))],
        V2View::Space => app
            .active_messages()
            .iter()
            .map(|message| {
                let color = author_color(&message.msg.author);
                let header = Line::from(vec![
                    Span::styled(
                        format!("{} ", initials(&message.msg.author)),
                        Style::new().fg(color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        message.msg.author.clone(),
                        Style::new().fg(color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  {}", format_ts(message.msg.ts)),
                        Style::new().fg(app.palette.dim),
                    ),
                ]);
                let mut body_spans = Vec::new();
                if message.private {
                    body_spans.push(Span::styled("🔒 ", Style::new().fg(app.palette.dim)));
                }
                body_spans.push(Span::styled(
                    message.msg.body.clone(),
                    Style::new().fg(app.palette.text),
                ));
                ListItem::new(Text::from(vec![header, Line::from(body_spans)]))
            })
            .collect::<Vec<_>>(),
    };
    let body = Rect::new(
        area.x,
        area.y + 2,
        area.width,
        area.height.saturating_sub(2),
    );
    f.render_widget(
        List::new(rows).block(Block::default().borders(Borders::ALL).title(" chat ")),
        body,
    );
}

fn draw_members(f: &mut Frame, app: &App, area: Rect) {
    let mut items = Vec::new();
    items.push(ListItem::new(Line::from(vec![
        icon_span(TerminalIcon::Members, app.icon_style, app.palette.accent),
        Span::styled(
            "MEMBERS",
            Style::new()
                .fg(app.palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
    ])));
    items.push(ListItem::new(Line::from(vec![
        Span::styled(
            format!("{} ", initials(&app.name)),
            Style::new()
                .fg(app.palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{} (you)", app.name),
            Style::new().fg(app.palette.selection),
        ),
    ])));

    let peers = app.active_peers();
    let mut online = Vec::new();
    let mut idle = Vec::new();
    let mut in_call = Vec::new();
    for peer in peers.iter().copied() {
        match app
            .peer_status
            .get(&peer)
            .copied()
            .unwrap_or(BirdStatus::Online)
        {
            BirdStatus::Online => online.push(peer),
            BirdStatus::Idle => idle.push(peer),
            BirdStatus::InCall => in_call.push(peer),
        }
    }
    let groups = [("Online", &online), ("Idle", &idle), ("In Call", &in_call)];
    for (label, members) in groups {
        if members.is_empty() {
            continue;
        }
        items.push(ListItem::new(Span::styled(
            format!("{label} — {}", members.len()),
            Style::new()
                .fg(app.palette.dim)
                .add_modifier(Modifier::BOLD),
        )));
        for peer in members {
            let name = app.peer_display_name(peer);
            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{} ", initials(&name)),
                    Style::new().fg(app.palette.text),
                ),
                Span::styled(name, Style::new().fg(app.palette.text)),
            ])));
        }
    }
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(" members ")),
        area,
    );
}

fn status_text(app: &App) -> String {
    if let Some(notice) = app.visible_status_notice(Instant::now()) {
        notice.to_string()
    } else if app.joining.is_some() {
        "Joining...".into()
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
            ToolbarAction::Leave => app.active_context().is_some(),
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

fn draw_call_overlay(f: &mut Frame, app: &App) {
    let area = centered(f.area(), 62, 14);
    f.render_widget(Clear, area);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(" CALL ")
            .border_style(Style::new().fg(app.palette.accent)),
        area,
    );
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(inner);
    f.render_widget(Paragraph::new(app.active_title()), rows[0]);
    let frames_available = app.local_video_frame.is_some() || !app.remote_video_frames.is_empty();
    if app.show_video && frames_available {
        let mut tiles: Vec<(String, Option<&RgbImage>)> = Vec::new();
        if let Some(frame) = app.local_video_frame.as_ref() {
            tiles.push((format!("{} (you)", app.name), Some(frame)));
        }
        for (peer, frame) in &app.remote_video_frames {
            tiles.push((app.peer_display_name(peer), Some(frame)));
        }
        draw_video_tiles(f, tiles, rows[1]);
    } else {
        let tiles = app
            .peers
            .iter()
            .map(|peer| {
                let name = app.peer_display_name(peer);
                let video = if app.show_video { " VIDEO" } else { " VOICE" };
                ListItem::new(Line::from(vec![
                    Span::styled(name, Style::new().fg(app.palette.text)),
                    Span::styled(video, Style::new().fg(app.palette.dim)),
                ]))
            })
            .collect::<Vec<_>>();
        f.render_widget(
            List::new(tiles).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" participants "),
            ),
            rows[1],
        );
    }
    let mute = if app.muted { "UNMUTE" } else { "MUTE" };
    let video = if app.show_video {
        "VIDEO OFF"
    } else {
        "VIDEO ON"
    };
    let controls = Line::from(vec![
        icon_span(TerminalIcon::Voice, app.icon_style, app.palette.accent),
        Span::styled(mute, Style::new().fg(app.palette.text)),
        Span::raw("   "),
        icon_span(TerminalIcon::Video, app.icon_style, app.palette.accent),
        Span::styled(video, Style::new().fg(app.palette.text)),
        Span::raw("   "),
        icon_span(TerminalIcon::Call, app.icon_style, app.palette.accent),
        Span::styled("HANG UP", Style::new().fg(app.palette.text)),
    ]);
    f.render_widget(controls, rows[2]);
}

fn draw_video_tiles(f: &mut Frame, tiles: Vec<(String, Option<&RgbImage>)>, area: Rect) {
    if tiles.is_empty() {
        return;
    }
    let mut columns = 1usize;
    while columns * columns < tiles.len() {
        columns += 1;
    }
    let grid_rows = tiles.len().div_ceil(columns);
    let row_areas =
        Layout::vertical(vec![Constraint::Ratio(1, grid_rows as u32); grid_rows]).split(area);
    for (row_idx, row_area) in row_areas.iter().enumerate() {
        let col_areas = Layout::horizontal(vec![Constraint::Ratio(1, columns as u32); columns])
            .split(*row_area);
        for (col_idx, tile_area) in col_areas.iter().enumerate() {
            let idx = row_idx * columns + col_idx;
            let Some((name, frame)) = tiles.get(idx) else {
                continue;
            };
            let block = Block::default().borders(Borders::ALL).title(name.as_str());
            let inner = block.inner(*tile_area);
            f.render_widget(block, *tile_area);
            if let Some(frame) = frame {
                let lines = crate::video::frame_to_lines(frame, inner.width, inner.height);
                f.render_widget(Paragraph::new(lines), inner);
            }
        }
    }
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

fn draw_settings_modal(f: &mut Frame, app: &App) {
    let area = centered(f.area(), 70, 18);
    f.render_widget(Clear, area);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(" SETTINGS ")
            .border_style(Style::new().fg(app.palette.accent)),
        area,
    );
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let columns = Layout::horizontal([Constraint::Length(18), Constraint::Min(1)]).split(inner);

    let tabs: Vec<ListItem> = [
        (SettingsTab::Account, "My Account"),
        (SettingsTab::Voice, "Voice & Video"),
        (SettingsTab::Appearance, "Appearance"),
        (SettingsTab::Notifications, "Notifications"),
        (SettingsTab::Keybinds, "Keybinds"),
    ]
    .iter()
    .map(|(tab, label)| {
        let selected = app.settings_tab == *tab;
        let style = if selected {
            Style::new()
                .fg(app.palette.selection)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(app.palette.text)
        };
        ListItem::new(Span::styled(label.to_string(), style))
    })
    .collect();
    f.render_widget(List::new(tabs), columns[0]);

    let content: String = match app.settings_tab {
        SettingsTab::Account => {
            "Display name\n  you\n\nEmail\n  you@starling.local".to_string()
        }
        SettingsTab::Voice => {
            "Input device\n  Default\n\nOutput device\n  Default\n\nPush to Talk\n  Off\n\nNoise suppression\n  On".to_string()
        }
        SettingsTab::Appearance => format!(
            "Theme\n  Dark\n\nAccent color\n  {}\n\nCompact mode\n  Off\n\nShow avatars\n  On\n\n\nEnter a #RRGGBB value.\n[ENTER APPLY]  [ESC CLOSE]",
            app.accent_input
        ),
        SettingsTab::Notifications => {
            "Desktop notifications\n  On\n\nMute @everyone\n  Off\n\nSounds\n  On".to_string()
        }
        SettingsTab::Keybinds => {
            "Mark server read\n  Shift + Esc\n\nToggle mute\n  Ctrl + Shift + M\n\nToggle deafen\n  Ctrl + Shift + D\n\nAnswer call\n  Ctrl + Enter".to_string()
        }
    };
    f.render_widget(Paragraph::new(content), columns[1]);
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

fn draw_create_roost_popup(f: &mut Frame, app: &App) {
    let popup = centered(f.area(), 60, 8);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(app.palette.border))
            .title(Span::styled(
                " Create a Roost ",
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
        Paragraph::new("Roost name:").style(Style::new().fg(app.palette.text)),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(format!(" {}_", app.create_roost_input))
            .style(Style::new().fg(app.palette.selection)),
        rows[1],
    );
    f.render_widget(
        Paragraph::new("Press Enter to create, Esc to cancel.")
            .style(Style::new().fg(app.palette.dim)),
        rows[2],
    );
}

fn draw_add_channel_popup(f: &mut Frame, app: &App) {
    let popup = centered(f.area(), 60, 8);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(app.palette.border))
            .title(Span::styled(
                " Add Channel ",
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
        Paragraph::new("Channel name:").style(Style::new().fg(app.palette.text)),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(format!(" {}_", app.add_channel_input))
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

fn draw_bird_profile_popup(f: &mut Frame, app: &App) {
    let peer = match app.bird_profile_peer {
        Some(id) => id,
        None => return,
    };
    let profile = app
        .active_context()
        .and_then(|ctx| app.presence.contexts.get(&ctx.id))
        .and_then(|presence| presence.members.get(&peer));
    let name = profile.map(|p| p.name.as_str()).unwrap_or("Unknown");
    let pronouns = profile.and_then(|p| {
        if p.pronouns.is_empty() {
            None
        } else {
            Some(p.pronouns.as_str())
        }
    });

    let popup = centered(f.area(), 40, 7);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(app.palette.border))
            .title(Span::styled(
                format!(" {name} "),
                Style::new()
                    .fg(app.palette.accent)
                    .add_modifier(Modifier::BOLD),
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

    let pronouns_text = pronouns.unwrap_or("—");
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Pronouns: ", Style::new().fg(app.palette.dim)),
            Span::styled(pronouns_text, Style::new().fg(app.palette.text)),
        ])),
        rows[0],
    );

    let id_short = peer.to_string();
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("ID: ", Style::new().fg(app.palette.dim)),
            Span::styled(
                &id_short[..24.min(id_short.len())],
                Style::new().fg(app.palette.dim),
            ),
        ])),
        rows[1],
    );

    #[cfg(feature = "audio")]
    {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "[ Call ]",
                Style::new()
                    .fg(app.palette.selection)
                    .add_modifier(Modifier::BOLD),
            )])),
            rows[2],
        );
    }

    f.render_widget(
        Paragraph::new(" Enter/C = call . Esc = close").style(Style::new().fg(app.palette.dim)),
        rows[3],
    );
}

fn draw_profile_modal(f: &mut Frame, app: &App) {
    let area = centered(f.area(), 70, 22);
    f.render_widget(Clear, area);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(" PROFILE ")
            .border_style(Style::new().fg(app.palette.accent)),
        area,
    );
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let p = &app.profile_panel;
    let avatar = if p.avatar_label.is_empty() {
        "STARLING"
    } else {
        p.avatar_label.as_str()
    };
    let lines = if p.editing {
        vec![
            format!("Banner: {}", p.draft_banner),
            format!("Avatar: {}", p.draft_avatar_label),
            format!("Name: {}", p.draft_name),
            format!("Status: {}", p.draft_custom_status),
            format!("About Me: {}", p.draft_about_me),
            format!("Pronouns: {}", p.draft_pronouns),
            format!("MOTD: {}", p.draft_motd),
            format!(
                "Editing: {:?}  [TAB FIELD] [ENTER SAVE] [ESC CANCEL]",
                p.field
            ),
        ]
    } else {
        vec![
            format!("{}  {}  [{}]", avatar, p.draft_name, p.custom_status),
            format!("Banner: {}", p.banner),
            format!("About Me: {}", p.about_me),
            format!("Pronouns: {}", p.pronouns),
            format!("MOTD: {}", p.motd),
            "[E EDIT]  [ESC CLOSE]".to_string(),
        ]
    };
    f.render_widget(Paragraph::new(lines.join("\n")), inner);
}

fn draw_context_menu(f: &mut Frame, app: &App) {
    let items: Vec<ListItem> = app
        .context_menu_items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let sel = i == app.context_menu_selection;
            let style = if sel {
                Style::new()
                    .fg(app.palette.selection)
                    .add_modifier(Modifier::BOLD)
            } else if item.enabled {
                Style::new().fg(app.palette.text)
            } else {
                Style::new().fg(app.palette.dim)
            };
            let prefix = if sel { "> " } else { "  " };
            ListItem::new(Line::from(Span::styled(
                format!("{prefix}{}", item.label),
                style,
            )))
        })
        .collect();

    let width = 28u16;
    let height = items.len() as u16 + 2;
    let popup = centered(f.area(), width, height);
    f.render_widget(Clear, popup);
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(app.palette.border))
                .title(Span::styled(
                    " Actions ",
                    Style::new().fg(app.palette.accent),
                )),
        ),
        popup,
    );
}

fn draw_role_submenu(f: &mut Frame, app: &App) {
    let roles: Vec<&str> = app
        .roosts
        .iter()
        .find(|r| !r.code.is_empty())
        .map(|_| vec!["Moderator", "Member"])
        .unwrap_or_default();

    let items: Vec<ListItem> = roles
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let sel = i == app.role_submenu_selection;
            let style = if sel {
                Style::new()
                    .fg(app.palette.selection)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(app.palette.text)
            };
            let prefix = if sel { "> " } else { "  " };
            ListItem::new(Line::from(Span::styled(format!("{prefix}{name}"), style)))
        })
        .collect();

    let width = 24u16;
    let height = items.len() as u16 + 2;
    let popup = centered(f.area(), width, height);
    f.render_widget(Clear, popup);
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(app.palette.border))
                .title(Span::styled(
                    " Assign Role ",
                    Style::new().fg(app.palette.accent),
                )),
        ),
        popup,
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

    #[test]
    fn leave_active_context_drops_context_and_flock_view() {
        let flock_id = SpaceId::Flock(starling::protocol::FlockId::random());
        let other_id = SpaceId::Flock(starling::protocol::FlockId::random());
        let mut app = App::default();
        app.insert_context(context(flock_id, "Night Birds", "NIGHT-CODE", None));
        app.insert_context(context(other_id, "Day Birds", "DAY-CODE", None));
        app.flocks.push(FlockView {
            code: "NIGHT-CODE".into(),
            name: "Night Birds".into(),
            messages: vec![],
            unread: 0,
        });
        app.flocks.push(FlockView {
            code: "DAY-CODE".into(),
            name: "Day Birds".into(),
            messages: vec![],
            unread: 0,
        });
        assert!(app.select_context(flock_id));

        let (code, title) = app.leave_active_context().expect("active context leaves");
        assert_eq!(code, "NIGHT-CODE");
        assert_eq!(title, "Night Birds");

        // The flock's context and view are gone; the other flock remains and
        // is now active.
        assert!(!app.contexts.contains_key(&flock_id));
        assert!(!app.context_order.contains(&flock_id));
        assert!(app.contexts.contains_key(&other_id));
        assert_eq!(app.active, Some(other_id));
        assert!(app.flocks.iter().all(|f| f.code != "NIGHT-CODE"));
        assert!(app.flocks.iter().any(|f| f.code == "DAY-CODE"));
    }

    #[test]
    fn leave_active_context_removes_every_roost_channel() {
        let roost = starling::protocol::RoostId::random();
        let general = SpaceId::RoostChannel {
            roost,
            channel: starling::protocol::ChannelId::random(),
        };
        let random_ch = SpaceId::RoostChannel {
            roost,
            channel: starling::protocol::ChannelId::random(),
        };
        let mut app = App::default();
        app.insert_context(context(general, "Nest/general", "NEST-CODE", Some(roost)));
        app.insert_context(context(random_ch, "Nest/random", "NEST-CODE", Some(roost)));
        app.roosts.push(RoostView {
            code: "NEST-CODE".into(),
            name: "Nest".into(),
            channels: vec![
                FlockView {
                    code: "NEST-CODE/general".into(),
                    name: "general".into(),
                    messages: vec![],
                    unread: 0,
                },
                FlockView {
                    code: "NEST-CODE/random".into(),
                    name: "random".into(),
                    messages: vec![],
                    unread: 0,
                },
            ],
            unread: 0,
        });
        assert!(app.select_context(general));

        let (code, title) = app.leave_active_context().expect("roost leaves");
        assert_eq!(code, "NEST-CODE");
        assert_eq!(title, "Nest/general");

        // Both channel contexts and the roost view are removed; nothing stays
        // active.
        assert!(!app.contexts.contains_key(&general));
        assert!(!app.contexts.contains_key(&random_ch));
        assert!(app.context_order.is_empty());
        assert!(app.roosts.is_empty());
        assert_eq!(app.active, None);
    }

    #[test]
    fn leave_active_context_returns_none_without_an_active_context() {
        let mut app = App::default();
        assert!(app.leave_active_context().is_none());
    }

    #[test]
    fn active_send_code_routes_roost_channels_on_the_full_channel_key() {
        let roost = starling::protocol::RoostId::random();
        let chan = starling::protocol::ChannelId::random();
        let space = SpaceId::RoostChannel {
            roost,
            channel: chan,
        };
        let mut app = App::default();
        app.insert_context(ContextView {
            id: space,
            title: "ROOST-CODE/general".into(),
            roost: Some(roost),
            base_invite_display: Some("ROOST-CODE".into()),
            messages: Vec::new(),
            unread: 0,
            state: ContextState::Ready,
            secret: Some("ROOST-CODE".into()),
        });
        app.active = Some(space);
        // Roost channels route on the full "{roost}/{channel}" key (the title),
        // not the base roost code that active_code() returns.
        assert_eq!(
            app.active_send_code().as_deref(),
            Some("ROOST-CODE/general")
        );
        let _base_invite = app.active_code();

        // Flocks route on their join code (the base invite).
        let flock = SpaceId::Flock(starling::protocol::FlockId::random());
        app.insert_context(ContextView {
            id: flock,
            title: "Night Birds".into(),
            roost: None,
            base_invite_display: Some("FLOCK-CODE".into()),
            messages: Vec::new(),
            unread: 0,
            state: ContextState::Ready,
            secret: Some("FLOCK-CODE".into()),
        });
        app.active = Some(flock);
        assert_eq!(app.active_send_code().as_deref(), Some("FLOCK-CODE"));
    }

    #[test]
    fn active_title_renders_roost_channels_as_roost_name_hash_channel() {
        let roost = starling::protocol::RoostId::random();
        let chan = starling::protocol::ChannelId::random();
        let space = SpaceId::RoostChannel {
            roost,
            channel: chan,
        };
        let mut app = App::default();
        app.insert_context(ContextView {
            id: space,
            title: "ROOST-CODE/general".into(),
            roost: Some(roost),
            base_invite_display: Some("ROOST-CODE".into()),
            messages: Vec::new(),
            unread: 0,
            state: ContextState::Ready,
            secret: Some("ROOST-CODE".into()),
        });
        app.roosts.push(RoostView {
            code: "ROOST-CODE".into(),
            name: "Nest".into(),
            channels: vec![FlockView {
                code: "ROOST-CODE/general".into(),
                name: "general".into(),
                messages: vec![],
                unread: 0,
            }],
            unread: 0,
        });
        app.active = Some(space);
        // The message header shows "Nest #general", not the raw routing key.
        assert_eq!(app.active_title(), "Nest #general");
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

    #[test]
    fn roost_channels_render_in_sidebar_not_server_rail() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let roost = starling::protocol::RoostId::random();
        let chan = crate::net::channel_id_from_name("general");
        let space = SpaceId::RoostChannel {
            roost,
            channel: chan,
        };
        let mut app = App::default();
        app.roosts.push(RoostView {
            code: "ROOST-CODE".into(),
            name: "Nest".into(),
            channels: vec![FlockView {
                code: "ROOST-CODE/general".into(),
                name: "general".into(),
                messages: vec![],
                unread: 0,
            }],
            unread: 0,
        });
        app.insert_context(ContextView {
            id: space,
            title: "ROOST-CODE/general".into(),
            roost: Some(roost),
            base_invite_display: Some("ROOST-CODE".into()),
            messages: Vec::new(),
            unread: 0,
            state: ContextState::Ready,
            secret: Some("ROOST-CODE".into()),
        });
        app.v2_view = V2View::Space;
        app.selection = Selection::Channel(0, 0);
        app.expanded.insert(0);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let buf = terminal.backend().buffer();
        let area = buf.area();
        let cells = buf.content();
        let full_width = area.width as usize;
        let rows: Vec<Vec<String>> = (0..area.height as usize)
            .map(|y| {
                (0..full_width)
                    .map(|x| cells[y * full_width + x].symbol().to_string())
                    .collect()
            })
            .collect();
        // v2 layout: the server rail is the leftmost 12 columns and lists
        // servers (flocks/roosts), never channels. The sidebar is the next 28
        // columns and renders the selected roost's channels.
        let rail: Vec<String> = rows
            .iter()
            .map(|row| row.iter().take(12).cloned().collect::<String>())
            .collect();
        let sidebar: Vec<String> = rows
            .iter()
            .map(|row| row.iter().skip(12).take(28).cloned().collect::<String>())
            .collect();
        assert!(
            sidebar.iter().any(|row| row.contains("general")),
            "roost channel did not render in the sidebar"
        );
        for row in &rail {
            assert!(
                !row.contains("general"),
                "roost channel leaked into the server rail: {row:?}"
            );
        }
    }
}
