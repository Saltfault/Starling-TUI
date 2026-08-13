use image::RgbImage;
use iroh::EndpointId;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};
use starling::event::{BirdStatus, ChatMessage};
use starling::protocol::{RoostId, SpaceId};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

const DEFAULT_ACCENT: Color = Color::Rgb(88, 101, 242);
const DEFAULT_AUTHOR: Color = Color::Rgb(240, 178, 50);
const DEFAULT_SELECTION: Color = Color::Rgb(242, 243, 245);
const DEFAULT_DIM: Color = Color::Rgb(148, 155, 164);
const DEFAULT_CHANNEL: Color = Color::Rgb(148, 155, 164);
const DEFAULT_INVITE: Color = Color::Rgb(35, 165, 90);

pub struct Palette {
    pub text: Color,
    pub fg_2: Color,
    pub muted: Color,
    pub background: Option<Color>,
    pub border: Color,
    pub surface: Color,
    pub surface_warm: Color,
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
            text: Color::Rgb(219, 222, 225),
            fg_2: Color::Rgb(242, 243, 245),
            muted: Color::Rgb(148, 155, 164),
            background: Some(Color::Rgb(49, 51, 56)),
            border: Color::Rgb(63, 65, 71),
            surface: Color::Rgb(43, 45, 49),
            surface_warm: Color::Rgb(30, 31, 34),
            accent: DEFAULT_ACCENT,
            author: DEFAULT_AUTHOR,
            selection: DEFAULT_SELECTION,
            dim: DEFAULT_DIM,
            channel: DEFAULT_CHANNEL,
            invite: DEFAULT_INVITE,
            hover: Color::Rgb(105, 118, 245),
            active: Color::Rgb(88, 101, 242),
            focus_ring: Color::Rgb(160, 170, 255),
        }
    }
}

impl Palette {
    pub fn success(&self) -> Color {
        Color::Rgb(35, 165, 90)
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

    #[allow(dead_code)]
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
    pub input_focus: bool,
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
    pub deafened: bool,
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
    pub notifications_muted: bool,
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
            input_focus: false,
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
            deafened: false,
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
            notifications_muted: false,
            accent_input: "#5865F2".to_string(),
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
    pub avatar_path: String,
    pub banner: String,
    pub banner_path: String,
    pub about_me: String,
    pub pronouns: String,
    pub motd: String,
    pub custom_status: String,
    pub draft_name: String,
    pub draft_avatar_label: String,
    pub draft_avatar_path: String,
    pub draft_banner: String,
    pub draft_banner_path: String,
    pub draft_about_me: String,
    pub draft_pronouns: String,
    pub draft_motd: String,
    pub draft_custom_status: String,
}

// Keep the preference in App so every renderer uses one consistent glyph policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum IconStyle {
    /// Short text labels such as `[Home]` and `[Call]`.
    Text,
    /// Nerd Font code points where available, falling back to unicode/ascii.
    NerdFont,
    /// Standard unicode symbols.
    #[default]
    Unicode,
    /// Pure ASCII fallbacks.
    Ascii,
}

impl IconStyle {
    pub fn from_env() -> Self {
        match std::env::var("STARLING_ICON_STYLE").ok().as_deref() {
            Some("text") => Self::Text,
            Some("nerd") | Some("nerdfont") => Self::NerdFont,
            Some("ascii") => Self::Ascii,
            Some("unicode") => Self::Unicode,
            _ => Self::Unicode,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::NerdFont => "Nerd Font",
            Self::Unicode => "Unicode",
            Self::Ascii => "ASCII",
        }
    }

    /// Cycle Text -> Unicode -> NerdFont -> ASCII -> Text.
    pub fn next(self) -> Self {
        match self {
            Self::Text => Self::Unicode,
            Self::Unicode => Self::NerdFont,
            Self::NerdFont => Self::Ascii,
            Self::Ascii => Self::Text,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum TerminalIcon {
    Home,
    Hash,
    Voice,
    Thread,
    Bell,
    BellSlash,
    Pin,
    Call,
    Plus,
    Gift,
    Emoji,
    Send,
    Mic,
    MicMuted,
    Headset,
    Deafened,
    Settings,
    Group,
    Server,
    Online,
    Idle,
    Dnd,
    InCall,
    Members,
    Close,
    Search,
    At,
    More,
    Video,
}

impl TerminalIcon {
    fn text(self) -> &'static str {
        match self {
            Self::Home => "[H]",
            Self::Hash => "[#]",
            Self::Voice => "[V]",
            Self::Thread => "[T]",
            Self::Bell => "[B]",
            Self::BellSlash => "[b]",
            Self::Pin => "[P]",
            Self::Call => "[C]",
            Self::Plus => "[+]",
            Self::Gift => "[G]",
            Self::Emoji => "[E]",
            Self::Send => "[S]",
            Self::Mic => "[M]",
            Self::MicMuted => "[X]",
            Self::Headset => "[A]",
            Self::Deafened => "[D]",
            Self::Settings => "[SET]",
            Self::Group => "[GRP]",
            Self::Server => "[SRV]",
            Self::Online => "[ON]",
            Self::Idle => "[IDLE]",
            Self::Dnd => "[DND]",
            Self::InCall => "[CALL]",
            Self::Members => "[MEM]",
            Self::Close => "[X]",
            Self::Search => "[?]",
            Self::At => "[@]",
            Self::More => "[...]",
            Self::Video => "[D]",
        }
    }

    fn nerd_font(self) -> Option<&'static str> {
        Some(match self {
            Self::Home => "\u{f015}",
            Self::Hash => "\u{f292}",
            Self::Voice => "\u{f130}",
            Self::Thread => "\u{f181}",
            Self::Bell => "\u{f0f3}",
            Self::BellSlash => "\u{f1f6}",
            Self::Pin => "\u{f08d}",
            Self::Call => "\u{f095}",
            Self::Plus => "\u{f067}",
            Self::Gift => "\u{f06b}",
            Self::Emoji => "\u{f118}",
            Self::Send => "\u{f1d8}",
            Self::Mic => "\u{f130}",
            Self::MicMuted => "\u{f131}",
            Self::Headset => "\u{f025}",
            Self::Deafened => "\u{f6a0}",
            Self::Settings => "\u{f013}",
            Self::Group => "\u{f0c0}",
            Self::Server => "\u{e795}",
            Self::Online => "\u{f111}",
            Self::Idle => "\u{f017}",
            Self::Dnd => "\u{f056}",
            Self::InCall => "\u{f232}",
            Self::Members => "\u{f0c0}",
            Self::Close => "\u{f00d}",
            Self::Search => "\u{f002}",
            Self::At => "\u{f1fa}",
            Self::More => "\u{f142}",
            Self::Video => "\u{f03d}",
        })
    }

    fn unicode(self) -> Option<&'static str> {
        Some(match self {
            Self::Home => "⌂",
            Self::Hash => "#",
            Self::Voice => "♫",
            Self::Thread => "@",
            Self::Bell => "🔔",
            Self::BellSlash => "🔕",
            Self::Pin => "📌",
            Self::Call => "📞",
            Self::Plus => "+",
            Self::Gift => "🎁",
            Self::Emoji => "☺",
            Self::Send => "▶",
            Self::Mic => "🎙",
            Self::MicMuted => "🔴",
            Self::Headset => "🎧",
            Self::Deafened => "🔇",
            Self::Settings => "⚙",
            Self::Group => "⚑",
            Self::Server => "◇",
            Self::Online => "●",
            Self::Idle => "◐",
            Self::Dnd => "●",
            Self::InCall => "●",
            Self::Members => "👥",
            Self::Close => "×",
            Self::Search => "🔍",
            Self::At => "@",
            Self::More => "⋮",
            Self::Video => "▣",
        })
    }

    fn ascii(self) -> &'static str {
        match self {
            Self::Home => "[H]",
            Self::Hash => "#",
            Self::Voice => "[V]",
            Self::Thread => "[T]",
            Self::Bell => "[B]",
            Self::BellSlash => "[b]",
            Self::Pin => "[P]",
            Self::Call => "[C]",
            Self::Plus => "+",
            Self::Gift => "[G]",
            Self::Emoji => "[E]",
            Self::Send => "[S]",
            Self::Mic => "[M]",
            Self::MicMuted => "[X]",
            Self::Headset => "[A]",
            Self::Deafened => "[D]",
            Self::Settings => "[SET]",
            Self::Group => "[GRP]",
            Self::Server => "[SRV]",
            Self::Online => "o",
            Self::Idle => "-",
            Self::Dnd => "x",
            Self::InCall => "o",
            Self::Members => "[MEM]",
            Self::Close => "[X]",
            Self::Search => "[?]",
            Self::At => "[@]",
            Self::More => "[...]",
            Self::Video => "[D]",
        }
    }

    pub fn glyph(self, style: IconStyle) -> &'static str {
        match style {
            IconStyle::Text => self.text(),
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
    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub fn visible_status_notice(&self, now: Instant) -> Option<&str> {
        self.status_notice.as_deref().filter(|_| {
            self.status_notice_expires_at
                .is_none_or(|expires_at| now < expires_at)
        })
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

    // Paint the whole terminal with the chat background first so no light
    // terminal default leaks through margins or border cells.
    f.render_widget(Block::default().bg(Color::Rgb(49, 51, 56)), area);

    // Reference proportions (CSS px -> terminal cols at ~8px/cell):
    // rail 72px ~9, sidebar 240px ~30, members 240px ~30.
    let columns = if matches!(app.v2_view, V2View::Home) {
        Layout::horizontal([
            Constraint::Length(9),
            Constraint::Length(30),
            Constraint::Min(1),
        ])
        .split(area)
    } else {
        Layout::horizontal([
            Constraint::Length(9),
            Constraint::Length(30),
            Constraint::Min(1),
            Constraint::Length(30),
        ])
        .split(area)
    };

    draw_server_rail(f, app, columns[0]);
    draw_sidebar(f, app, columns[1]);
    draw_chat(f, app, columns[2]);
    if matches!(app.v2_view, V2View::Space) {
        draw_members(f, app, columns[3]);
    }
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

fn draw_server_rail(f: &mut Frame, app: &App, area: Rect) {
    let rail_bg = Color::Rgb(30, 31, 34);
    let pill_bg = Color::Rgb(43, 45, 49);
    let active_bg = Color::Rgb(88, 101, 242);
    let muted = Color::Rgb(148, 155, 164);
    let fg = Color::Rgb(242, 243, 245);

    let mut items = Vec::new();
    let home_active = matches!(app.v2_view, V2View::Home);
    let home_label = TerminalIcon::Home.glyph(app.icon_style);
    let home_bg = if home_active { active_bg } else { pill_bg };
    let home_text_fg = if home_active {
        Color::Rgb(255, 255, 255)
    } else {
        fg
    };
    let home_margin = if home_active { "▎" } else { "  " };
    items.push(ListItem::new(Line::from(vec![
        Span::styled(home_margin, Style::new().fg(rail_bg).bg(rail_bg)),
        Span::styled(
            format!("{} {} ", home_margin, home_label),
            Style::new().fg(home_text_fg).bg(home_bg),
        ),
        Span::styled("", Style::new().bg(rail_bg)),
    ])));
    items.push(ListItem::new(Line::from(vec![Span::styled(
        "─────────",
        Style::new().fg(muted),
    )])));

    for (index, roost) in app.roosts.iter().enumerate() {
        let active = matches!(app.selection, Selection::Channel(ri, _) if ri == index)
            && matches!(app.v2_view, V2View::Space);
        let unread = roost.unread > 0;
        let label = if roost.name.is_empty() {
            "R".to_string()
        } else {
            roost
                .name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "R".to_string())
        };
        let bg = if active { active_bg } else { pill_bg };
        let text_fg = if active || unread {
            Color::Rgb(255, 255, 255)
        } else {
            fg
        };
        let margin = if active {
            "▎"
        } else if unread {
            "●"
        } else {
            " "
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                margin,
                Style::new()
                    .fg(if unread {
                        Color::Rgb(255, 255, 255)
                    } else {
                        rail_bg
                    })
                    .bg(rail_bg),
            ),
            Span::styled(
                format!(" {} {} ", margin, label),
                Style::new().fg(text_fg).bg(bg),
            ),
            Span::styled("", Style::new().bg(rail_bg)),
        ])));
    }
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::new().fg(muted).bg(rail_bg))
                .bg(rail_bg),
        ),
        area,
    );
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(area);

    // Header: server name (Space) or "Messages" (Home)
    let header_text = match app.v2_view {
        V2View::Home => app.name.clone(),
        V2View::Space => app
            .active_context()
            .and_then(|_| Some(app.active_title()))
            .unwrap_or_else(|| "Server".to_string()),
    };
    f.render_widget(
        Paragraph::new(header_text)
            .style(
                Style::new()
                    .fg(Color::Rgb(242, 243, 245))
                    .add_modifier(Modifier::BOLD),
            )
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(
                        Style::new()
                            .fg(Color::Rgb(63, 65, 71))
                            .bg(Color::Rgb(43, 45, 49)),
                    )
                    .bg(Color::Rgb(43, 45, 49)),
            ),
        chunks[0],
    );

    // Scroll items
    let mut items = Vec::new();
    if matches!(app.v2_view, V2View::Home) {
        // DM section header
        let plus_icon = TerminalIcon::Plus.glyph(app.icon_style);
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                "DIRECT MESSAGES",
                Style::new()
                    .fg(Color::Rgb(148, 155, 164))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}", plus_icon),
                Style::new().fg(Color::Rgb(148, 155, 164)),
            ),
        ])));
        for peer in app
            .peers
            .iter()
            .take(chunks[1].height.saturating_sub(8) as usize)
        {
            let name = app.peer_display_name(peer);
            let selected = app.selected_dm == Some(*peer);
            let status_icon = match app.peer_status.get(peer) {
                Some(BirdStatus::Online) => (TerminalIcon::Online, Color::Rgb(35, 165, 90)),
                Some(BirdStatus::Idle) => (TerminalIcon::Idle, Color::Rgb(240, 178, 50)),
                Some(BirdStatus::InCall) => (TerminalIcon::InCall, Color::Rgb(88, 101, 242)),
                None => (TerminalIcon::Dnd, Color::Rgb(148, 155, 164)),
            };
            let status_char = status_icon.0.glyph(app.icon_style);
            let status_fg = status_icon.1;
            let fg = if selected {
                Color::Rgb(255, 255, 255)
            } else {
                Color::Rgb(219, 222, 225)
            };
            let bg = if selected {
                Color::Rgb(58, 60, 66)
            } else {
                Color::Rgb(43, 45, 49)
            };
            let initials_str = initials(&name);
            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {} ", initials_str),
                    Style::new()
                        .bg(app.palette.accent)
                        .fg(Color::Rgb(255, 255, 255)),
                ),
                Span::styled(
                    format!("{} ", status_char),
                    Style::new().fg(status_fg).bg(bg),
                ),
                Span::styled(name.clone(), Style::new().fg(fg).bg(bg)),
            ])));
        }
        // Groups / flocks under a GROUPS header
        if !app.flocks.is_empty() {
            items.push(ListItem::new(Line::from(vec![Span::styled(
                "GROUPS",
                Style::new()
                    .fg(Color::Rgb(148, 155, 164))
                    .add_modifier(Modifier::BOLD),
            )])));
            for (index, flock) in app.flocks.iter().enumerate() {
                let selected = matches!(app.selection, Selection::Flock(i) if i == index);
                let fg = if selected {
                    Color::Rgb(255, 255, 255)
                } else {
                    Color::Rgb(219, 222, 225)
                };
                let bg = if selected {
                    Color::Rgb(58, 60, 66)
                } else {
                    Color::Rgb(43, 45, 49)
                };
                let group_icon = TerminalIcon::Group.glyph(app.icon_style);
                let mut spans = vec![
                    Span::styled(
                        format!("{} ", group_icon),
                        Style::new().fg(Color::Rgb(148, 155, 164)).bg(bg),
                    ),
                    Span::styled(flock.name.clone(), Style::new().fg(fg).bg(bg)),
                ];
                if flock.unread > 0 {
                    spans.push(Span::styled(
                        format!(" {} ", flock.unread),
                        Style::new()
                            .fg(Color::Rgb(255, 255, 255))
                            .bg(Color::Rgb(242, 63, 67)),
                    ));
                }
                items.push(ListItem::new(Line::from(spans)));
            }
        }
    } else {
        match app.selection {
            Selection::Flock(_) => {
                items.push(ListItem::new(Line::from(vec![Span::styled(
                    "CHANNELS",
                    Style::new()
                        .fg(Color::Rgb(148, 155, 164))
                        .add_modifier(Modifier::BOLD),
                )])));
                items.push(ListItem::new(Line::from(vec![
                    Span::styled("# ", Style::new().fg(Color::Rgb(148, 155, 164))),
                    Span::styled("general", Style::new().fg(Color::Rgb(219, 222, 225))),
                ])));
            }
            Selection::Channel(ri, ci) => {
                if let Some(roost) = app.roosts.get(ri) {
                    items.push(ListItem::new(Line::from(vec![Span::styled(
                        "TEXT CHANNELS",
                        Style::new()
                            .fg(Color::Rgb(148, 155, 164))
                            .add_modifier(Modifier::BOLD),
                    )])));
                    for (index, channel) in roost.channels.iter().enumerate() {
                        let selected = index == ci;
                        let fg = if selected {
                            Color::Rgb(255, 255, 255)
                        } else {
                            Color::Rgb(219, 222, 225)
                        };
                        let bg = if selected {
                            Color::Rgb(58, 60, 66)
                        } else {
                            Color::Rgb(43, 45, 49)
                        };
                        let hash_icon = TerminalIcon::Hash.glyph(app.icon_style);
                        let mut spans = vec![
                            Span::styled(
                                format!("{} ", hash_icon),
                                Style::new().fg(Color::Rgb(148, 155, 164)).bg(bg),
                            ),
                            Span::styled(channel.name.clone(), Style::new().fg(fg).bg(bg)),
                        ];
                        if channel.unread > 0 {
                            spans.push(Span::styled(
                                format!(" {} ", channel.unread),
                                Style::new()
                                    .fg(Color::Rgb(255, 255, 255))
                                    .bg(Color::Rgb(242, 63, 67)),
                            ));
                        }
                        items.push(ListItem::new(Line::from(spans)));
                    }
                }
            }
        }
    }

    f.render_widget(
        List::new(items).block(Block::default().bg(Color::Rgb(43, 45, 49))),
        chunks[1],
    );

    // Footer / user bar
    let initials_str = initials(&app.name);
    let user_line = Line::from(vec![
        Span::styled(
            format!(" {} ", initials_str),
            Style::new()
                .bg(app.palette.accent)
                .fg(Color::Rgb(255, 255, 255)),
        ),
        Span::styled(" ", Style::new().bg(Color::Rgb(43, 45, 49))),
        Span::styled(
            app.name.clone(),
            Style::new()
                .fg(Color::Rgb(242, 243, 245))
                .add_modifier(Modifier::BOLD)
                .bg(Color::Rgb(43, 45, 49)),
        ),
        Span::styled(
            " #0",
            Style::new()
                .fg(Color::Rgb(148, 155, 164))
                .bg(Color::Rgb(43, 45, 49)),
        ),
    ]);
    let mic_icon = if app.muted {
        TerminalIcon::MicMuted
    } else {
        TerminalIcon::Mic
    };
    let headset_icon = if app.deafened {
        TerminalIcon::Deafened.glyph(app.icon_style)
    } else {
        TerminalIcon::Headset.glyph(app.icon_style)
    };
    let settings_icon = TerminalIcon::Settings.glyph(app.icon_style);
    let controls_line = Line::from(vec![
        Span::styled(
            format!("{} ", mic_icon.glyph(app.icon_style)),
            Style::new().fg(if app.muted {
                Color::Rgb(242, 63, 67)
            } else {
                Color::Rgb(148, 155, 164)
            }),
        ),
        Span::styled(
            format!("{}  ", headset_icon),
            Style::new().fg(if app.deafened {
                Color::Rgb(242, 63, 67)
            } else {
                Color::Rgb(148, 155, 164)
            }),
        ),
        Span::styled(
            format!("{} SET", settings_icon),
            Style::new()
                .fg(Color::Rgb(148, 155, 164))
                .bg(Color::Rgb(43, 45, 49)),
        ),
    ]);
    f.render_widget(
        Paragraph::new(Text::from(vec![user_line, controls_line])).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(
                    Style::new()
                        .fg(Color::Rgb(63, 65, 71))
                        .bg(Color::Rgb(43, 45, 49)),
                )
                .bg(Color::Rgb(43, 45, 49)),
        ),
        chunks[2],
    );
}

fn flattened_channels(app: &App) -> impl Iterator<Item = &FlockView> {
    app.roosts.iter().flat_map(|roost| roost.channels.iter())
}

fn draw_chat(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(area);

    // Header
    let at_icon = TerminalIcon::At.glyph(app.icon_style);
    let title = match app.v2_view {
        V2View::Home => app
            .selected_dm
            .map(|peer| format!("{} {}", at_icon, app.peer_display_name(&peer)))
            .unwrap_or_else(|| "Messages".to_string()),
        V2View::Space => app.active_title(),
    };
    let topic = match app.v2_view {
        V2View::Home => String::new(),
        V2View::Space => "General discussion".to_string(),
    };
    let thread_icon = TerminalIcon::Thread.glyph(app.icon_style);
    let bell_icon = if app.notifications_muted {
        TerminalIcon::BellSlash
    } else {
        TerminalIcon::Bell
    }
    .glyph(app.icon_style);
    let call_icon = TerminalIcon::Call.glyph(app.icon_style);
    let more_icon = TerminalIcon::More.glyph(app.icon_style);
    let hash_icon = TerminalIcon::Hash.glyph(app.icon_style);
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            format!("{} ", hash_icon),
            Style::new()
                .fg(Color::Rgb(148, 155, 164))
                .bg(Color::Rgb(49, 51, 56)),
        ),
        Span::styled(
            title.clone(),
            Style::new()
                .fg(Color::Rgb(242, 243, 245))
                .add_modifier(Modifier::BOLD)
                .bg(Color::Rgb(49, 51, 56)),
        ),
        Span::styled("  ", Style::new().bg(Color::Rgb(49, 51, 56))),
        Span::styled(
            topic,
            Style::new()
                .fg(Color::Rgb(148, 155, 164))
                .bg(Color::Rgb(49, 51, 56)),
        ),
        Span::styled(
            format!(
                "    {} {} {}  {}",
                thread_icon, bell_icon, call_icon, more_icon
            ),
            Style::new()
                .fg(Color::Rgb(148, 155, 164))
                .bg(Color::Rgb(49, 51, 56)),
        ),
    ]))
    .alignment(Alignment::Left)
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(
                Style::new()
                    .fg(Color::Rgb(63, 65, 71))
                    .bg(Color::Rgb(49, 51, 56)),
            )
            .bg(Color::Rgb(49, 51, 56)),
    );
    f.render_widget(header, chunks[0]);

    // Message list
    let rows: Vec<ListItem> = match app.v2_view {
        V2View::Home => vec![ListItem::new(Line::from(vec![Span::styled(
            if app.selected_dm.is_some() {
                "No direct messages yet."
            } else {
                "Select a peer to begin a direct message."
            },
            Style::new()
                .fg(Color::Rgb(148, 155, 164))
                .bg(Color::Rgb(49, 51, 56)),
        )]))],
        V2View::Space => {
            let msgs = app.active_messages();
            if msgs.is_empty() {
                vec![
                    ListItem::new(Line::from(vec![Span::styled(
                        format!(
                            "Welcome to {}{}!",
                            TerminalIcon::Hash.glyph(app.icon_style),
                            title
                        ),
                        Style::new()
                            .fg(Color::Rgb(242, 243, 245))
                            .add_modifier(Modifier::BOLD)
                            .bg(Color::Rgb(49, 51, 56)),
                    )])),
                    ListItem::new(Line::from(vec![Span::styled(
                        "This is the start of the channel.",
                        Style::new()
                            .fg(Color::Rgb(148, 155, 164))
                            .bg(Color::Rgb(49, 51, 56)),
                    )])),
                ]
            } else {
                let mut rows = vec![ListItem::new(Line::from(vec![Span::styled(
                    "────── Today ──────",
                    Style::new()
                        .fg(Color::Rgb(63, 65, 71))
                        .bg(Color::Rgb(49, 51, 56)),
                )]))];
                for message in msgs {
                    let color = author_color(&message.msg.author);
                    let header = Line::from(vec![
                        Span::styled(
                            format!(" {} ", initials(&message.msg.author)),
                            Style::new().bg(color).fg(Color::Rgb(255, 255, 255)),
                        ),
                        Span::styled(
                            format!(" {}", message.msg.author),
                            Style::new().fg(color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("  {}", format_ts(message.msg.ts)),
                            Style::new().fg(Color::Rgb(148, 155, 164)),
                        ),
                    ]);
                    let mut body_spans = Vec::new();
                    if message.private {
                        let lock_icon = TerminalIcon::Close.glyph(app.icon_style);
                        body_spans.push(Span::styled(
                            format!("{} ", lock_icon),
                            Style::new().fg(Color::Rgb(148, 155, 164)),
                        ));
                    }
                    body_spans.push(Span::styled(
                        message.msg.body.clone(),
                        Style::new().fg(Color::Rgb(219, 222, 225)),
                    ));
                    rows.push(ListItem::new(Text::from(vec![
                        header,
                        Line::from(body_spans),
                    ])));
                }
                rows
            }
        }
    };
    f.render_widget(
        List::new(rows).block(Block::default().bg(Color::Rgb(49, 51, 56))),
        chunks[1],
    );

    // Input
    draw_message_bar(f, app, chunks[2]);
}

fn draw_members(f: &mut Frame, app: &App, area: Rect) {
    let mut items = Vec::new();
    let members_icon = TerminalIcon::Members.glyph(app.icon_style);
    items.push(ListItem::new(Line::from(vec![Span::styled(
        format!("{} MEMBERS", members_icon),
        Style::new()
            .fg(Color::Rgb(148, 155, 164))
            .add_modifier(Modifier::BOLD)
            .bg(Color::Rgb(43, 45, 49)),
    )])));
    let initials_str = initials(&app.name);
    items.push(ListItem::new(Line::from(vec![
        Span::styled(
            format!(" {} ", initials_str),
            Style::new()
                .bg(app.palette.accent)
                .fg(Color::Rgb(255, 255, 255)),
        ),
        Span::styled(" ", Style::new().bg(Color::Rgb(43, 45, 49))),
        Span::styled(
            format!("{} (you)", app.name),
            Style::new()
                .fg(Color::Rgb(219, 222, 225))
                .bg(Color::Rgb(43, 45, 49)),
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
    let sections = [
        ("Online", online.as_slice()),
        ("Idle", idle.as_slice()),
        ("In Call", in_call.as_slice()),
    ];

    for (label, members) in sections {
        let header = if label.is_empty() {
            format!("Members — {}", members.len())
        } else {
            format!("{} — {}", label, members.len())
        };
        items.push(ListItem::new(Line::from(vec![Span::styled(
            header,
            Style::new()
                .fg(Color::Rgb(148, 155, 164))
                .add_modifier(Modifier::BOLD)
                .bg(Color::Rgb(43, 45, 49)),
        )])));
        for peer in members {
            let name = app.peer_display_name(peer);
            let (status_icon, status_fg) = match app.peer_status.get(peer) {
                Some(BirdStatus::Online) => (TerminalIcon::Online, Color::Rgb(35, 165, 90)),
                Some(BirdStatus::Idle) => (TerminalIcon::Idle, Color::Rgb(240, 178, 50)),
                Some(BirdStatus::InCall) => (TerminalIcon::InCall, Color::Rgb(88, 101, 242)),
                None => (TerminalIcon::Dnd, Color::Rgb(148, 155, 164)),
            };
            let status_char = status_icon.glyph(app.icon_style);
            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {} ", initials(&name)),
                    Style::new()
                        .bg(app.palette.accent)
                        .fg(Color::Rgb(255, 255, 255)),
                ),
                Span::styled(" ", Style::new().bg(Color::Rgb(43, 45, 49))),
                Span::styled(
                    format!("{} {}", status_char, name),
                    Style::new().fg(status_fg).bg(Color::Rgb(43, 45, 49)),
                ),
            ])));
        }
    }
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(
                    Style::new()
                        .fg(Color::Rgb(63, 65, 71))
                        .bg(Color::Rgb(43, 45, 49)),
                )
                .bg(Color::Rgb(43, 45, 49)),
        ),
        area,
    );
}

fn draw_message_bar(f: &mut Frame, app: &App, area: Rect) {
    let input_bg = Color::Rgb(56, 58, 64);
    let border_color = if app.input_focus {
        app.palette.accent
    } else {
        Color::Rgb(63, 65, 71)
    };
    f.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::new().fg(border_color).bg(Color::Rgb(49, 51, 56)))
            .bg(Color::Rgb(49, 51, 56)),
        area,
    );
    let outer = area.inner(Margin {
        horizontal: 1,
        vertical: 0,
    });

    let placeholder = match app.v2_view {
        V2View::Home => {
            if let Some(peer) = app.selected_dm {
                format!(
                    "Message {}{}",
                    TerminalIcon::At.glyph(app.icon_style),
                    app.peer_display_name(&peer)
                )
            } else {
                "Message".to_string()
            }
        }
        V2View::Space => format!(
            "Message {}{}",
            TerminalIcon::Hash.glyph(app.icon_style),
            app.active_title()
        ),
    };
    let plus_icon = TerminalIcon::Plus.glyph(app.icon_style);
    let gift_icon = TerminalIcon::Gift.glyph(app.icon_style);
    let emoji_icon = TerminalIcon::Emoji.glyph(app.icon_style);
    let send_icon = TerminalIcon::Send.glyph(app.icon_style);

    let body = if app.input.is_empty() {
        format!("{} {}  {}", plus_icon, gift_icon, placeholder)
    } else {
        format!("{} {}  {}", plus_icon, gift_icon, app.input)
    };
    let input_color = if app.input.is_empty() {
        Color::Rgb(148, 155, 164)
    } else {
        Color::Rgb(219, 222, 225)
    };
    let content = format!("{} {} {}", body, emoji_icon, send_icon);

    f.render_widget(
        Paragraph::new(content)
            .style(Style::new().fg(input_color).bg(input_bg))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(border_color))
                    .bg(input_bg),
            ),
        outer,
    );
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
    let area = centered(f.area(), 80, 20);
    f.render_widget(Clear, area);
    let modal_bg = Color::Rgb(43, 45, 49);
    let nav_bg = Color::Rgb(37, 39, 42);
    let border_fg = Color::Rgb(63, 65, 71);
    let text = Color::Rgb(219, 222, 225);
    let muted = Color::Rgb(148, 155, 164);
    let fg_2 = Color::Rgb(242, 243, 245);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(" SETTINGS ")
            .border_style(Style::new().fg(border_fg).bg(modal_bg))
            .bg(modal_bg),
        area,
    );
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    let columns = Layout::horizontal([Constraint::Length(22), Constraint::Min(1)]).split(inner);

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
        let bg = if selected {
            Color::Rgb(58, 60, 66)
        } else {
            nav_bg
        };
        let style = if selected {
            Style::new().fg(fg_2).add_modifier(Modifier::BOLD).bg(bg)
        } else {
            Style::new().fg(muted).bg(bg)
        };
        ListItem::new(Span::styled(label.to_string(), style)).bg(bg)
    })
    .collect();
    f.render_widget(
        List::new(tabs).block(
            Block::default()
                .borders(Borders::RIGHT)
                .border_style(Style::new().fg(border_fg).bg(nav_bg))
                .bg(nav_bg),
        ),
        columns[0],
    );

    let content = Paragraph::new(match app.settings_tab {
        SettingsTab::Account => format!(
            "Display name\n  {}\n\nEmail\n  you@starling.local\n\nAvatar label\n  {}\n\n[ESC] close",
            app.name,
            app.profile_panel.avatar_label
        ),
        SettingsTab::Voice => {
            "Input device\n  Default\n\nOutput device\n  Default\n\nPush to Talk\n  Off\n\nNoise suppression\n  On".to_string()
        }
        SettingsTab::Appearance => format!(
            "Theme\n  Dark\n\nAccent color\n  {}\n\nIcon style\n  {}\n\nCompact mode\n  Off\n\nShow avatars\n  On\n\n[ENTER] apply accent  [TAB] cycle icons  [ESC] close",
            app.accent_input, app.icon_style.label()
        ),
        SettingsTab::Notifications => {
            "Desktop notifications\n  On\n\nMute @everyone\n  Off\n\nSounds\n  On".to_string()
        }
        SettingsTab::Keybinds => {
            "Mark server read\n  Shift + Esc\n\nToggle mute\n  Ctrl + Shift + M\n\nToggle deafen\n  Ctrl + Shift + D\n\nAnswer call\n  Ctrl + Enter".to_string()
        }
    })
    .style(Style::new().fg(text).bg(modal_bg))
    .wrap(Wrap { trim: true });
    f.render_widget(content, columns[1]);
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
    let term = f.area();
    let width = 50u16.min(term.width.saturating_sub(4)).max(30);
    let height = 18u16.min(term.height.saturating_sub(4)).max(10);
    let area = Rect {
        x: 2,
        y: term.height.saturating_sub(height + 2),
        width,
        height,
    };
    f.render_widget(Clear, area);
    let modal_bg = Color::Rgb(43, 45, 49);
    let border_fg = Color::Rgb(63, 65, 71);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(" PROFILE ")
            .border_style(Style::new().fg(border_fg).bg(modal_bg))
            .bg(modal_bg),
        area,
    );
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let p = &app.profile_panel;
    let banner_line = if p.banner.is_empty() {
        if p.banner_path.is_empty() {
            "──────── banner ────────".to_string()
        } else {
            format!("banner: {}", p.banner_path)
        }
    } else {
        p.banner.clone()
    };
    let avatar_text = if p.avatar_label.is_empty() {
        initials(&p.draft_name)
    } else {
        p.avatar_label.clone()
    };
    let avatar_path_line = if p.editing {
        format!("Avatar path: {}", p.draft_avatar_path)
    } else if !p.avatar_path.is_empty() {
        format!("Avatar path: {}", p.avatar_path)
    } else {
        "Avatar path: (none)".to_string()
    };
    let banner_path_line = if p.editing {
        format!("Banner path: {}", p.draft_banner_path)
    } else if !p.banner_path.is_empty() {
        format!("Banner path: {}", p.banner_path)
    } else {
        "Banner path: (none)".to_string()
    };
    let name_text = if p.editing {
        p.draft_name.clone()
    } else {
        app.name.clone()
    };
    let status_text = if p.editing {
        p.draft_custom_status.clone()
    } else {
        p.custom_status.clone()
    };
    let pronouns_text = if p.editing {
        p.draft_pronouns.clone()
    } else {
        p.pronouns.clone()
    };
    let about_text = if p.editing {
        p.draft_about_me.clone()
    } else {
        p.about_me.clone()
    };
    let motd_text = if p.editing {
        p.draft_motd.clone()
    } else {
        p.motd.clone()
    };
    let accent = app.palette.accent;

    let mut lines = vec![
        Line::from(vec![Span::styled(
            banner_line.clone(),
            Style::new().fg(accent).bg(Color::Rgb(78, 80, 88)),
        )]),
        Line::from(vec![
            Span::styled(
                format!(" {} ", avatar_text),
                Style::new().bg(accent).fg(Color::Rgb(255, 255, 255)),
            ),
            Span::styled("  ", Style::new().bg(modal_bg)),
            Span::styled(
                name_text,
                Style::new()
                    .fg(Color::Rgb(242, 243, 245))
                    .add_modifier(Modifier::BOLD)
                    .bg(modal_bg),
            ),
        ]),
    ];
    if !status_text.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            format!(
                " {} {}",
                TerminalIcon::Online.glyph(app.icon_style),
                status_text
            ),
            Style::new().fg(Color::Rgb(148, 155, 164)).bg(modal_bg),
        )]));
    }
    lines.push(Line::from(vec![Span::styled(
        avatar_path_line,
        Style::new().fg(Color::Rgb(148, 155, 164)).bg(modal_bg),
    )]));
    lines.push(Line::from(vec![Span::styled(
        banner_path_line,
        Style::new().fg(Color::Rgb(148, 155, 164)).bg(modal_bg),
    )]));
    if !pronouns_text.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(
                "Pronouns: ",
                Style::new().fg(Color::Rgb(148, 155, 164)).bg(modal_bg),
            ),
            Span::styled(
                pronouns_text,
                Style::new().fg(Color::Rgb(219, 222, 225)).bg(modal_bg),
            ),
        ]));
    }
    if !about_text.is_empty() {
        lines.push(Line::from(vec![]));
        lines.push(Line::from(vec![Span::styled(
            "About Me",
            Style::new()
                .fg(Color::Rgb(242, 243, 245))
                .add_modifier(Modifier::BOLD)
                .bg(modal_bg),
        )]));
        lines.push(Line::from(vec![Span::styled(
            about_text,
            Style::new().fg(Color::Rgb(219, 222, 225)).bg(modal_bg),
        )]));
    }
    if !motd_text.is_empty() {
        lines.push(Line::from(vec![]));
        lines.push(Line::from(vec![Span::styled(
            "MOTD",
            Style::new()
                .fg(Color::Rgb(242, 243, 245))
                .add_modifier(Modifier::BOLD)
                .bg(modal_bg),
        )]));
        lines.push(Line::from(vec![Span::styled(
            motd_text,
            Style::new().fg(Color::Rgb(219, 222, 225)).bg(modal_bg),
        )]));
    }

    lines.push(Line::from(vec![]));
    let footer = if p.editing {
        format!(
            "Field: {:?} | [TAB] next | [ENTER] save | [ESC] cancel",
            p.field
        )
    } else {
        "[E] edit | [ESC] close".to_string()
    };
    lines.push(Line::from(vec![Span::styled(
        footer,
        Style::new().fg(Color::Rgb(148, 155, 164)).bg(modal_bg),
    )]));

    f.render_widget(Paragraph::new(Text::from(lines)), inner);
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
        // v2 layout: the server rail is the leftmost 9 columns and lists
        // servers (flocks/roosts), never channels. The sidebar is the next 30
        // columns and renders the selected roost's channels.
        let rail: Vec<String> = rows
            .iter()
            .map(|row| row.iter().take(9).cloned().collect::<String>())
            .collect();
        let sidebar: Vec<String> = rows
            .iter()
            .map(|row| row.iter().skip(9).take(30).cloned().collect::<String>())
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
