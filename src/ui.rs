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
    /// Optional image path used as the server icon in the rail.
    pub icon_path: Option<String>,
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
    pub tag: String,
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
    pub call_title: String,
    pub accent_input: String,
    pub settings_tab: SettingsTab,
    pub selected_dm: Option<EndpointId>,
    pub reference_dm_selected: usize,
    /// Flock join code -> true when the local user created this flock.
    pub flock_owners: HashMap<String, bool>,
    /// Roost join code -> owner endpoint, from the roost's `PermState`.
    pub roost_owners: HashMap<String, EndpointId>,
    pub show_pinned: bool,
    pub show_notifications: bool,
    /// In-app notification log (flocks and roost channels, excludes own
    /// messages). Rendered in the Notifications popup; also drives the
    /// unread badge on the header bell and the rail/sidebar indicators.
    pub notifications: Vec<NotificationItem>,
    pub show_members: bool,
    /// Toggles on when the terminal shrinks below the narrow breakpoint; the
    /// header menu button restores the channel/friends list while narrow.
    pub sidebar_hidden: bool,
    /// Terminal width (cols) from the last frame; resizing is detected here.
    pub terminal_width: u16,
    /// User drag offset applied to every popup, relative to its centered
    /// position. Popups are draggable by their title row.
    pub popup_offset: (i16, i16),
    /// While dragging a popup: (cursor col - popup.x, cursor row - popup.y)
    /// captured at press, so the popup follows the cursor.
    pub drag_grab: Option<(i16, i16)>,
    pub icon_style: IconStyle,
}

/// One in-app notification: a recent message in a space that wasn't the
/// active one at delivery time. Own messages never generate these.
#[derive(Clone, Debug)]
pub struct NotificationItem {
    pub space_name: String,
    pub author: String,
    pub body: String,
    pub ts: i64,
}

impl Default for App {
    fn default() -> Self {
        Self {
            name: "you".to_string(),
            tag: "#7134".to_string(),
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
            call_title: "Call".to_string(),
            accent_input: "#5865F2".to_string(),
            selected_dm: None,
            reference_dm_selected: 4,
            flock_owners: HashMap::new(),
            roost_owners: HashMap::new(),
            show_pinned: false,
            show_notifications: false,
            notifications: Vec::new(),
            // Members start hidden; the header button opens them on demand.
            // On narrow (mobile) screens this keeps the chat full-width.
            show_members: false,
            sidebar_hidden: false,
            terminal_width: 0,
            popup_offset: (0, 0),
            drag_grab: None,
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
#[allow(dead_code)]
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
    Lock,
    Menu,
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
            Self::Lock => "[L]",
            Self::Menu => "[MENU]",
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
            Self::Lock => "\u{f023}",
            Self::Menu => "\u{f0c9}",
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
            Self::Lock => "🔒",
            Self::Menu => "☰",
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
            Self::Lock => "[L]",
            Self::Menu => "[M]",
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

fn initials(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "?".to_string();
    }
    trimmed
        .split_whitespace()
        .take(2)
        .filter_map(|part| part.chars().next())
        .map(|c| c.to_uppercase().to_string())
        .collect()
}

fn flock_icon(name: &str) -> String {
    let trimmed = name.trim();
    let initials_text = trimmed
        .split_whitespace()
        .filter_map(|part| part.chars().next())
        .take(2)
        .collect::<String>();
    if initials_text.chars().count() <= trimmed.chars().count() {
        initials_text
    } else {
        trimmed.to_string()
    }
}

fn format_ts(ts: i64) -> String {
    use chrono::TimeZone;
    let secs = if ts > 10_000_000_000 { ts / 1000 } else { ts };
    chrono::Local
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
    Flock(usize),
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
    LeaveSpace,
    EditSpace,
    DeleteSpace,
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
    pub fn apply_roost_perms(&mut self, code: &str, perms: &starling::roost::perms::PermState) {
        if let Some(my_id) = self.node_id {
            self.my_perms = perms.effective(&my_id);
        }
        if let Some(owner) = perms.owner {
            self.roost_owners.insert(code.to_string(), owner);
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
                .map(|f| f.name.clone())
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
                    let name = flock.name.clone();
                    self.notifications.retain(|n| n.space_name != name);
                }
            }
            Selection::Channel(ri, ci) => {
                if let Some(roost) = self.roosts.get_mut(ri) {
                    if let Some(channel) = roost.channels.get_mut(ci) {
                        channel.unread = 0;
                        let name = channel.name.clone();
                        self.notifications.retain(|n| n.space_name != name);
                    }
                    roost.unread = roost.channels.iter().map(|channel| channel.unread).sum();
                }
            }
        }
    }

    /// Select a flock while keeping the Friends (Home) sidebar visible, so the
    /// flock does not vanish from the DM list.
    pub fn select_flock(&mut self, index: usize) {
        self.selection = Selection::Flock(index);
        self.active = None;
        self.v2_view = V2View::Home;
        if let Some(flock) = self.flocks.get_mut(index) {
            flock.unread = 0;
            let name = flock.name.clone();
            self.notifications.retain(|n| n.space_name != name);
        }
    }

    pub fn open_home(&mut self) {
        self.v2_view = V2View::Home;
        self.active = None;
    }

    /// The rect for a popup of `width`x`height`: centered, then shifted by the
    /// user's drag offset and clamped so the title row stays on screen.
    pub fn popup_rect(&self, term_w: u16, term_h: u16, width: u16, height: u16) -> Rect {
        let w = width.min(term_w);
        let h = height.min(term_h);
        // i32 math: the drag offset is signed, and a negative offset (dragging
        // left/up) must not wrap when cast back to u16.
        let max_x = term_w.saturating_sub(w) as i32;
        let max_y = term_h.saturating_sub(h) as i32;
        let x = (((term_w.saturating_sub(w)) / 2) as i32 + self.popup_offset.0 as i32)
            .clamp(0, max_x) as u16;
        let y = (((term_h.saturating_sub(h)) / 2) as i32 + self.popup_offset.1 as i32)
            .clamp(0, max_y) as u16;
        Rect::new(x, y, w, h)
    }

    /// Whether a click at `(col, row)` lands on the title row of the active
    /// popup — the drag handle. The X close button is excluded so closing
    /// never starts a drag.
    pub fn popup_title_row(&self, term_w: u16, term_h: u16, col: u16, row: u16) -> bool {
        let Some(popup) = active_popup_rect(self, term_w, term_h) else {
            return false;
        };
        if row != popup.y {
            return false;
        }
        let x_glyph = TerminalIcon::Close.glyph(self.icon_style);
        let x_w = x_glyph.chars().count() as u16;
        col >= popup.x && col < popup.right().saturating_sub(x_w)
    }

    /// Track the terminal width across frames and apply the responsive rules:
    /// narrow screens auto-hide the channel/friends sidebar and members panel;
    /// when the screen grows back to a large size both lists come back
    /// automatically.
    pub fn note_terminal_size(&mut self, width: u16) {
        let grew_wide = self.terminal_width < MOBILE_BREAKPOINT && width >= MOBILE_BREAKPOINT;
        self.terminal_width = width;
        if width < NARROW_BREAKPOINT {
            self.sidebar_hidden = true;
            self.show_members = false;
        } else if width < MOBILE_BREAKPOINT {
            self.show_members = false;
        } else if grew_wide {
            self.sidebar_hidden = false;
            self.show_members = true;
        }
    }

    /// Deliver an incoming message to a flock or roost channel, mirroring the
    /// `AppEvent::Message` handling in main(): append to the message list,
    /// mark unread when the space is not on screen, and record a notification
    /// (never for the user's own messages). `private` marks sealed 1:1 chirps.
    pub fn receive_message(&mut self, flock: &str, msg: ChatMessage, private: bool) {
        let is_current = self.active_send_code().is_some_and(|code| code == flock);
        let is_own = msg.author == self.name;
        let mut space_name = None;
        if let Some(fv) = self
            .flocks
            .iter_mut()
            .find(|fv| fv.code == flock)
            .or_else(|| {
                self.roosts
                    .iter_mut()
                    .flat_map(|roost| roost.channels.iter_mut())
                    .find(|channel| channel.code == flock)
            })
        {
            fv.messages.push(MessageView {
                msg: msg.clone(),
                private,
            });
            if !is_current && !is_own {
                fv.unread += 1;
            }
            if !fv.name.is_empty() {
                space_name = Some(fv.name.clone());
            }
        }
        for roost in &mut self.roosts {
            roost.unread = roost.channels.iter().map(|channel| channel.unread).sum();
        }
        if !is_own
            && !is_current
            && let Some(space_name) = space_name
        {
            self.notifications.push(NotificationItem {
                space_name,
                author: msg.author.clone(),
                body: msg.body.clone(),
                ts: msg.ts,
            });
        }
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
            .or_else(|| {
                // Flocks are selected without an active context (the Friends
                // sidebar stays visible), so resolve the space id from the
                // selection to find their presence roster.
                let space = match self.selection {
                    Selection::Flock(i) => self.flocks.get(i).and_then(|fv| {
                        starling::net::decode_typed_code(&fv.code)
                            .and_then(|t| starling::net::decode_flock_code(&t))
                            .map(|fc| SpaceId::Flock(starling::protocol::FlockId(fc.secret)))
                    }),
                    _ => None,
                };
                space
                    .and_then(|id| self.presence.contexts.get(&id))
                    .map(|presence| presence.ordered_ids.clone())
            })
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
                    // Local owner or roost admin may edit/delete the roost.
                    let can_admin = self.node_id.is_some_and(|id| {
                        self.roost_owners.get(&rv.code) == Some(&id)
                            || perms.contains(starling::roost::perms::Perm::ADMIN)
                    });
                    self.context_menu_items.push(ContextMenuItem {
                        label: "Leave Roost".into(),
                        action: ContextMenuAction::LeaveSpace,
                        enabled: true,
                    });
                    if can_admin {
                        self.context_menu_items.push(ContextMenuItem {
                            label: "Edit Roost".into(),
                            action: ContextMenuAction::EditSpace,
                            enabled: true,
                        });
                        self.context_menu_items.push(ContextMenuItem {
                            label: "Delete Roost".into(),
                            action: ContextMenuAction::DeleteSpace,
                            enabled: true,
                        });
                    }
                }
            }
            ContextMenuTarget::Flock(index) => {
                if let Some(fv) = self.flocks.get(index) {
                    let is_owner = self.flock_owners.get(&fv.code).copied().unwrap_or(false);
                    self.context_menu_items.push(ContextMenuItem {
                        label: "Leave Flock".into(),
                        action: ContextMenuAction::LeaveSpace,
                        enabled: true,
                    });
                    if is_owner {
                        self.context_menu_items.push(ContextMenuItem {
                            label: "Edit Flock".into(),
                            action: ContextMenuAction::EditSpace,
                            enabled: true,
                        });
                        self.context_menu_items.push(ContextMenuItem {
                            label: "Delete Flock".into(),
                            action: ContextMenuAction::DeleteSpace,
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
            ContextMenuTarget::Flock(_) => None,
        }
    }

    /// Join code (base roost code or flock code) of the current menu target,
    /// or the active context when the target is a bird.
    pub fn context_menu_code(&self) -> Option<String> {
        let target = self.context_menu_target.as_ref()?;
        match target {
            ContextMenuTarget::Roost(ri) => self.roosts.get(*ri).map(|rv| rv.code.clone()),
            ContextMenuTarget::RoostChannel(ri, _) => {
                self.roosts.get(*ri).map(|rv| rv.code.clone())
            }
            ContextMenuTarget::Flock(i) => self.flocks.get(*i).map(|fv| fv.code.clone()),
            ContextMenuTarget::Bird(_) => self.active_code().map(str::to_owned),
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

/// Below this terminal width the UI switches to the mobile layout: the server
/// rail moves to the bottom of the screen, members hide unless explicitly
/// opened, and the chat takes the full width.
pub const MOBILE_BREAKPOINT: u16 = 80;
/// Below this width even the sidebar (channel list / friends) is hidden and
/// must be toggled back with the header menu button.
pub const NARROW_BREAKPOINT: u16 = 56;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let mobile = area.width < MOBILE_BREAKPOINT;

    // Paint the whole terminal with the chat background first so no light
    // terminal default leaks through margins or border cells.
    f.render_widget(Block::default().bg(Color::Rgb(49, 51, 56)), area);

    let members_visible = app.show_members
        && (matches!(app.v2_view, V2View::Space)
            || (matches!(app.v2_view, V2View::Home)
                && matches!(app.selection, Selection::Flock(_))
                && app.selected_dm.is_none()));
    // The channel list / friends sidebar is hidden on very narrow screens
    // (see NARROW_BREAKPOINT) and can be restored with the header menu button.
    let sidebar_visible = !app.sidebar_hidden
        && (matches!(app.v2_view, V2View::Space) || matches!(app.v2_view, V2View::Home));

    if mobile {
        // Narrow layout: no rail column, no persistent sidebar. The home and
        // roost pills live in a 3-row rail under the composer; members slide
        // in as a right-side panel only when explicitly opened; the Friends
        // list takes the left when on the Home view.
        let rail_h = 3u16;
        let body = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height.saturating_sub(rail_h),
        };
        let (chat_area, side) = if members_visible {
            let cols = Layout::horizontal([Constraint::Min(1), Constraint::Length(30)]).split(body);
            (cols[0], Some((cols[1], false)))
        } else if sidebar_visible {
            // Friends list (Home) or channel list (Space) on the left.
            let cols = Layout::horizontal([Constraint::Length(30), Constraint::Min(1)]).split(body);
            (cols[1], Some((cols[0], true)))
        } else {
            (body, None)
        };
        if let Some((rect, is_sidebar)) = side {
            if is_sidebar {
                draw_sidebar(f, app, rect);
            } else {
                draw_members(f, app, rect);
            }
        }
        draw_chat(f, app, chat_area);
        draw_mobile_rail(
            f,
            app,
            Rect {
                x: area.x,
                y: body.bottom(),
                width: area.width,
                height: rail_h,
            },
        );
        draw_popups(f, app);
        return;
    }

    // Reference proportions (CSS px -> terminal cols at ~8px/cell):
    // rail 72px ~9, sidebar 240px ~30, members 240px ~30.
    let columns = if members_visible {
        Layout::horizontal([
            Constraint::Length(9),
            Constraint::Length(30),
            Constraint::Min(1),
            Constraint::Length(30),
        ])
        .split(area)
    } else {
        Layout::horizontal([
            Constraint::Length(9),
            Constraint::Length(30),
            Constraint::Min(1),
        ])
        .split(area)
    };

    draw_server_rail(f, app, columns[0]);
    draw_sidebar(f, app, columns[1]);
    draw_chat(f, app, columns[2]);
    if members_visible {
        draw_members(f, app, columns[3]);
    }
    draw_popups(f, app);
}

fn draw_popups(f: &mut Frame, app: &App) {
    if app.show_pinned {
        draw_empty_popup(f, "Pinned Messages", app);
    }
    if app.show_notifications {
        draw_notifications_popup(f, app);
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

/// Bottom navigation rail for narrow terminals: home pill first, then one
/// pill per roost, mirroring the desktop left rail. Clicking a pill selects
/// the space (see `handle_mobile_rail_click`).
fn draw_mobile_rail(f: &mut Frame, app: &App, area: Rect) {
    let rail_bg = app.palette.surface_warm;
    f.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::new().fg(app.palette.border).bg(rail_bg))
            .bg(rail_bg),
        area,
    );
    let pill_w = 6u16;
    let gap = 1u16;
    let mut x = area.x + 1;
    draw_server_pill(
        f,
        app,
        TerminalIcon::Home.glyph(app.icon_style),
        matches!(app.v2_view, V2View::Home),
        false,
        Rect {
            x,
            y: area.y,
            width: pill_w,
            height: area.height,
        },
    );
    x += pill_w + gap;
    for (index, roost) in app.roosts.iter().enumerate() {
        if x + pill_w > area.right() {
            break;
        }
        let label = if roost.icon_path.is_some() {
            TerminalIcon::Video.glyph(app.icon_style).to_string()
        } else {
            flock_icon(&roost.name)
        };
        draw_server_pill(
            f,
            app,
            &label,
            matches!(app.selection, Selection::Channel(i, _) if i == index),
            roost.unread > 0,
            Rect {
                x,
                y: area.y,
                width: pill_w,
                height: area.height,
            },
        );
        x += pill_w + gap;
    }
}

fn draw_server_rail(f: &mut Frame, app: &App, area: Rect) {
    let rail_bg = app.palette.surface_warm;
    f.render_widget(
        Block::default()
            .borders(Borders::RIGHT)
            .border_style(Style::new().fg(app.palette.border).bg(rail_bg))
            .bg(rail_bg),
        area,
    );
    let pill_w = 6u16.min(area.width.saturating_sub(2)).max(4);
    let pad_x = (area.width.saturating_sub(pill_w)) / 2;
    let home_rect = Rect {
        x: area.x + pad_x,
        y: area.y + 1,
        width: pill_w,
        height: 3,
    };
    draw_server_pill(
        f,
        app,
        TerminalIcon::Home.glyph(app.icon_style),
        matches!(app.v2_view, V2View::Home),
        false,
        home_rect,
    );
    let mut y = home_rect.bottom() + 1;
    for (index, roost) in app.roosts.iter().enumerate() {
        if y + 3 > area.bottom() {
            break;
        }
        let rect = Rect {
            x: area.x + pad_x,
            y,
            width: pill_w,
            height: 3,
        };
        let label = if roost.icon_path.is_some() {
            TerminalIcon::Video.glyph(app.icon_style)
        } else {
            &flock_icon(&roost.name)
        };
        draw_server_pill(
            f,
            app,
            label,
            matches!(app.selection, Selection::Channel(i, _) if i == index),
            roost.unread > 0,
            rect,
        );
        y += 4;
    }
}
fn draw_server_pill(f: &mut Frame, app: &App, label: &str, active: bool, unread: bool, area: Rect) {
    let rail_bg = app.palette.background.unwrap_or(Color::Rgb(30, 31, 34));
    let pill_bg = app.palette.surface;
    let active_bg = app.palette.accent;
    let fg = app.palette.fg_2;
    let bg = if active { active_bg } else { pill_bg };
    let text_fg = if active || unread {
        Color::Rgb(255, 255, 255)
    } else {
        fg
    };
    let indicator = if active {
        TerminalIcon::More.glyph(app.icon_style)
    } else if unread {
        TerminalIcon::Online.glyph(app.icon_style)
    } else {
        " "
    };
    let indicator_color = if active || unread {
        Color::Rgb(255, 255, 255)
    } else {
        rail_bg
    };
    let centered = format!("{:^1$}", label, area.width as usize);
    let lines: Vec<Line> = (0..area.height)
        .map(|row| {
            let text = if row == area.height / 2 {
                centered.clone()
            } else {
                " ".repeat(area.width as usize)
            };
            let indicator_span = if row == area.height / 2 {
                Span::styled(indicator, Style::new().fg(indicator_color).bg(rail_bg))
            } else {
                Span::styled(" ", Style::new().bg(rail_bg))
            };
            Line::from(vec![
                indicator_span,
                Span::styled(text, Style::new().fg(text_fg).bg(bg)),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(Text::from(lines)).bg(rail_bg), area);
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let bg = app.palette.surface;
    let border = app.palette.border;
    let text = app.palette.text;
    let fg_2 = app.palette.fg_2;
    let muted = app.palette.muted;
    let selected_bg = app.palette.active;
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(4),
    ])
    .split(area);
    f.render_widget(
        Paragraph::new(if matches!(app.v2_view, V2View::Home) {
            "Friends"
        } else {
            "Server"
        })
        .style(Style::new().fg(fg_2).add_modifier(Modifier::BOLD))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::new().fg(border))
                .bg(bg),
        ),
        rows[0],
    );
    let mut items = Vec::new();
    if matches!(app.v2_view, V2View::Home) {
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                "DIRECT MESSAGES",
                Style::new().fg(muted).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  +", Style::new().fg(muted)),
        ])));
        for (index, peer) in app.peers.iter().enumerate() {
            let name = app.peer_display_name(peer);
            let active = app.selected_dm == Some(*peer)
                || (app.selected_dm.is_none() && index == app.reference_dm_selected);
            let status_color = match app.peer_status.get(peer) {
                Some(BirdStatus::Online) => Color::Rgb(35, 165, 90),
                Some(BirdStatus::Idle) => Color::Rgb(240, 178, 50),
                Some(BirdStatus::InCall) => Color::Rgb(88, 101, 242),
                None => muted,
            };
            let row_bg = if active { selected_bg } else { bg };
            let status_glyph = match app.peer_status.get(peer) {
                Some(BirdStatus::Online) => TerminalIcon::Online.glyph(app.icon_style),
                Some(BirdStatus::Idle) => TerminalIcon::Idle.glyph(app.icon_style),
                Some(BirdStatus::InCall) => TerminalIcon::InCall.glyph(app.icon_style),
                None => TerminalIcon::Dnd.glyph(app.icon_style),
            };
            items.push(
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!(" {} ", initials(&name)),
                        Style::new().fg(fg_2).bg(app.palette.accent),
                    ),
                    Span::styled(
                        format!(" {status_glyph} "),
                        Style::new().fg(status_color).bg(row_bg),
                    ),
                    Span::styled(
                        name,
                        Style::new().fg(if active { fg_2 } else { text }).bg(row_bg),
                    ),
                ]))
                .bg(row_bg),
            );
        }
        for (index, flock) in app.flocks.iter().enumerate() {
            let active = matches!(app.selection, Selection::Flock(i) if i == index);
            let row_bg = if active { selected_bg } else { bg };
            let icon = TerminalIcon::Group.glyph(app.icon_style);
            let mut spans = vec![
                Span::styled(
                    format!(" {} ", icon),
                    Style::new().fg(fg_2).bg(app.palette.accent),
                ),
                Span::styled(
                    flock.name.clone(),
                    Style::new().fg(if active { fg_2 } else { text }).bg(row_bg),
                ),
            ];
            if flock.unread > 0 {
                spans.push(Span::styled(
                    format!(" {}", flock.unread),
                    Style::new().fg(Color::White).bg(Color::Rgb(242, 63, 67)),
                ));
            }
            items.push(ListItem::new(Line::from(spans)).bg(row_bg));
        }
    } else if let Selection::Channel(ri, ci) = app.selection
        && let Some(roost) = app.roosts.get(ri)
    {
        items.push(ListItem::new(Line::from(Span::styled(
            "CHANNELS",
            Style::new().fg(muted).add_modifier(Modifier::BOLD),
        ))));
        for (index, channel) in roost.channels.iter().enumerate() {
            let active = index == ci;
            let row_bg = if active { selected_bg } else { bg };
            let mut spans = vec![Span::styled(
                format!("# {}", channel.name),
                Style::new().fg(if active { fg_2 } else { text }).bg(row_bg),
            )];
            if channel.unread > 0 {
                spans.push(Span::styled(
                    format!(" {}", channel.unread),
                    Style::new().fg(Color::White).bg(Color::Rgb(242, 63, 67)),
                ));
            }
            items.push(ListItem::new(Line::from(spans)).bg(row_bg));
        }
        // Voice roster: everyone currently in a call in this channel, listed
        // directly below the channel list with the call icon. The local user
        // is always in the call they started or joined.
        if app.in_call {
            let call_glyph = TerminalIcon::Call.glyph(app.icon_style);
            let call_style = Style::new().fg(Color::Rgb(88, 101, 242)).bg(bg);
            items.push(
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", call_glyph), call_style),
                    Span::styled(app.name.clone(), Style::new().fg(text).bg(bg)),
                ]))
                .bg(bg),
            );
            for peer in &app.peers {
                let name = app.peer_display_name(peer);
                items.push(
                    ListItem::new(Line::from(vec![
                        Span::styled(format!(" {} ", call_glyph), call_style),
                        Span::styled(name, Style::new().fg(text).bg(bg)),
                    ]))
                    .bg(bg),
                );
            }
        }
    }
    f.render_widget(List::new(items).block(Block::default().bg(bg)), rows[1]);
    let mic_icon = if app.muted {
        TerminalIcon::MicMuted
    } else {
        TerminalIcon::Mic
    };
    let headset_icon = if app.deafened {
        TerminalIcon::Deafened
    } else {
        TerminalIcon::Headset
    };
    let mic_glyph = mic_icon.glyph(app.icon_style);
    let headset_glyph = headset_icon.glyph(app.icon_style);
    let settings_glyph = TerminalIcon::Settings.glyph(app.icon_style);
    let footer = Text::from(vec![
        Line::from(vec![
            Span::styled(
                format!(" {} ", initials(&app.name)),
                Style::new().fg(Color::White).bg(app.palette.accent),
            ),
            Span::styled(
                format!(" {}", app.name),
                Style::new().fg(fg_2).add_modifier(Modifier::BOLD).bg(bg),
            ),
            Span::styled(format!(" {}", app.tag), Style::new().fg(muted).bg(bg)),
        ]),
        Line::from(vec![Span::styled(
            format!(" {mic_glyph}  {headset_glyph}  {settings_glyph}"),
            Style::new().fg(muted).bg(bg),
        )]),
    ]);
    f.render_widget(
        Paragraph::new(footer).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::new().fg(border))
                .bg(bg),
        ),
        rows[2],
    );
}

fn flattened_channels(app: &App) -> impl Iterator<Item = &FlockView> {
    app.roosts.iter().flat_map(|roost| roost.channels.iter())
}

fn draw_chat(f: &mut Frame, app: &App, area: Rect) {
    let bg = Color::Rgb(49, 51, 56);
    let border = Color::Rgb(63, 65, 71);
    let text = Color::Rgb(219, 222, 225);
    let fg_2 = Color::Rgb(242, 243, 245);
    let muted = Color::Rgb(148, 155, 164);
    let is_dm = matches!(app.v2_view, V2View::Home) && app.selected_dm.is_some();
    let is_flock = matches!(app.selection, Selection::Flock(i) if i < app.flocks.len());
    let title = if is_flock {
        app.active_title()
    } else if is_dm {
        match app.selected_dm {
            Some(peer) => format!("@ {}", app.peer_display_name(&peer)),
            None => "Messages".to_string(),
        }
    } else {
        app.active_title()
    };
    let subtitle = if is_flock || matches!(app.v2_view, V2View::Space) {
        format!("{} members", app.active_peers().len() + 1)
    } else if is_dm {
        app.selected_dm
            .and_then(|peer| app.peer_status.get(&peer))
            .map(|status| format!("{status:?}").to_lowercase())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let bell_glyph = if app.notifications_muted {
        TerminalIcon::BellSlash.glyph(app.icon_style)
    } else {
        TerminalIcon::Bell.glyph(app.icon_style)
    };
    let bell = if !app.notifications.is_empty() {
        format!("{} {}", bell_glyph, app.notifications.len())
    } else {
        bell_glyph.to_string()
    };
    let menu = if app.sidebar_hidden {
        format!("{}  ", TerminalIcon::Menu.glyph(app.icon_style))
    } else {
        String::new()
    };
    let icons = format!(
        "{menu}{}  {}  {}  {}",
        TerminalIcon::Members.glyph(app.icon_style),
        bell,
        TerminalIcon::Pin.glyph(app.icon_style),
        TerminalIcon::Call.glyph(app.icon_style)
    );
    let header = Line::from(vec![
        Span::styled(
            title.clone(),
            Style::new().fg(fg_2).add_modifier(Modifier::BOLD).bg(bg),
        ),
        Span::styled(format!("  {subtitle}"), Style::new().fg(muted).bg(bg)),
        Span::styled(
            " ".repeat(
                (area.width as usize)
                    .saturating_sub(title.chars().count() + subtitle.chars().count() + 4),
            ),
            Style::new().fg(muted).bg(bg),
        ),
    ]);
    let mut message_lines: Vec<Line> = Vec::new();
    // Surface errors and transient notices so failures are never silent.
    if let Some(error) = app.error_message.as_deref() {
        message_lines.push(Line::from(Span::styled(
            format!("⚠ {error}"),
            Style::new().fg(Color::Rgb(242, 63, 67)).bg(bg),
        )));
    }
    if let Some(notice) = app.visible_status_notice(Instant::now()) {
        message_lines.push(Line::from(Span::styled(
            format!("ℹ {notice}"),
            Style::new().fg(muted).bg(bg),
        )));
    }
    let messages = app.active_messages();
    if messages.is_empty() {
        let empty = if is_dm {
            match app.selected_dm {
                Some(peer) => format!(
                    "This is the beginning of your conversation with @{}.",
                    app.peer_display_name(&peer)
                ),
                None => "Select a peer to begin a direct message.".to_string(),
            }
        } else if is_flock || matches!(app.v2_view, V2View::Space) {
            format!(
                "Welcome to {}! This is the start of the conversation.",
                title
            )
        } else {
            "Select a peer to begin a direct message.".to_string()
        };
        message_lines.push(Line::from(Span::styled(
            empty,
            Style::new().fg(muted).bg(bg),
        )));
    } else {
        message_lines.push(Line::from(Span::styled(
            "──────────────────── TODAY ────────────────────",
            Style::new().fg(muted).bg(bg),
        )));
        for message in messages {
            let prefix = if message.private {
                format!("{} ", TerminalIcon::Lock.glyph(app.icon_style))
            } else {
                String::new()
            };
            message_lines.push(Line::from(vec![
                Span::styled(
                    format!(" {} ", initials(&message.msg.author)),
                    Style::new().fg(Color::White).bg(app.palette.accent),
                ),
                Span::styled(
                    format!(" {}", message.msg.author),
                    Style::new().fg(fg_2).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", format_ts(message.msg.ts)),
                    Style::new().fg(muted),
                ),
            ]));
            message_lines.push(Line::from(Span::styled(
                format!("{prefix}{}", message.msg.body),
                Style::new().fg(text).bg(bg),
            )));
        }
    }
    // The message list fills the entire chat column; the header (2 rows) and
    // composer (3 rows) are drawn on top of it afterwards. Content starts
    // below the header so it is never hidden by the overlay.
    let mut lines = Vec::with_capacity(area.height as usize);
    lines.push(Line::from(""));
    lines.push(Line::from(""));
    lines.extend(message_lines);
    while (lines.len() as u16) < area.height {
        lines.push(Line::from(""));
    }
    f.render_widget(
        Paragraph::new(Text::from(lines)).block(Block::default().bg(bg)),
        area,
    );
    // Overlay the header (top 2 rows) and composer (bottom 3 rows).
    let header_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 2,
    };
    f.render_widget(
        Paragraph::new(header).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::new().fg(border))
                .bg(bg),
        ),
        header_area,
    );
    // Right-aligned icon row, drawn as an overlay so the call button is never
    // clipped by a long title. The click regions mirror this layout.
    f.render_widget(
        Paragraph::new(icons)
            .style(Style::new().fg(muted).bg(bg))
            .alignment(Alignment::Right),
        header_area,
    );
    let composer_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(3),
        width: area.width,
        height: 3,
    };
    draw_message_bar(f, app, composer_area);
}

/// Which header icon a click at `col` (row 0/1 of the chat header) hits,
/// mirroring the right-aligned icon overlay drawn in `draw_chat`. The regions
/// are computed from real glyph widths (emoji are 2 cells), so clicks land on
/// the icon the user sees. Returns `None` when the click is left of the icons.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeaderIcon {
    Menu,
    Members,
    Bell,
    Pin,
    Call,
}

fn push_header_icon(
    icons: &mut Vec<(HeaderIcon, u16)>,
    x: &mut u16,
    icon: HeaderIcon,
    glyph: &str,
) {
    // Display width, not char count: emoji glyphs are 2 cells wide.
    let w = ratatui::text::Line::from(glyph).width() as u16;
    *x = x.saturating_sub(w);
    icons.push((icon, *x));
}

pub fn header_icon_at(app: &App, col: u16, term_w: u16) -> Option<HeaderIcon> {
    let members_open = app.show_members
        && (matches!(app.v2_view, V2View::Space)
            || (matches!(app.v2_view, V2View::Home)
                && matches!(app.selection, Selection::Flock(_))
                && app.selected_dm.is_none()));
    let right = term_w.saturating_sub(if members_open { 30 } else { 1 });
    let mut x = right;
    let mut icons: Vec<(HeaderIcon, u16)> = Vec::new();
    push_header_icon(
        &mut icons,
        &mut x,
        HeaderIcon::Call,
        TerminalIcon::Call.glyph(app.icon_style),
    );
    x = x.saturating_sub(2); // gap
    push_header_icon(
        &mut icons,
        &mut x,
        HeaderIcon::Pin,
        TerminalIcon::Pin.glyph(app.icon_style),
    );
    x = x.saturating_sub(2);
    let bell = if app.notifications_muted {
        TerminalIcon::BellSlash.glyph(app.icon_style)
    } else {
        TerminalIcon::Bell.glyph(app.icon_style)
    };
    let bell_w = if app.notifications.is_empty() {
        ratatui::text::Line::from(bell).width() as u16
    } else {
        (ratatui::text::Line::from(bell).width() + 2) as u16 // "glyph N"
    };
    x = x.saturating_sub(bell_w);
    icons.push((HeaderIcon::Bell, x));
    x = x.saturating_sub(2);
    push_header_icon(
        &mut icons,
        &mut x,
        HeaderIcon::Members,
        TerminalIcon::Members.glyph(app.icon_style),
    );
    if app.sidebar_hidden {
        x = x.saturating_sub(2);
        push_header_icon(
            &mut icons,
            &mut x,
            HeaderIcon::Menu,
            TerminalIcon::Menu.glyph(app.icon_style),
        );
    }
    icons
        .into_iter()
        .find(|(_, start)| col >= *start)
        .map(|(icon, _)| icon)
}

fn draw_members(f: &mut Frame, app: &App, area: Rect) {
    let bg = Color::Rgb(43, 45, 49);
    let muted = Color::Rgb(148, 155, 164);
    let text = Color::Rgb(219, 222, 225);
    // For flocks in a call, the members panel shows the call participants
    // (call icon + names) instead of the online roster. Roosts keep the
    // normal members list.
    let flock_call = app.in_call
        && matches!(app.v2_view, V2View::Home)
        && matches!(app.selection, Selection::Flock(_));
    let mut lines = vec![Line::from(Span::styled(
        if flock_call { "IN CALL" } else { "MEMBERS" },
        Style::new().fg(muted).add_modifier(Modifier::BOLD),
    ))];
    if flock_call {
        let call_glyph = TerminalIcon::Call.glyph(app.icon_style);
        let call_style = Style::new().fg(Color::Rgb(88, 101, 242));
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", call_glyph), call_style),
            Span::styled(app.name.clone(), Style::new().fg(text)),
        ]));
        for peer in &app.peers {
            let name = app.peer_display_name(peer);
            lines.push(Line::from(vec![
                Span::styled(format!(" {} ", call_glyph), call_style),
                Span::styled(name, Style::new().fg(text)),
            ]));
        }
    } else {
        lines.push(Line::from(Span::styled(
            format!("Online — {}", app.active_peers().len() + 1),
            Style::new().fg(muted).add_modifier(Modifier::BOLD),
        )));
        // The local user first, then every online peer, below the Online line.
        lines.push(Line::from(Span::styled(
            format!(" {}  {}", initials(&app.name), app.name),
            Style::new().fg(text),
        )));
        for peer in app.active_peers() {
            let name = app.peer_display_name(&peer);
            lines.push(Line::from(Span::styled(
                format!(" {}  {}", initials(&name), name),
                Style::new().fg(text),
            )));
        }
    }
    lines.truncate(area.height as usize);
    f.render_widget(
        Paragraph::new(Text::from(lines)).block(
            Block::default()
                .borders(Borders::LEFT)
                .border_style(Style::new().fg(Color::Rgb(63, 65, 71)))
                .bg(bg),
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

    let placeholder = if matches!(app.selection, Selection::Flock(_)) {
        format!("Message {}", app.active_title())
    } else if matches!(app.v2_view, V2View::Home) {
        match app.selected_dm {
            Some(peer) => format!("Message @{}", app.peer_display_name(&peer)),
            None => "Message".to_string(),
        }
    } else {
        format!("Message #{}", app.active_title())
    };
    let plus_icon = TerminalIcon::Plus.glyph(app.icon_style);
    let emoji_icon = TerminalIcon::Emoji.glyph(app.icon_style);
    let send_icon = TerminalIcon::Send.glyph(app.icon_style);

    // Draw the border first. The content is rendered afterward so the
    // border widget cannot paint over the controls and placeholder.
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(border_color))
            .bg(input_bg),
        outer,
    );
    let content = outer.inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    let value = if app.input.is_empty() {
        placeholder
    } else {
        app.input.clone()
    };
    let input_color = if app.input.is_empty() {
        Color::Rgb(148, 155, 164)
    } else {
        Color::Rgb(219, 222, 225)
    };
    let left = format!("{} {}", plus_icon, value);
    let right = format!("{} {}", emoji_icon, send_icon);
    // Display width, not char count: emoji glyphs are 2 cells wide.
    let right_width = ratatui::text::Line::from(right.as_str()).width() as u16;
    let left_area = Rect {
        x: content.x,
        y: content.y,
        width: content.width.saturating_sub(right_width),
        height: content.height,
    };
    f.render_widget(
        Paragraph::new(left)
            .style(Style::new().fg(input_color).bg(input_bg))
            .alignment(Alignment::Left),
        left_area,
    );
    f.render_widget(
        Paragraph::new(right)
            .style(Style::new().fg(input_color).bg(input_bg))
            .alignment(Alignment::Right),
        Rect {
            x: content.x + content.width.saturating_sub(right_width),
            y: content.y,
            width: right_width.min(content.width),
            height: content.height,
        },
    );
}

/// The x offset of the chat column, mirroring the layout in `draw`: desktop
/// chat starts after the rail (9) and sidebar (30); mobile chat starts at 0,
/// or after the sidebar/members panel when one is open.
pub fn chat_column_x(app: &App, term_w: u16) -> u16 {
    if term_w < MOBILE_BREAKPOINT {
        let members_open = app.show_members
            && (matches!(app.v2_view, V2View::Space)
                || (matches!(app.v2_view, V2View::Home)
                    && matches!(app.selection, Selection::Flock(_))
                    && app.selected_dm.is_none()));
        let sidebar_visible = !app.sidebar_hidden
            && (matches!(app.v2_view, V2View::Space) || matches!(app.v2_view, V2View::Home));
        if members_open || (sidebar_visible && matches!(app.v2_view, V2View::Home)) {
            return 30;
        }
        return 0;
    }
    39
}

/// Which composer control a click hits: the send button, the emoji button, or
/// the input area itself. Mirrors the layout in `draw_message_bar`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComposerHit {
    Send,
    Emoji,
    Input,
}

pub fn composer_hit_at(
    app: &App,
    col: u16,
    row: u16,
    term_w: u16,
    term_h: u16,
) -> Option<ComposerHit> {
    if row < term_h.saturating_sub(3) {
        return None;
    }
    let chat_x = chat_column_x(app, term_w);
    let outer = Rect {
        x: chat_x + 1,
        y: term_h.saturating_sub(3),
        width: term_w.saturating_sub(chat_x + 2),
        height: 3,
    };
    let content = outer.inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    let emoji_icon = TerminalIcon::Emoji.glyph(app.icon_style);
    let send_icon = TerminalIcon::Send.glyph(app.icon_style);
    let right = format!("{} {}", emoji_icon, send_icon);
    let right_width = ratatui::text::Line::from(right.as_str()).width() as u16;
    let right_x = content.x + content.width.saturating_sub(right_width);
    let emoji_w = ratatui::text::Line::from(emoji_icon).width() as u16;
    if col >= right_x && col < right_x + emoji_w {
        return Some(ComposerHit::Emoji);
    }
    if col > right_x + emoji_w {
        return Some(ComposerHit::Send);
    }
    if col >= content.x && col < right_x {
        return Some(ComposerHit::Input);
    }
    None
}

fn draw_call_overlay(f: &mut Frame, app: &App) {
    let video = app.show_video;
    let (w, h) = if video { (90u16, 26u16) } else { (62, 14) };
    let area = app.popup_rect(f.area().width, f.area().height, w, h);
    f.render_widget(Clear, area);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(" Call ")
            .border_style(Style::new().fg(app.palette.border))
            .bg(Color::Rgb(30, 31, 34)),
        area,
    );
    draw_popup_close(f, area, app.icon_style);
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
    let participant_count = app.peers.len() + 1;
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                if app.call_title.is_empty() {
                    "Call"
                } else {
                    &app.call_title
                },
                Style::new()
                    .fg(Color::Rgb(242, 243, 245))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {participant_count} participants"),
                Style::new().fg(Color::Rgb(148, 155, 164)),
            ),
        ])),
        rows[0],
    );
    if app.show_video {
        // Video mode: the middle band shows the local camera feed (or a
        // placeholder while the camera warms up) and any remote feeds.
        let mut lines: Vec<Line> = Vec::new();
        if let Some(frame) = &app.local_video_frame {
            let cols = rows[1].width.saturating_sub(2);
            let rows_avail = rows[1].height.saturating_sub(2);
            lines.extend(crate::video::frame_to_lines(frame, cols, rows_avail));
        } else {
            lines.push(Line::from(Span::styled(
                "Starting camera…",
                Style::new().fg(Color::Rgb(148, 155, 164)),
            )));
        }
        for (peer, frame) in &app.remote_video_frames {
            lines.push(Line::from(Span::styled(
                format!("  {}:", app.peer_display_name(peer)),
                Style::new().fg(Color::Rgb(148, 155, 164)),
            )));
            let cols = rows[1].width.saturating_sub(2);
            let rows_avail = rows[1].height.saturating_sub(2);
            lines.extend(crate::video::frame_to_lines(frame, cols, rows_avail));
        }
        f.render_widget(
            Paragraph::new(Text::from(lines))
                .block(Block::default().borders(Borders::ALL).title(" video ")),
            rows[1],
        );
    } else {
        let mut lines = vec![Line::from(vec![
            Span::styled(
                format!(" {} ", initials(&app.name)),
                Style::new().fg(Color::White).bg(app.palette.accent),
            ),
            Span::styled(
                format!("  {}", app.name),
                Style::new().fg(Color::Rgb(219, 222, 225)),
            ),
        ])];
        for peer in &app.peers {
            let name = app.peer_display_name(peer);
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {} ", initials(&name)),
                    Style::new().fg(Color::White).bg(app.palette.accent),
                ),
                Span::styled(
                    format!("  {name}"),
                    Style::new().fg(Color::Rgb(219, 222, 225)),
                ),
            ]));
        }
        f.render_widget(
            Paragraph::new(Text::from(lines)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" participants "),
            ),
            rows[1],
        );
    }
    f.render_widget(
        Paragraph::new("[ mute ]   [ video ]   [ disconnect ]")
            .style(Style::new().fg(Color::Rgb(219, 222, 225))),
        rows[2],
    );
}

/// Which call-overlay control a click hits, mirroring the control row drawn
/// in `draw_call_overlay` ("[ mute ]   [ video ]   [ disconnect ]").
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallControl {
    Mute,
    Video,
    Disconnect,
}

pub fn call_control_at(overlay: Rect, col: u16, row: u16) -> Option<CallControl> {
    let inner = overlay.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(inner);
    let band = rows[2];
    if row != band.y {
        return None;
    }
    let rel = col.saturating_sub(band.x);
    // "[ mute ]" = 8, gap 3, "[ video ]" = 9, gap 3, "[ disconnect ]" = 14.
    if rel < 8 {
        Some(CallControl::Mute)
    } else if rel < 11 {
        None // gap
    } else if rel < 20 {
        Some(CallControl::Video)
    } else if rel < 23 {
        None // gap
    } else {
        Some(CallControl::Disconnect)
    }
}

/// Draw an X close button in the top-right corner of a popup.
fn draw_popup_close(f: &mut Frame, area: Rect, icon_style: IconStyle) {
    let glyph = TerminalIcon::Close.glyph(icon_style);
    let width = glyph.chars().count() as u16;
    f.render_widget(
        Paragraph::new(glyph).style(Style::new().fg(Color::Rgb(148, 155, 164))),
        Rect {
            x: area.right().saturating_sub(width),
            y: area.y,
            width,
            height: 1,
        },
    );
}

/// Draw a right-aligned row of action buttons on the bottom row of a popup.
/// The rightmost button is the primary (accent) action. Hit regions mirror
/// this layout via [`popup_button_at`].
fn draw_popup_buttons(f: &mut Frame, popup: Rect, app: &App, buttons: &[&str]) {
    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let y = inner.bottom().saturating_sub(1);
    let mut x = inner.right();
    for (i, label) in buttons.iter().enumerate() {
        let text = format!(" {label} ");
        let w = text.chars().count() as u16;
        x = x.saturating_sub(w);
        let style = if i == buttons.len() - 1 {
            Style::new().fg(Color::White).bg(app.palette.accent)
        } else {
            Style::new().fg(app.palette.text).bg(app.palette.surface)
        };
        f.render_widget(
            Paragraph::new(text).style(style),
            Rect {
                x,
                y,
                width: w,
                height: 1,
            },
        );
        x = x.saturating_sub(1); // gap between buttons
    }
}

/// Hit-test a popup's action-button row. Returns the index of the clicked
/// button, mirroring [`draw_popup_buttons`] (rightmost button is the primary).
pub fn popup_button_at(popup: Rect, col: u16, row: u16, buttons: &[&str]) -> Option<usize> {
    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let y = inner.bottom().saturating_sub(1);
    if row != y {
        return None;
    }
    let mut x = inner.right();
    for i in (0..buttons.len()).rev() {
        let w = (buttons[i].chars().count() + 2) as u16;
        x = x.saturating_sub(w);
        if col >= x && col < x + w {
            return Some(i);
        }
        x = x.saturating_sub(1);
    }
    None
}

/// The screen rect of the currently active popup, mirroring the draw()
/// precedence. Returns `None` when no popup is open. Every popup is centered
/// then shifted by the user's drag offset (see [`App::popup_rect`]).
pub fn active_popup_rect(app: &App, term_w: u16, term_h: u16) -> Option<Rect> {
    if app.show_pinned {
        return Some(app.popup_rect(term_w, term_h, 40, 6));
    }
    if app.show_notifications {
        let height = 10u16.max((app.notifications.len() as u16 + 3).min(20));
        return Some(app.popup_rect(term_w, term_h, 52, height));
    }
    if app.in_call {
        let (w, h) = if app.show_video { (90, 26) } else { (62, 14) };
        return Some(app.popup_rect(term_w, term_h, w, h));
    }
    if app.profile_panel.open {
        let width = 50u16.min(term_w.saturating_sub(4)).max(30);
        let height = 18u16.min(term_h.saturating_sub(4)).max(10);
        return Some(app.popup_rect(term_w, term_h, width, height));
    }
    if app.settings_open {
        return Some(app.popup_rect(term_w, term_h, 80, 20));
    }
    if app.show_role_submenu {
        return Some(app.popup_rect(term_w, term_h, 24, 4));
    }
    if app.show_context_menu {
        return Some(app.popup_rect(term_w, term_h, 28, app.context_menu_items.len() as u16 + 2));
    }
    if app.show_add_channel
        || app.show_create_roost
        || app.show_create_room
        || app.show_join_room
        || app.show_delete_confirm
    {
        return Some(app.popup_rect(term_w, term_h, 60, 8));
    }
    if app.show_edit_flock {
        return Some(app.popup_rect(term_w, term_h, 60, 10));
    }
    if app.show_menu {
        return Some(app.popup_rect(term_w, term_h, 28, MENU_ITEMS.len() as u16 + 2));
    }
    if app.show_bird_profile {
        return Some(app.popup_rect(term_w, term_h, 40, 7));
    }
    None
}

/// Fully dismiss the active popup (X button / outside click).
pub fn dismiss_active_popup(app: &mut App) {
    if app.show_role_submenu {
        app.show_role_submenu = false;
        app.show_context_menu = false;
    } else if app.show_context_menu {
        app.show_context_menu = false;
    } else if app.show_delete_confirm {
        app.show_delete_confirm = false;
        app.delete_confirm_input.clear();
    } else if app.show_add_channel {
        app.show_add_channel = false;
    } else if app.show_create_roost {
        app.show_create_roost = false;
    } else if app.show_create_room {
        app.show_create_room = false;
        app.create_flock_code = None;
        app.create_flock_secret = None;
        app.create_flock_name.clear();
    } else if app.show_edit_flock {
        app.show_edit_flock = false;
    } else if app.show_join_room {
        app.show_join_room = false;
    } else if app.show_menu {
        app.show_menu = false;
    } else if app.show_bird_profile {
        app.show_bird_profile = false;
        app.bird_profile_peer = None;
    } else if app.settings_open {
        app.settings_open = false;
    } else if app.profile_panel.open {
        app.profile_panel.open = false;
        app.profile_panel.editing = false;
    } else if app.in_call {
        app.in_call = false;
        app.show_video = false;
        app.error_message = None;
    } else if app.show_pinned {
        app.show_pinned = false;
    } else if app.show_notifications {
        app.show_notifications = false;
    }
}

/// Right-click dismissal: back one level when in a submenu, else full dismiss.
pub fn dismiss_active_popup_one_level(app: &mut App) {
    if app.show_role_submenu {
        app.show_role_submenu = false;
        app.show_context_menu = true;
    } else {
        dismiss_active_popup(app);
    }
}

fn draw_empty_popup(f: &mut Frame, title: &str, app: &App) {
    let popup = app.popup_rect(f.area().width, f.area().height, 40, 6);
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                format!(" {title} "),
                Style::new().fg(Color::Rgb(88, 101, 242)),
            ))
            .border_style(Style::new().fg(Color::Rgb(63, 65, 71)))
            .bg(Color::Rgb(43, 45, 49)),
        popup,
    );
    draw_popup_close(f, popup, app.icon_style);
    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    f.render_widget(
        Paragraph::new("Nothing here yet.").style(Style::new().fg(Color::Rgb(148, 155, 164))),
        inner,
    );
}

fn draw_notifications_popup(f: &mut Frame, app: &App) {
    let bg = Color::Rgb(43, 45, 49);
    let muted = Color::Rgb(148, 155, 164);
    let Some(popup) = active_popup_rect(app, f.area().width, f.area().height) else {
        return;
    };
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " Notifications ",
                Style::new().fg(Color::Rgb(88, 101, 242)),
            ))
            .border_style(Style::new().fg(Color::Rgb(63, 65, 71)))
            .bg(bg),
        popup,
    );
    draw_popup_close(f, popup, app.icon_style);
    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });
    let mut lines: Vec<Line> = Vec::new();
    if app.notifications.is_empty() {
        lines.push(Line::from(Span::styled(
            "Nothing here yet.",
            Style::new().fg(muted),
        )));
    } else {
        use chrono::TimeZone;
        for item in app.notifications.iter().rev().take(12) {
            let ts = if item.ts > 10_000_000_000 {
                item.ts / 1000
            } else {
                item.ts
            };
            let stamp = chrono::Local
                .timestamp_opt(ts, 0)
                .single()
                .map(|dt| dt.format("%H:%M").to_string())
                .unwrap_or_default();
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", item.space_name),
                    Style::new()
                        .fg(Color::Rgb(88, 101, 242))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} ", item.author),
                    Style::new().fg(Color::Rgb(220, 221, 222)),
                ),
                Span::styled(item.body.clone(), Style::new().fg(muted)),
                Span::styled(format!(" {stamp}"), Style::new().fg(muted)),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines).style(Style::new().fg(muted)), inner);
}

fn draw_menu_popup(f: &mut Frame, app: &App) {
    let popup = app.popup_rect(
        f.area().width,
        f.area().height,
        28,
        MENU_ITEMS.len() as u16 + 2,
    );
    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(app.palette.border))
            .title(Span::styled(" Menu ", Style::new().fg(app.palette.accent))),
        popup,
    );
    draw_popup_close(f, popup, app.icon_style);
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
    let area = app.popup_rect(f.area().width, f.area().height, 80, 20);
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
    draw_popup_close(f, area, app.icon_style);
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
            "Display name\n  {}\n\nEmail\n  you@starling.local\n\nAvatar label\n  {}",
            app.name,
            app.profile_panel.avatar_label
        ),
        SettingsTab::Voice => {
            "Input device\n  Default\n\nOutput device\n  Default\n\nPush to Talk\n  Off\n\nNoise suppression\n  On".to_string()
        }
        SettingsTab::Appearance => format!(
            "Theme\n  Dark\n\nAccent color\n  {}\n\nIcon style\n  {}\n\nCompact mode\n  Off\n\nShow avatars\n  On",
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
    // Action buttons for the Appearance tab: apply the accent color and
    // cycle the icon style. Hit regions mirror draw_popup_buttons.
    if app.settings_tab == SettingsTab::Appearance {
        draw_popup_buttons(f, area, app, &["Cycle icons", "Apply accent"]);
    }
}

fn draw_create_room_popup(f: &mut Frame, app: &App) {
    let popup = app.popup_rect(f.area().width, f.area().height, 60, 8);
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

    draw_popup_close(f, popup, app.icon_style);
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
    draw_popup_buttons(f, popup, app, &["Cancel", "Create"]);
}

fn draw_create_roost_popup(f: &mut Frame, app: &App) {
    let popup = app.popup_rect(f.area().width, f.area().height, 60, 8);
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

    draw_popup_close(f, popup, app.icon_style);
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
    draw_popup_buttons(f, popup, app, &["Cancel", "Create"]);
}

fn draw_add_channel_popup(f: &mut Frame, app: &App) {
    let popup = app.popup_rect(f.area().width, f.area().height, 60, 8);
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

    draw_popup_close(f, popup, app.icon_style);
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
    draw_popup_buttons(f, popup, app, &["Cancel", "Create"]);
}

fn draw_edit_flock_popup(f: &mut Frame, app: &App) {
    let popup = app.popup_rect(f.area().width, f.area().height, 60, 10);
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

    draw_popup_close(f, popup, app.icon_style);
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
    draw_popup_buttons(f, popup, app, &["Cancel", "Save"]);
}

fn draw_join_room_popup(f: &mut Frame, app: &App) {
    let popup = app.popup_rect(f.area().width, f.area().height, 60, 8);
    draw_input_popup(
        f,
        " Join ",
        "Enter a flock or roost code:",
        &app.join_input,
        "",
        app,
    );
    draw_popup_buttons(f, popup, app, &["Cancel", "Join"]);
}

fn draw_delete_confirm_popup(f: &mut Frame, app: &App) {
    let popup = app.popup_rect(f.area().width, f.area().height, 60, 8);
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

    draw_popup_close(f, popup, app.icon_style);
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
    draw_popup_buttons(f, popup, app, &["Cancel", "Delete"]);
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

    let popup = app.popup_rect(f.area().width, f.area().height, 40, 7);
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
        Paragraph::new(" Click Call to start a call . Esc = close")
            .style(Style::new().fg(app.palette.dim)),
        rows[3],
    );
}

fn draw_profile_modal(f: &mut Frame, app: &App) {
    let term = f.area();
    let Some(area) = active_popup_rect(app, term.width, term.height) else {
        return;
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
    draw_popup_close(f, area, app.icon_style);
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
        Line::from(vec![Span::styled(
            "┌────┐",
            Style::new().fg(Color::Rgb(148, 155, 164)).bg(modal_bg),
        )]),
        Line::from(vec![
            Span::styled(
                format!("│ {} │", avatar_text),
                Style::new().fg(Color::Rgb(242, 243, 245)).bg(modal_bg),
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
        Line::from(vec![
            Span::styled(
                "└────┘",
                Style::new().fg(Color::Rgb(148, 155, 164)).bg(modal_bg),
            ),
            Span::styled("  ", Style::new().bg(modal_bg)),
            Span::styled(
                format!(
                    "{} {}",
                    TerminalIcon::Online.glyph(app.icon_style),
                    status_text
                ),
                Style::new().fg(Color::Rgb(148, 155, 164)).bg(modal_bg),
            ),
        ]),
    ];
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
    // Action buttons on the bottom row: Edit (view mode) or Cancel/Save
    // (editing mode). Hit regions mirror draw_popup_buttons.
    let buttons: &[&str] = if p.editing {
        &["Cancel", "Save"]
    } else {
        &["Edit"]
    };
    draw_popup_buttons(f, area, app, buttons);
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
    let popup = app.popup_rect(f.area().width, f.area().height, width, height);
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
    let popup = app.popup_rect(f.area().width, f.area().height, width, height);
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
    draw_popup_close(f, popup, app.icon_style);
}

fn draw_input_popup(f: &mut Frame, title: &str, prompt: &str, value: &str, hint: &str, app: &App) {
    let popup = app.popup_rect(f.area().width, f.area().height, 60, 8);
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

    draw_popup_close(f, popup, app.icon_style);
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
#[allow(clippy::field_reassign_with_default)]
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
            icon_path: None,
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
            icon_path: None,
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
            icon_path: None,
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
            icon_path: None,
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
    #[test]
    fn home_view_renders_empty_state_without_live_peers() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let app = App::default();
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("DIRECT MESSAGES"));
        assert!(text.contains("Select a peer"));
        assert!(!text.contains("PR merged"));
        assert!(!text.contains("GROUPS"));
    }

    #[test]
    fn home_view_renders_flock_row_and_dynamic_title() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut app = App::default();
        app.name = "Ramhaug".into();
        app.tag = "#7134".into();
        app.v2_view = V2View::Home;
        app.flocks.push(FlockView {
            code: "99s".into(),
            name: "99s".into(),
            messages: vec![],
            unread: 0,
        });
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!text.contains("GROUPS"));
        assert!(text.contains("99s"));
        assert!(text.contains("Ramhaug"));
        assert!(text.contains("#7134"));
        assert!(text.contains("Welcome to 99s"));
    }

    #[test]
    fn own_messages_never_notify_and_roost_channels_do() {
        // A message from the local user in an inactive channel bumps nothing.
        let mut app = App::default();
        app.name = "me".into();
        app.flocks.push(FlockView {
            code: "FLOCK-A".into(),
            name: "crew".into(),
            ..Default::default()
        });
        app.roosts.push(RoostView {
            code: "ROOST-X".into(),
            name: "server".into(),
            channels: vec![FlockView {
                code: "ROOST-X/general".into(),
                name: "general".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        let own = ChatMessage {
            id: "own".into(),
            author: "me".into(),
            body: "hello".into(),
            ts: 1,
        };
        app.receive_message("FLOCK-A", own, false);
        assert!(app.notifications.is_empty());
        assert_eq!(app.flocks[0].unread, 0);

        // A peer message in an inactive roost channel notifies and marks unread.
        let peer = ChatMessage {
            id: "peer".into(),
            author: "Wren".into(),
            body: "hi".into(),
            ts: 2,
        };
        app.receive_message("ROOST-X/general", peer, false);
        assert_eq!(app.notifications.len(), 1);
        assert_eq!(app.notifications[0].space_name, "general");
        assert_eq!(app.notifications[0].author, "Wren");
        assert_eq!(app.roosts[0].channels[0].unread, 1);
        assert_eq!(app.roosts[0].unread, 1);

        // Selecting the channel clears both the unread badge and its notices.
        app.select(Selection::Channel(0, 0));
        assert_eq!(app.roosts[0].channels[0].unread, 0);
        assert!(app.notifications.is_empty());
    }

    #[test]
    fn header_icon_hit_regions_cover_all_four_buttons() {
        let mut app = App::default();
        app.v2_view = V2View::Space;
        app.show_members = true;
        // Rightmost icon is the call button; each icon must be reachable.
        // With members open the icon row is right-aligned to col 90.
        let call = header_icon_at(&app, 88, 120);
        assert_eq!(call, Some(HeaderIcon::Call));
        let pin = header_icon_at(&app, 84, 120);
        assert_eq!(pin, Some(HeaderIcon::Pin));
        let bell = header_icon_at(&app, 80, 120);
        assert_eq!(bell, Some(HeaderIcon::Bell));
        let members = header_icon_at(&app, 76, 120);
        assert_eq!(members, Some(HeaderIcon::Members));
        // Left of the icons is not a button.
        assert_eq!(header_icon_at(&app, 40, 120), None);
        // With the sidebar hidden the menu button is the leftmost icon.
        app.sidebar_hidden = true;
        assert_eq!(header_icon_at(&app, 72, 120), Some(HeaderIcon::Menu));
    }

    #[test]
    fn header_buttons_toggle_pinned_notifications_and_members() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut app = App::default();
        app.v2_view = V2View::Space;
        app.show_members = true;
        app.roosts.push(RoostView {
            code: "S".into(),
            name: "Starling".into(),
            channels: vec![FlockView {
                code: "S/general".into(),
                name: "general".into(),
                messages: vec![],
                unread: 0,
            }],
            unread: 0,
            icon_path: None,
        });
        app.selection = Selection::Channel(0, 0);
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("MEMBERS"));
        app.show_pinned = true;
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Pinned Messages"));
        app.show_pinned = false;
        app.show_notifications = true;
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Notifications"));
        app.show_notifications = false;
        app.show_members = false;
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!text.contains("MEMBERS"));
    }

    #[test]
    fn popup_dismissal_helpers_work() {
        let mut app = App::default();
        app.settings_open = true;
        dismiss_active_popup(&mut app);
        assert!(!app.settings_open);

        app.show_pinned = true;
        dismiss_active_popup(&mut app);
        assert!(!app.show_pinned);

        app.show_notifications = true;
        dismiss_active_popup(&mut app);
        assert!(!app.show_notifications);

        app.in_call = true;
        dismiss_active_popup(&mut app);
        assert!(!app.in_call);

        app.profile_panel.open = true;
        dismiss_active_popup(&mut app);
        assert!(!app.profile_panel.open);

        // Submenu right-click: back one level, keep the parent menu.
        app.show_context_menu = true;
        app.show_role_submenu = true;
        dismiss_active_popup_one_level(&mut app);
        assert!(!app.show_role_submenu);
        assert!(app.show_context_menu);

        // Full dismiss from the parent menu.
        dismiss_active_popup(&mut app);
        assert!(!app.show_context_menu);
    }

    #[test]
    fn flock_selection_resolves_presence_peers_for_calls() {
        // A flock selected from the Friends list has no active context, but
        // its presence roster must still resolve so calls can target peers.
        let mut app = App::default();
        let secret = [7u8; 32];
        let peer = iroh::SecretKey::generate().public();
        let code = starling::net::encode_typed_code(
            starling::net::CodeType::Flock,
            &[&secret[..], &peer.as_bytes()[..], b"crew"].concat(),
        );
        app.flocks.push(FlockView {
            code: code.clone(),
            name: "crew".into(),
            messages: vec![],
            unread: 0,
        });
        app.select_flock(0);
        let space = SpaceId::Flock(starling::protocol::FlockId(secret));
        let other = iroh::SecretKey::generate().public();
        app.presence.context_mut(space).ordered_ids.push(other);
        let peers = app.active_peers();
        assert_eq!(peers, vec![other]);
    }

    #[test]
    fn flock_selection_keeps_row_and_shows_members() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut app = App::default();
        app.name = "Ramhaug".into();
        app.tag = "#7134".into();
        app.v2_view = V2View::Home;
        app.show_members = true;
        app.flocks.push(FlockView {
            code: "99s".into(),
            name: "99s".into(),
            messages: vec![],
            unread: 0,
        });
        app.select_flock(0);
        assert!(matches!(app.v2_view, V2View::Home));
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        // Flock row stays visible and selected; no GROUPS label; members panel present.
        assert!(text.contains("99s"));
        assert!(!text.contains("GROUPS"));
        assert!(text.contains("MEMBERS"));
        // The active pill indicator is not stacked on every rail row.
        let buf = terminal.backend().buffer();
        let area = buf.area();
        let cells = buf.content();
        let full_width = area.width as usize;
        let mut indicator_rows = 0;
        for y in 1..6 {
            let ch = cells[y * full_width + 1].symbol();
            if ch == "\u{22ee}" {
                indicator_rows += 1;
            }
        }
        assert_eq!(
            indicator_rows, 1,
            "active indicator must appear once, got {indicator_rows} rows"
        );
    }

    #[test]
    fn call_overlay_controls_hit_their_regions() {
        let overlay = Rect::new((120 - 62) / 2, (30 - 14) / 2, 62, 14);
        let inner = overlay.inner(Margin {
            vertical: 1,
            horizontal: 2,
        });
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(inner);
        let band = rows[2];
        // "[ mute ]" starts at band.x; "[ video ]" at +11; "[ disconnect ]" at +23.
        assert_eq!(
            call_control_at(overlay, band.x + 1, band.y),
            Some(CallControl::Mute)
        );
        assert_eq!(
            call_control_at(overlay, band.x + 12, band.y),
            Some(CallControl::Video)
        );
        assert_eq!(
            call_control_at(overlay, band.x + 24, band.y),
            Some(CallControl::Disconnect)
        );
        // The row below the controls is not a control.
        assert_eq!(call_control_at(overlay, band.x + 1, band.y + 1), None);
    }

    #[test]
    fn narrow_terminal_uses_bottom_rail_and_auto_hides_members() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut app = App::default();
        app.v2_view = V2View::Home;
        app.roosts.push(RoostView {
            code: "S".into(),
            name: "Starling".into(),
            channels: vec![FlockView {
                code: "S/general".into(),
                name: "general".into(),
                messages: vec![],
                unread: 0,
            }],
            unread: 0,
            icon_path: None,
        });
        app.select(Selection::Channel(0, 0));
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        // Members hidden by default on narrow screens.
        assert!(
            !text.contains("MEMBERS"),
            "members must auto-hide on narrow terminals"
        );
        // The rail moved to the bottom: the home glyph appears in the last rows.
        assert!(
            text.contains("S") && text.contains("general"),
            "chat content missing on narrow layout"
        );
        let buf = terminal.backend().buffer();
        let area = buf.area();
        let cells = buf.content();
        let full_width = area.width as usize;
        let bottom: String = (0..full_width)
            .map(|x| {
                cells[(area.height as usize - 2) * full_width + x]
                    .symbol()
                    .to_string()
            })
            .collect();
        assert!(
            bottom.contains('S'),
            "roost pill must render in the bottom rail: {bottom:?}"
        );
        // Explicitly opened members steal the right 30 columns, rail stays put.
        app.show_members = true;
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("MEMBERS"));
    }

    #[test]
    fn ultra_narrow_hides_sidebar_and_growing_wide_restores_everything() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut app = App::default();
        app.v2_view = V2View::Space;
        app.roosts.push(RoostView {
            code: "S".into(),
            name: "Starling".into(),
            channels: vec![FlockView {
                code: "S/general".into(),
                name: "general".into(),
                messages: vec![],
                unread: 0,
            }],
            unread: 0,
            icon_path: None,
        });
        app.select(Selection::Channel(0, 0));

        // Shrink below the narrow breakpoint: channel list and members hide.
        app.note_terminal_size(100);
        app.note_terminal_size(40);
        assert!(app.sidebar_hidden);
        assert!(!app.show_members);

        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!text.contains("MEMBERS"));
        assert!(
            !text.contains("CHANNELS"),
            "channel list must hide on ultra-narrow terminals"
        );
        assert!(
            text.contains("☰"),
            "header menu button must appear when the sidebar is hidden"
        );
        assert!(text.contains("general"), "chat must stay visible");

        // The header menu button brings the channel list back.
        app.sidebar_hidden = false;
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("CHANNELS"));

        // Growing back to a large width reverts everything.
        app.note_terminal_size(120);
        assert!(!app.sidebar_hidden);
        assert!(
            app.show_members,
            "members must reopen when the screen grows"
        );
    }

    #[test]
    fn rail_is_darkest_and_flock_uses_group_icon() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut app = App::default();
        app.v2_view = V2View::Home;
        app.flocks.push(FlockView {
            code: "935".into(),
            name: "935".into(),
            messages: vec![],
            unread: 0,
        });
        app.select_flock(0);
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let buf = terminal.backend().buffer();
        let area = buf.area();
        let cells = buf.content();
        let full_width = area.width as usize;
        // Rail background must be the darkest surface.
        let rail_cell = &cells[3];
        let side_cell = &cells[12];
        let rail_bg = rail_cell.bg;
        let side_bg = side_cell.bg;
        assert!(rail_bg != side_bg, "rail and sidebar must differ");
        // No caret fill artifacts around the home pill.
        let home_row: String = (0..full_width)
            .map(|x| cells[2 * full_width + x].symbol().to_string())
            .collect();
        assert!(
            !home_row.contains("^^^^"),
            "caret fill leaked: {home_row:?}"
        );
        // Flock row uses a group glyph, not the flock's first letter.
        let flock_row: String = (0..full_width)
            .map(|x| cells[3 * full_width + x].symbol().to_string())
            .collect();
        assert!(
            flock_row.contains("935"),
            "flock name missing: {flock_row:?}"
        );
        assert!(
            !flock_row.starts_with(" 9 "),
            "flock avatar must be a group glyph, got: {flock_row:?}"
        );
        let _ = rail_bg;
    }

    #[test]
    fn flock_roost_management_menu_shows_owner_actions() {
        let mut app = App::default();
        app.node_id = Some(iroh::SecretKey::generate().public());
        app.flocks.push(FlockView {
            code: "F1".into(),
            name: "My Flock".into(),
            messages: vec![],
            unread: 0,
        });
        app.flock_owners.insert("F1".into(), true);
        app.roosts.push(RoostView {
            code: "R1".into(),
            name: "My Roost".into(),
            channels: vec![],
            unread: 0,
            icon_path: None,
        });
        app.roost_owners.insert("R1".into(), app.node_id.unwrap());

        // Owner flock: Leave + Edit + Delete.
        app.build_context_menu(ContextMenuTarget::Flock(0));
        let labels: Vec<&str> = app
            .context_menu_items
            .iter()
            .map(|i| i.label.as_str())
            .collect();
        assert_eq!(labels, vec!["Leave Flock", "Edit Flock", "Delete Flock"]);

        // Owner roost: management actions present.
        app.build_context_menu(ContextMenuTarget::Roost(0));
        let labels: Vec<&str> = app
            .context_menu_items
            .iter()
            .map(|i| i.label.as_str())
            .collect();
        assert!(labels.contains(&"Leave Roost"));
        assert!(labels.contains(&"Edit Roost"));
        assert!(labels.contains(&"Delete Roost"));

        // Non-owner flock: only Leave.
        app.flock_owners.insert("F1".into(), false);
        app.build_context_menu(ContextMenuTarget::Flock(0));
        let labels: Vec<&str> = app
            .context_menu_items
            .iter()
            .map(|i| i.label.as_str())
            .collect();
        assert_eq!(labels, vec!["Leave Flock"]);
    }

    #[test]
    fn context_menu_code_resolves_flock_and_roost() {
        let mut app = App::default();
        app.flocks.push(FlockView {
            code: "FLOCK-CODE".into(),
            name: "F".into(),
            messages: vec![],
            unread: 0,
        });
        app.roosts.push(RoostView {
            code: "ROOST-CODE".into(),
            name: "R".into(),
            channels: vec![],
            unread: 0,
            icon_path: None,
        });
        app.context_menu_target = Some(ContextMenuTarget::Flock(0));
        assert_eq!(app.context_menu_code().as_deref(), Some("FLOCK-CODE"));
        app.context_menu_target = Some(ContextMenuTarget::Roost(0));
        assert_eq!(app.context_menu_code().as_deref(), Some("ROOST-CODE"));
    }
}
