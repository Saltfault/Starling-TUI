mod at_rest;
mod call;
mod clipboard;
mod event;
mod history;
mod history_store;
mod net;
#[cfg(feature = "audio")]
mod opus_ffi;
mod persistence;
#[cfg(feature = "audio")]
mod playback;
mod sanitize;
mod setup;
mod sync;
mod ui;
mod video;
#[cfg(feature = "audio")]
mod voice;

use crate::clipboard::Clipboard;
#[allow(unused_imports)]
use crossterm::{
    event::{
        self as ct_event, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
        MouseEventKind,
    },
    execute,
    style::Print,
    terminal::*,
};
use event::{AppEvent, Command};
#[allow(unused_imports)]
use iroh::EndpointId;
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::mpsc;
use ui::{
    App, ContextMenuAction, ContextMenuTarget, FlockView, MENU_ITEMS, Popup, RoostView,
    ScrollPanel, Selection, SettingsTab,
};

const MOUSE_TRACKING_ON: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h";
const MOUSE_TRACKING_OFF: &str = "\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l";

struct TerminalCleanup {
    mouse: bool,
}

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = std::io::stdout();
        if self.mouse {
            let _ = execute!(
                stdout,
                Print(MOUSE_TRACKING_OFF),
                LeaveAlternateScreen,
                ct_event::DisableMouseCapture,
                ct_event::DisableBracketedPaste
            );
        } else {
            let _ = execute!(
                stdout,
                LeaveAlternateScreen,
                ct_event::DisableBracketedPaste
            );
        }
    }
}

fn apply_profile(app: &mut App, profile: &starling::config::Profile) {
    use starling::config::{DEFAULT_ACCENT_COLOR, DEFAULT_DIM_COLOR};

    app.name.clone_from(&profile.name);
    app.pronouns.clone_from(&profile.pronouns);

    let mut palette = ui::Palette::default();
    if let Some(color) = ui::hex_to_color(&profile.text_color) {
        palette.text = color;
    }
    if let Some(color) = ui::hex_to_color(&profile.border_color) {
        palette.border = color;
    }
    palette.background = if profile.bg_color.is_empty() {
        Some(ui::hex_to_color(starling::config::DEFAULT_BG_COLOR).unwrap())
    } else {
        ui::hex_to_color(&profile.bg_color)
    };
    if let Some(color) = ui::hex_to_color(&profile.accent_color) {
        palette.accent = color;
        if !profile
            .accent_color
            .eq_ignore_ascii_case(DEFAULT_ACCENT_COLOR)
        {
            palette.invite = color;
        }
    }
    if let Some(color) = ui::hex_to_color(&profile.author_color) {
        palette.author = color;
    }
    if let Some(color) = ui::hex_to_color(&profile.selection_color) {
        palette.selection = color;
    }
    if let Some(color) = ui::hex_to_color(&profile.dim_color) {
        palette.dim = color;
        if !profile.dim_color.eq_ignore_ascii_case(DEFAULT_DIM_COLOR) {
            palette.channel = color;
        }
    }
    app.palette = palette;
    app.accent_input = profile.accent_color.clone();
    if let Some(accent) = ui::hex_to_color(&profile.accent_color) {
        app.palette.hover = ui::shade_color(accent, 20);
        app.palette.active = ui::shade_color(accent, 10);
        app.palette.focus_ring = ui::shade_color(accent, 55);
    }
    app.profile_panel.banner.clone_from(&profile.banner);
    app.profile_panel
        .avatar_label
        .clone_from(&profile.avatar_label);
    app.profile_panel.about_me.clone_from(&profile.about_me);
    app.profile_panel.pronouns.clone_from(&profile.pronouns);
    app.profile_panel.motd.clone_from(&profile.motd);
    app.profile_panel
        .custom_status
        .clone_from(&profile.custom_status);
}

fn open_profile(app: &mut App) {
    let p = &mut app.profile_panel;
    p.open = true;
    p.editing = false;
    p.draft_name.clone_from(&app.name);
    p.draft_avatar_label.clone_from(&p.avatar_label);
    p.draft_avatar_path.clone_from(&p.avatar_path);
    p.draft_banner.clone_from(&p.banner);
    p.draft_banner_path.clone_from(&p.banner_path);
    p.draft_about_me.clone_from(&p.about_me);
    p.draft_pronouns.clone_from(&p.pronouns);
    p.draft_motd.clone_from(&p.motd);
    p.draft_custom_status.clone_from(&p.custom_status);
}

fn save_profile(app: &mut App, profile: &mut starling::config::Profile) -> anyhow::Result<()> {
    let p = &mut app.profile_panel;
    app.name = p.draft_name.trim().to_string();
    app.pronouns = p.draft_pronouns.trim().to_string();
    p.banner = p.draft_banner.trim().to_string();
    p.banner_path = p.draft_banner_path.trim().to_string();
    p.avatar_label = p.draft_avatar_label.trim().to_string();
    p.avatar_path = p.draft_avatar_path.trim().to_string();
    p.about_me = p.draft_about_me.trim().to_string();
    p.pronouns = app.pronouns.clone();
    p.motd = p.draft_motd.trim().to_string();
    p.custom_status = p.draft_custom_status.trim().to_string();
    profile.name = app.name.clone();
    profile.pronouns = app.pronouns.clone();
    profile.banner = p.banner.clone();
    profile.avatar_label = p.avatar_label.clone();
    profile.about_me = p.about_me.clone();
    profile.motd = p.motd.clone();
    profile.custom_status = p.custom_status.clone();
    profile.save()?;
    p.editing = false;
    Ok(())
}

fn handle_profile_key(
    app: &mut App,
    profile: &mut starling::config::Profile,
    key: &KeyEvent,
) -> anyhow::Result<KeyOutcome> {
    match key.code {
        KeyCode::Tab if app.profile_panel.editing => {
            app.profile_panel.field = match app.profile_panel.field {
                ui::ProfileField::Name => ui::ProfileField::Avatar,
                ui::ProfileField::Avatar => ui::ProfileField::Banner,
                ui::ProfileField::Banner => ui::ProfileField::AboutMe,
                ui::ProfileField::AboutMe => ui::ProfileField::Pronouns,
                ui::ProfileField::Pronouns => ui::ProfileField::Motd,
                ui::ProfileField::Motd => ui::ProfileField::CustomStatus,
                ui::ProfileField::CustomStatus => ui::ProfileField::Name,
            };
        }
        KeyCode::Enter if app.profile_panel.editing => save_profile(app, profile)?,
        KeyCode::Esc => {
            app.profile_panel.open = false;
            app.profile_panel.editing = false;
        }
        KeyCode::Backspace if app.profile_panel.editing => {
            profile_field_mut(&mut app.profile_panel).pop();
        }
        KeyCode::Char(c) if app.profile_panel.editing => {
            profile_field_mut(&mut app.profile_panel).push(c);
        }
        _ => {}
    }
    Ok(KeyOutcome::Handled)
}

fn profile_field_mut(p: &mut ui::LocalProfilePanel) -> &mut String {
    match p.field {
        ui::ProfileField::Name => &mut p.draft_name,
        ui::ProfileField::Avatar => &mut p.draft_avatar_path,
        ui::ProfileField::Banner => &mut p.draft_banner_path,
        ui::ProfileField::AboutMe => &mut p.draft_about_me,
        ui::ProfileField::Pronouns => &mut p.draft_pronouns,
        ui::ProfileField::Motd => &mut p.draft_motd,
        ui::ProfileField::CustomStatus => &mut p.draft_custom_status,
    }
}

fn handle_settings_key(
    app: &mut App,
    profile: &mut starling::config::Profile,
    key: &KeyEvent,
) -> anyhow::Result<()> {
    match key.code {
        KeyCode::Left => {
            let tabs = [
                SettingsTab::Account,
                ui::SettingsTab::Voice,
                SettingsTab::Appearance,
                ui::SettingsTab::Notifications,
                ui::SettingsTab::Keybinds,
            ];
            let pos = tabs
                .iter()
                .position(|t| *t == app.settings_tab)
                .unwrap_or(0);
            app.settings_tab = tabs[pos.saturating_sub(1)];
        }
        KeyCode::Right => {
            let tabs = [
                SettingsTab::Account,
                ui::SettingsTab::Voice,
                SettingsTab::Appearance,
                ui::SettingsTab::Notifications,
                ui::SettingsTab::Keybinds,
            ];
            let pos = tabs
                .iter()
                .position(|t| *t == app.settings_tab)
                .unwrap_or(0);
            app.settings_tab = tabs[(pos + 1).min(tabs.len() - 1)];
        }
        KeyCode::Enter if app.settings_tab == SettingsTab::Appearance => {
            let value = app.accent_input.clone();
            if ui::apply_accent_color(app, &value) {
                profile.accent_color = value;
                profile.save()?;
            } else {
                app.error_message = Some("Use #RRGGBB".into());
            }
        }
        KeyCode::Tab if app.settings_tab == SettingsTab::Appearance => {
            app.icon_style = app.icon_style.next();
        }
        KeyCode::Backspace if app.settings_tab == SettingsTab::Appearance => {
            app.accent_input.pop();
        }
        KeyCode::Char(c) if app.settings_tab == SettingsTab::Appearance => {
            if "#0123456789abcdefABCDEF".contains(c) && app.accent_input.len() < 7 {
                app.accent_input.push(c);
            }
        }
        KeyCode::Esc => app.settings_open = false,
        _ => {}
    }
    Ok(())
}

fn refresh_create_flock_code(app: &mut App) {
    app.create_flock_code = match (
        app.node_id,
        app.create_flock_secret,
        app.create_flock_name.trim(),
    ) {
        (Some(opener), Some(secret), name) if !name.is_empty() => {
            Some(starling::net::encode_flock_code(&secret, &opener, name))
        }
        _ => None,
    };
}

fn open_create_room(app: &mut App) {
    app.create_flock_secret = Some(iroh::SecretKey::generate().to_bytes());
    app.create_flock_name.clear();
    refresh_create_flock_code(app);
    app.show_create_room = true;
}

fn open_edit_flock(app: &mut App) -> bool {
    let Some(code) = app.active_code().map(str::to_owned) else {
        app.error_message = Some("Select a flock or roost to edit".into());
        return false;
    };
    app.edit_flock_code = code;
    app.edit_flock_name = app
        .flocks
        .iter()
        .find(|f| f.code == app.edit_flock_code)
        .map(|f| f.name.clone())
        .or_else(|| {
            app.roosts
                .iter()
                .find(|r| r.code == app.edit_flock_code)
                .map(|r| r.name.clone())
        })
        .unwrap_or_default();
    app.show_edit_flock = true;
    true
}

fn merge_history(app: &mut App, flock: &str, old: Vec<starling::event::ChatMessage>) {
    let view = app
        .flocks
        .iter_mut()
        .find(|view| view.code == flock)
        .or_else(|| {
            app.roosts
                .iter_mut()
                .flat_map(|roost| roost.channels.iter_mut())
                .find(|view| view.code == flock)
        });
    if let Some(view) = view {
        let known: std::collections::HashSet<_> = view
            .messages
            .iter()
            .map(|message| message.msg.id.clone())
            .collect();
        let mut fresh: Vec<_> = old
            .into_iter()
            .filter(|message| !known.contains(&message.id))
            .map(|message| ui::MessageView {
                msg: message,
                private: false,
            })
            .collect();
        fresh.extend(std::mem::take(&mut view.messages));
        fresh.sort_by_key(|message| message.msg.ts);
        view.messages = fresh;
    }
}

fn nav_items(app: &App) -> Vec<Selection> {
    let mut nav = Vec::new();
    for i in 0..app.flocks.len() {
        // Skip legacy flocks that already have a typed context so
        // keyboard navigation matches what the sidebar renders.
        let Some(flock) = app.flocks.get(i) else {
            continue;
        };
        if app
            .contexts
            .values()
            .any(|ctx| ctx.secret.as_deref() == Some(flock.code.as_str()))
        {
            continue;
        }
        nav.push(Selection::Flock(i));
    }
    for (ri, rv) in app.roosts.iter().enumerate() {
        if app.expanded.contains(&ri) {
            for ci in 0..rv.channels.len() {
                nav.push(Selection::Channel(ri, ci));
            }
        }
    }
    nav
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    starling::logger::init()?;

    let args: Vec<String> = std::env::args().collect();
    let first = args.get(1).map(String::as_str);

    if first == Some("--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if first == Some("leave") {
        return run_leave(&args);
    }

    if matches!(first, Some("profile" | "settings")) {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            ct_event::EnableMouseCapture,
            ct_event::EnableBracketedPaste,
            Print(MOUSE_TRACKING_ON)
        )?;
        let _cleanup = TerminalCleanup { mouse: true };
        let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
        if first == Some("profile") {
            setup::run_profile(&mut term)?;
        } else {
            setup::run_settings(&mut term)?;
        }
        return Ok(());
    }

    let bootstrap = match parse_join_arg(&args) {
        Ok(Some(code)) => Some(code),
        Ok(None) => None,
        Err(usage) => {
            eprintln!("{usage}");
            return Ok(());
        }
    };

    let profile = starling::config::Profile::load();

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        ct_event::EnableMouseCapture,
        ct_event::EnableBracketedPaste,
        Print(MOUSE_TRACKING_ON)
    )?;
    let _cleanup = TerminalCleanup { mouse: true };
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
    let mut app = App::default();
    let secret = starling::config::Profile::load_or_create_secret();
    let my_node_id: iroh::EndpointId = secret.public();
    let config_dir = starling::config::Profile::config_dir();
    let history_path = at_rest::history_dir(&config_dir);
    let identity_secret = secret.to_bytes();
    let state_path = config_dir.join("public").join("contexts.bin");
    let protected_path = config_dir.join("protected").join("credentials.bin");
    if let Err(error) = persistence::recover(&state_path) {
        app.error_message = Some(format!("Could not recover saved contexts: {error}"));
    } else if state_path.exists() {
        match persistence::load_public(&state_path) {
            Ok(saved) => {
                // Seed flock/roost views from every saved descriptor, including
                // those with join secrets, so spaces stay visible even while
                // everyone is offline. The auto-rejoin refreshes them when
                // peers come back; it never removes them.
                // Roost descriptors carry labels of the form "{code}/{channel}";
                // group them per code so the roost gets a real name and its
                // channels instead of a raw routing key.
                let mut roost_channels: std::collections::HashMap<String, Vec<String>> =
                    std::collections::HashMap::new();
                let mut roost_names: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                for descriptor in &saved.contexts {
                    let Some(secret) = descriptor.secret.clone() else {
                        app.insert_context(ui::ContextView {
                            id: descriptor.space,
                            title: descriptor.label.clone(),
                            roost: match descriptor.space {
                                starling::protocol::SpaceId::RoostChannel { roost, .. } => {
                                    Some(roost)
                                }
                                starling::protocol::SpaceId::Flock(_) => None,
                            },
                            base_invite_display: None,
                            messages: Vec::new(),
                            unread: 0,
                            state: ui::ContextState::Restoring,
                            secret: None,
                        });
                        continue;
                    };
                    // Space with a join secret: re-create the view immediately
                    // so it survives offline restarts.
                    app.insert_context(ui::ContextView {
                        id: descriptor.space,
                        title: descriptor.label.clone(),
                        roost: match descriptor.space {
                            starling::protocol::SpaceId::RoostChannel { roost, .. } => Some(roost),
                            starling::protocol::SpaceId::Flock(_) => None,
                        },
                        base_invite_display: Some(secret.clone()),
                        messages: Vec::new(),
                        unread: 0,
                        state: ui::ContextState::Restoring,
                        secret: Some(secret.clone()),
                    });
                    match descriptor.space {
                        starling::protocol::SpaceId::Flock(_) => {
                            if !app.flocks.iter().any(|f| f.code == secret) {
                                let mut fv = ui::FlockView {
                                    code: secret.clone(),
                                    name: descriptor.label.clone(),
                                    messages: Vec::new(),
                                    unread: 0,
                                };
                                if let Err(e) = at_rest::load_into_view(
                                    &history_path,
                                    &identity_secret,
                                    &secret,
                                    &mut fv,
                                ) {
                                    starling::logger::warn(&format!(
                                        "could not load saved history for {}: {e}",
                                        descriptor.label
                                    ));
                                }
                                app.flocks.push(fv);
                            }
                        }
                        starling::protocol::SpaceId::RoostChannel { .. } => {
                            // "{code}/{channel}" -> roost name is the part
                            // before the slash; the channel is the suffix.
                            let (roost_name, channel) = match descriptor.label.split_once('/') {
                                Some((rn, ch)) => (rn.to_string(), ch.to_string()),
                                None => (descriptor.label.clone(), "general".to_string()),
                            };
                            roost_names.entry(secret.clone()).or_insert(roost_name);
                            roost_channels
                                .entry(secret.clone())
                                .or_default()
                                .push(channel);
                        }
                    }
                }
                for (code, channels) in roost_channels {
                    if app.roosts.iter().any(|r| r.code == code) {
                        continue;
                    }
                    let name = roost_names.remove(&code).unwrap_or_else(|| code.clone());
                    let channels = channels
                        .into_iter()
                        .map(|channel| {
                            let mut fv = ui::FlockView {
                                code: format!("{code}/{channel}"),
                                name: channel,
                                messages: Vec::new(),
                                unread: 0,
                            };
                            let channel_code = fv.code.clone();
                            if let Err(e) = at_rest::load_into_view(
                                &history_path,
                                &identity_secret,
                                &channel_code,
                                &mut fv,
                            ) {
                                starling::logger::warn(&format!(
                                    "could not load saved history for {}/{}: {e}",
                                    code, fv.name
                                ));
                            }
                            fv
                        })
                        .collect();
                    app.roosts.push(ui::RoostView {
                        code: code.clone(),
                        name,
                        channels,
                        unread: 0,
                        icon_path: None,
                    });
                }
                if let Some(active) = saved.active_space {
                    app.select_context(active);
                }
            }
            Err(error) => {
                app.error_message = Some(format!("Could not load saved contexts: {error}"));
            }
        }
    }
    let mut protected_state = persistence::ProtectedSecretState::default();
    if protected_path.exists() {
        match persistence::load_protected(&protected_path) {
            Ok(loaded) => protected_state = loaded,
            Err(error) => {
                app.error_message = Some(format!("Could not load saved credentials: {error}"));
            }
        }
    }
    let mut clipboard = clipboard::SystemClipboard::new().ok();

    let profile = match profile {
        Some(profile) => profile,
        None => match setup::run_setup(&mut term)? {
            Some(profile) => profile,
            None => return Ok(()),
        },
    };

    let mut profile = profile;

    let name = profile.name.clone();
    let pronouns = profile.pronouns.clone();
    #[cfg(feature = "audio")]
    let input_device = profile.input_device.clone();
    #[cfg(feature = "audio")]
    let output_device = profile.output_device.clone();
    #[allow(unused)]
    let camera_index = profile.camera_index;
    apply_profile(&mut app, &profile);

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Command>();
    let (evt_tx, mut evt_rx) = mpsc::unbounded_channel::<AppEvent>();
    #[allow(unused)]
    let muted_flag = Arc::new(AtomicBool::new(false));

    let restored_contexts = app.context_order.clone();
    let mut restore_codes: std::collections::HashMap<starling::protocol::SpaceId, String> =
        std::collections::HashMap::new();
    // Build restore codes from the saved descriptors so that contexts
    // which were skipped during restore (because they carry secrets) are
    // still re-joined on startup.
    if let Ok(saved) = persistence::load_public(&state_path) {
        for descriptor in &saved.contexts {
            if let Some(ref code) = descriptor.secret {
                restore_codes.insert(descriptor.space, code.clone());
            }
        }
    }
    for space in &restored_contexts {
        if let Some(context) = app.contexts.get(space)
            && let Some(ref code) = context.secret
        {
            restore_codes.insert(*space, code.clone());
        }
    }
    let mut net_task = tokio::spawn(net::run(
        bootstrap,
        restore_codes,
        cmd_rx,
        evt_tx,
        muted_flag.clone(),
        my_node_id,
        name,
        input_device,
        camera_index,
        pronouns,
    ));
    if !restored_contexts.is_empty() {
        let _ = cmd_tx.send(Command::RestoreContexts(restored_contexts));
    }

    #[cfg(feature = "audio")]
    let mut playback = match crate::playback::Playback::new(output_device.as_deref()) {
        Ok(p) => Some(p),
        Err(e) => {
            starling::logger::warn(&format!("audio playback unavailable: {e}"));
            app.error_message = Some(format!("Audio output unavailable: {e}"));
            None
        }
    };

    let mut last_frame = Instant::now();
    let mut quit_sent = false;
    loop {
        tokio::select! {
                result = &mut net_task => {
                    match result {
                        Ok(Ok(())) if app.quit_requested => break,
                        Ok(Ok(())) => anyhow::bail!("network task stopped unexpectedly"),
                        Ok(Err(error)) => return Err(error.context("network task failed")),
                        Err(error) => {
                            return Err(anyhow::Error::new(error).context("network task panicked"));
                        }
                    }
                }
                _ = std::future::ready(()) => {
            let now = Instant::now();
            let dt = now.duration_since(last_frame).as_secs_f32().min(0.1);
            last_frame = now;
            let (term_w, term_h) = crossterm::terminal::size()?;
            app.note_terminal_size(term_w);
            let (_, flock_h, _, roost_h, bird_h) = panel_geometry(term_h);
            app.update_scroll_bounds(
                flock_h.saturating_sub(2) as usize,
                roost_h.saturating_sub(2) as usize,
                bird_h.saturating_sub(2) as usize,
            );
            app.advance_scroll(dt);
            app.expire_status_notice(Instant::now());
            for ctx in app.presence.contexts.values_mut() {
                ctx.expire(tokio::time::Instant::now());
            }
            term.draw(|f| ui::draw(f, &app))?;

            while let Ok(ev) = evt_rx.try_recv() {
                match ev {
                    AppEvent::ContextStateChanged { space, state } => {
                        if let Some(context) = app.contexts.get_mut(&space) {
                            context.state = state;
                        }
                    }
                    AppEvent::Message {
                        flock,
                        msg,
                        private,
                    } => {
                        app.receive_message(&flock, msg, private);
                    }
                    AppEvent::DmKey { endpoint, dm_pk } => {
                        // Only Phase 9 profile announcements ever produce this
                        // event, and the receive loop already authenticated the
                        // announcement's envelope, so `endpoint` is a verified
                        // author, not a claimed id. Cache for `/chirp`.
                        if app.peer_dm_keys.get(&endpoint) != Some(&dm_pk) {
                            app.peer_dm_keys.insert(endpoint, dm_pk);
                        }
                    }
                    AppEvent::JoinedFlock { code, name } => {
                        app.joining = None;
                        if app.flocks.iter().any(|flock| flock.code == code) {
                            continue;
                        }
                        // Also create a ContextView for persistence and V1 tracking.
                        if let Some(typed) = starling::net::decode_typed_code(&code)
                            && let Some(flock_code) = starling::net::decode_flock_code(&typed)
                        {
                            let space_id = starling::protocol::SpaceId::Flock(
                                starling::protocol::FlockId(flock_code.secret),
                            );
                            // Update the existing context when an auto-rejoin
                            // fires for a flock whose context was pre-loaded
                            // from persistence (e.g. legacy contexts without
                            // secrets that were inserted with Restoring state).
                            if let Some(existing) = app.contexts.get_mut(&space_id) {
                                existing.state = ui::ContextState::Ready;
                                existing.title = flock_code.name.clone();
                                existing.secret = Some(code.clone());
                                existing.base_invite_display = Some(code.clone());
                            } else {
                                app.insert_context(ui::ContextView {
                                    id: space_id,
                                    title: flock_code.name.clone(),
                                    roost: None,
                                    base_invite_display: Some(code.clone()),
                                    messages: Vec::new(),
                                    unread: 0,
                                    state: ui::ContextState::Ready,
                                    secret: Some(code.clone()),
                                });
                            }
                        }
                        app.flocks.push(FlockView {
                            code,
                            name,
                            messages: vec![],
                            unread: 0,
                        });
                    }
                    AppEvent::JoinedRoost {
                        code,
                        name,
                        channels,
                        perms,
                    } => {
                        app.joining = None;
                        app.apply_roost_perms(&code, &perms);
                        let roost_channels: Vec<(String, String)> = channels
                            .into_iter()
                            .map(|channel| {
                                let channel_code = format!("{code}/{channel}");
                                (channel, channel_code)
                            })
                            .collect();
                        // Update a seeded (offline) roost in place so its real
                        // name and channels replace the placeholder, keeping any
                        // messages restored from the encrypted at-rest history.
                        let build_channels = |existing: &[FlockView]| {
                            roost_channels
                                .iter()
                                .map(|(name, code)| FlockView {
                                    code: code.clone(),
                                    name: name.clone(),
                                    messages: existing
                                        .iter()
                                        .find(|c| c.code == *code)
                                        .map(|c| c.messages.clone())
                                        .unwrap_or_default(),
                                    unread: 0,
                                })
                                .collect()
                        };
                        if let Some(rv) = app.roosts.iter_mut().find(|r| r.code == code) {
                            let existing = std::mem::take(&mut rv.channels);
                            rv.name = name;
                            rv.channels = build_channels(&existing);
                            rv.unread = 0;
                        } else {
                            app.roosts.push(RoostView {
                                code: code.clone(),
                                name,
                                channels: build_channels(&[]),
                                unread: 0,
                                icon_path: None,
                            });
                        }
                        // Create ContextViews for each roost channel (V1 tracking).
                        if let Some(typed) = starling::net::decode_typed_code(&code)
                            && let Some(node_id) = starling::net::typed_code_node_id(&typed)
                        {
                            let roost_id = starling::protocol::RoostId(*node_id.as_bytes());
                            for (channel_name, _) in &roost_channels {
                                let space_id =
                                    starling::protocol::SpaceId::RoostChannel {
                                        roost: roost_id,
                                        channel: crate::net::channel_id_from_name(
                                            channel_name,
                                        ),
                                    };
                                app.insert_context(ui::ContextView {
                                    id: space_id,
                                    title: format!("{code}/{channel_name}"),
                                    roost: Some(roost_id),
                                    base_invite_display: Some(code.clone()),
                                    messages: Vec::new(),
                                    unread: 0,
                                    state: ui::ContextState::Ready,
                                    secret: Some(code.clone()),
                                });
                            }
                        }
                    }
                    AppEvent::RoostUpdate {
                        code,
                        name,
                        channels,
                        perms,
                    } => {
                        app.apply_roost_perms(&code, &perms);
                        if let Some(rv) = app.roosts.iter_mut().find(|r| r.code == code) {
                            rv.name = name;
                            let mut previous: std::collections::HashMap<_, _> = rv
                                .channels
                                .drain(..)
                                .map(|channel| (channel.name.clone(), channel))
                                .collect();
                            rv.channels = channels
                                .into_iter()
                                .map(|channel| {
                                    previous.remove(&channel).unwrap_or_else(|| FlockView {
                                        code: format!("{code}/{channel}"),
                                        name: channel,
                                        messages: vec![],
                                        unread: 0,
                                    })
                                })
                                .collect();
                            rv.unread = rv.channels.iter().map(|channel| channel.unread).sum();
                        }
                    }
                    AppEvent::PeerConnected { space, id } => {
                        let ctx = app.presence.context_mut(space);
                        ctx.members
                            .entry(id)
                            .or_insert_with(|| ui::MemberProfile {
                                endpoint: id,
                                name: String::new(),
                                pronouns: String::new(),
                            });
                        if !ctx.ordered_ids.contains(&id) {
                            ctx.ordered_ids.push(id);
                        }
                    }
                    AppEvent::PeerConnectivityHintDown(id) => {
                        // Signed presence leases, not transport neighbors, determine liveness.
                        app.presence.neighbor_down(id);
                    }

                    AppEvent::PeerNamed { space, id, name, pronouns } => {
                        if id != my_node_id {
                            let ctx = app.presence.context_mut(space);
                            ctx.set_profile(ui::MemberProfile {
                                endpoint: id,
                                name: name.clone(),
                                pronouns: pronouns.clone(),
                            });
                            if !ctx.ordered_ids.contains(&id) {
                                ctx.ordered_ids.push(id);
                            }
                        }
                        app.peer_names.insert(id, name);
                    }
                    AppEvent::PeerStatus(id, status) => {
                        app.peer_status.insert(id, status);
                    }
                    AppEvent::PresenceLease(lease) => {
                        let remaining_ms =
                            (lease.body.expiry_unix_ms - chrono::Utc::now().timestamp_millis()).max(0);
                        let remaining = std::time::Duration::from_millis(remaining_ms as u64);
                        let live_lease = ui::LiveLease {
                            deadline: starling::presence::lease_deadline(remaining),
                            sequence: lease.body.sequence,
                        };
                        let now = tokio::time::Instant::now();
                        app.presence
                            .context_mut(lease.body.space)
                            .apply_verified_lease(lease.body.endpoint, live_lease, now);
                    }
                    AppEvent::Ticket(node_id) => {
                        app.node_id = Some(node_id);
                    }
                    AppEvent::Error(error) => {
                        starling::logger::warn(&error);
                        app.error_message = Some(error);
                        app.joining = None;
                    }
                    #[cfg(feature = "audio")]
                    AppEvent::VoiceFrame(bytes) => {
                        if let Some(p) = &mut playback {
                            p.push_opus(&bytes);
                        }
                    }
                    #[cfg(feature = "audio")]
                    AppEvent::CallStarted(peer) => {
                        if !app.peers.contains(&peer) {
                            app.peers.push(peer);
                        }
                        app.in_call = true;
                        app.error_message = None;
                    }
                    #[cfg(feature = "audio")]
                    AppEvent::CallEnded(_peer) => {
                        app.in_call = false;
                    }
                    #[cfg(feature = "video")]
                    AppEvent::LocalVideoFrame(jpeg) => {
                        if let Ok(img) = image::load_from_memory(&jpeg) {
                            app.local_video_frame = Some(img.to_rgb8());
                            app.error_message = None;
                        }
                    }
                    #[cfg(feature = "video")]
                    AppEvent::LocalVideoFailed(error) => {
                        app.show_video = false;
                        app.local_video_frame = None;
                        app.error_message = Some(error);
                    }
                    #[cfg(feature = "video")]
                    AppEvent::RemoteVideoFrame { peer, jpeg } => {
                        if let Ok(img) = image::load_from_memory(&jpeg) {
                            app.remote_video_frames.insert(peer, img.to_rgb8());
                        }
                    }
                    #[cfg(feature = "video")]
                    AppEvent::RemoteVideoStopped(peer) => {
                        app.remote_video_frames.remove(&peer);
                    }
                    AppEvent::HistoryChunk { flock, messages } => {
                        merge_history(&mut app, &flock, messages);
                    }
                    AppEvent::Notice(text) => app.show_status_notice(text, Instant::now()),
                }
            }

            if ct_event::poll(std::time::Duration::from_millis(50))? {
                let event = ct_event::read()?;

                if let Event::Paste(raw) = &event {
                    if app.profile_panel.open && app.profile_panel.editing {
                        profile_field_mut(&mut app.profile_panel).push_str(raw);
                        continue;
                    } else if app.settings_open {
                        app.accent_input.push_str(raw);
                        continue;
                    }
                    if app.show_create_room {
                        app.create_flock_name =
                            sanitize::sanitize_name(&format!("{}{}", app.create_flock_name, raw));
                        refresh_create_flock_code(&mut app);
                    } else if app.show_edit_flock {
                        app.edit_flock_name =
                            sanitize::sanitize_name(&format!("{}{}", app.edit_flock_name, raw));
                    } else if app.show_join_room {
                        let combined = format!("{}{}", app.join_input, raw);
                        if let Some(code) = sanitize::sanitize_code(&combined) {
                            app.join_input = code;
                        }
                        app.error_message = None;
                    } else {
                        app.input = sanitize::sanitize_message(&format!("{}{}", app.input, raw));
                    }
                    continue;
                }

                if let Event::Key(k) = &event {
                    if k.kind != KeyEventKind::Press {
                        continue;
                    }
                    if matches!(k.code, KeyCode::Char('c' | 'C'))
                        && k.modifiers.contains(KeyModifiers::CONTROL)
                        && k.modifiers.contains(KeyModifiers::SHIFT)
                    {
                        copy_active_invite(&mut app, clipboard.as_mut(), Instant::now());
                        continue;
                    }
                    if matches!(k.code, KeyCode::Char('m' | 'M'))
                        && k.modifiers.contains(KeyModifiers::CONTROL)
                        && k.modifiers.contains(KeyModifiers::SHIFT)
                    {
                        app.muted = !app.muted;
                        muted_flag.store(app.muted, Ordering::Relaxed);
                        continue;
                    }
                    if matches!(k.code, KeyCode::Char('d' | 'D'))
                        && k.modifiers.contains(KeyModifiers::CONTROL)
                        && k.modifiers.contains(KeyModifiers::SHIFT)
                    {
                        app.deafened = !app.deafened;
                        app.muted = app.deafened;
                        muted_flag.store(app.muted, Ordering::Relaxed);
                        continue;
                    }

                    if app.profile_panel.open {
                        handle_profile_key(&mut app, &mut profile, k)?;
                        continue;
                    }
                    if app.settings_open {
                        handle_settings_key(&mut app, &mut profile, k)?;
                        continue;
                    }

                    let outcome = match app.active_popup() {
                        Popup::DeleteConfirm =>
                            Ok(handle_delete_confirm_key(&mut app, k)),
                        Popup::AddChannel =>
                            Ok(handle_add_channel_key(&mut app, k, &cmd_tx)),
                        Popup::CreateRoost =>
                            Ok(handle_create_roost_key(&mut app, k, &cmd_tx)),
                        Popup::CreateRoom =>
                            Ok(handle_create_room_key(&mut app, k, &cmd_tx)),
                        Popup::EditFlock =>
                            Ok(handle_edit_flock_key(&mut app, k, &cmd_tx)),
                        Popup::JoinRoom =>
                            Ok(handle_join_room_key(&mut app, k, &cmd_tx)),
                        Popup::Menu =>
                            handle_menu_key(&mut app, k, &cmd_tx, &mut term),
                        Popup::BirdProfile =>
                            Ok(handle_bird_profile_key(&mut app, k, &cmd_tx)),
                        Popup::ContextMenu => {
                            handle_context_menu_key(&mut app, k, &cmd_tx)?;
                            Ok(KeyOutcome::Handled)
                        }
                        Popup::RoleSubmenu => {
                            handle_role_submenu_key(&mut app, k, &cmd_tx)?;
                            Ok(KeyOutcome::Handled)
                        }
                        Popup::None =>
                            handle_normal_key(&mut app, k, &cmd_tx),
                    }?;
                    if matches!(outcome, KeyOutcome::Handled) {
                        continue;
                    }
                } else if let Event::Mouse(m) = event {
                    if app.show_menu {
                        let (term_w, term_h) = crossterm::terminal::size()?;
                        update_menu_hover(&mut app, term_w, term_h, m.column, m.row);
                    }
                    match m.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            // Dragging a popup by its title row moves it;
                            // everything else falls through to the click handler.
                            let (term_w, term_h) = crossterm::terminal::size()?;
                            if app.popup_title_row(term_w, term_h, m.column, m.row) {
                                if let Some(popup) = ui::active_popup_rect(&app, term_w, term_h) {
                                    app.drag_grab = Some((
                                        m.column as i16 - popup.x as i16,
                                        m.row as i16 - popup.y as i16,
                                    ));
                                }
                            } else {
                                handle_mouse_click(
                                    &mut app,
                                    &cmd_tx,
                                    &muted_flag,
                                    clipboard.as_mut(),
                                    &mut term,
                                    m.column,
                                    m.row,
                                )?;
                            }
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            if let Some((grab_x, grab_y)) = app.drag_grab {
                                let (term_w, term_h) = crossterm::terminal::size()?;
                                if let Some(popup) = ui::active_popup_rect(&app, term_w, term_h) {
                                    let centered_x = (term_w.saturating_sub(popup.width)) / 2;
                                    let centered_y = (term_h.saturating_sub(popup.height)) / 2;
                                    app.popup_offset = (
                                        m.column as i16 - grab_x - centered_x as i16,
                                        m.row as i16 - grab_y - centered_y as i16,
                                    );
                                }
                            }
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            app.drag_grab = None;
                        }
                        MouseEventKind::Moved => {}
                        MouseEventKind::ScrollUp if !app.show_menu => {
                            handle_mouse_scroll(&mut app, m.column, m.row, -3.0)?;
                        }
                        MouseEventKind::ScrollDown if !app.show_menu => {
                            handle_mouse_scroll(&mut app, m.column, m.row, 3.0)?;
                        }
                        MouseEventKind::Down(MouseButton::Right) => {
                            handle_right_click(&mut app, m.column, m.row)?;
                        }
                        MouseEventKind::Up(MouseButton::Right)
                        | MouseEventKind::Drag(MouseButton::Right) => {}
                        _ => {}
                    }
                }
            }

                if app.quit_requested && !quit_sent {
                    let _ = cmd_tx.send(Command::Quit);
                    quit_sent = true;
                }
            }
        }
    }

    if !app.skip_save_on_exit {
        if let Err(error) = save_context_state(&state_path, &app) {
            starling::logger::warn(&format!("could not save contexts: {error}"));
        }
        if let Err(error) =
            at_rest::save_all(&history_path, &identity_secret, &app.flocks, &app.roosts)
        {
            starling::logger::warn(&format!("could not save message history: {error}"));
        }
        if let Err(error) = persistence::save_protected(&protected_path, &protected_state) {
            starling::logger::warn(&format!("could not save credentials: {error}"));
        }
    }
    disable_raw_mode()?;
    execute!(
        term.backend_mut(),
        Print(MOUSE_TRACKING_OFF),
        LeaveAlternateScreen,
        ct_event::DisableMouseCapture,
        ct_event::DisableBracketedPaste
    )?;
    Ok(())
}

/// Outcome of a key handler, mirroring the event loop's original control flow.
///
/// `Handled` reproduces the old `continue;` that each popup guard (and some
/// normal-mode error arms) used to skip the rest of the loop iteration; the
/// caller issues the `continue` on this signal. `Fallthrough` reproduces the
/// normal-mode match that simply ended and let the iteration proceed to the
/// quit-request check.
enum KeyOutcome {
    Handled,
    Fallthrough,
}

fn handle_delete_confirm_key(app: &mut App, k: &KeyEvent) -> KeyOutcome {
    match k.code {
        KeyCode::Enter => confirm_delete(app),
        KeyCode::Char(c) => {
            app.delete_confirm_input.push(c);
        }
        KeyCode::Backspace => {
            app.delete_confirm_input.pop();
        }
        KeyCode::Esc => {
            app.show_delete_confirm = false;
            app.delete_confirm_input.clear();
        }
        _ => {}
    }
    KeyOutcome::Handled
}

/// Confirm the "Delete all data" popup: erase the config directory and quit.
/// Shared by the Enter key and the Delete button.
fn confirm_delete(app: &mut App) {
    if app.delete_confirm_input.trim() == "DELETE" {
        app.show_delete_confirm = false;
        let dir = starling::config::Profile::config_dir();
        if dir.exists() {
            if let Err(error) = std::fs::remove_dir_all(&dir) {
                app.error_message = Some(format!("Failed to delete data: {error}"));
            } else {
                app.skip_save_on_exit = true;
                app.quit_requested = true;
            }
        } else {
            app.skip_save_on_exit = true;
            app.quit_requested = true;
        }
    } else {
        app.error_message = Some("Type DELETE to confirm".into());
    }
}

fn handle_create_room_key(
    app: &mut App,
    k: &KeyEvent,
    cmd_tx: &mpsc::UnboundedSender<Command>,
) -> KeyOutcome {
    match k.code {
        KeyCode::Enter if app.create_flock_code.is_some() => {
            confirm_create_flock(app, cmd_tx);
        }
        KeyCode::Char(c) => {
            app.create_flock_name =
                sanitize::sanitize_name(&format!("{}{}", app.create_flock_name, c));
            refresh_create_flock_code(app);
        }
        KeyCode::Backspace => {
            app.create_flock_name.pop();
            refresh_create_flock_code(app);
        }
        KeyCode::Esc => {
            app.create_flock_code = None;
            app.create_flock_secret = None;
            app.create_flock_name.clear();
            app.show_create_room = false;
        }
        _ => {}
    }
    KeyOutcome::Handled
}

fn handle_create_roost_key(
    app: &mut App,
    k: &KeyEvent,
    cmd_tx: &mpsc::UnboundedSender<Command>,
) -> KeyOutcome {
    match k.code {
        KeyCode::Enter if !app.create_roost_input.is_empty() => {
            confirm_create_roost(app, cmd_tx);
        }
        KeyCode::Char(c) if !c.is_control() => {
            app.create_roost_input.push(c);
        }
        KeyCode::Backspace => {
            app.create_roost_input.pop();
        }
        KeyCode::Esc => {
            app.show_create_roost = false;
        }
        _ => {}
    }
    KeyOutcome::Handled
}

fn handle_add_channel_key(
    app: &mut App,
    k: &KeyEvent,
    cmd_tx: &mpsc::UnboundedSender<Command>,
) -> KeyOutcome {
    match k.code {
        KeyCode::Enter if !app.add_channel_input.is_empty() => {
            confirm_add_channel(app, cmd_tx);
        }
        KeyCode::Char(c) if !c.is_control() => {
            app.add_channel_input.push(c);
        }
        KeyCode::Backspace => {
            app.add_channel_input.pop();
        }
        KeyCode::Esc => {
            app.show_add_channel = false;
        }
        _ => {}
    }
    KeyOutcome::Handled
}

fn handle_edit_flock_key(
    app: &mut App,
    k: &KeyEvent,
    cmd_tx: &mpsc::UnboundedSender<Command>,
) -> KeyOutcome {
    match k.code {
        KeyCode::Enter => {
            confirm_edit_flock(app, cmd_tx);
        }
        KeyCode::Backspace => {
            app.edit_flock_name.pop();
        }
        KeyCode::Char(c) => {
            app.edit_flock_name = sanitize::sanitize_name(&format!("{}{}", app.edit_flock_name, c));
        }
        KeyCode::Delete => {
            let code = std::mem::take(&mut app.edit_flock_code);
            app.show_edit_flock = false;
            app.flocks.retain(|f| f.code != code);
            let _ = cmd_tx.send(Command::Leave { code });
        }
        KeyCode::Esc => {
            app.show_edit_flock = false;
        }
        _ => {}
    }
    KeyOutcome::Handled
}

fn handle_join_room_key(
    app: &mut App,
    k: &KeyEvent,
    cmd_tx: &mpsc::UnboundedSender<Command>,
) -> KeyOutcome {
    match k.code {
        KeyCode::Enter => match sanitize::invite(app.join_input.trim()) {
            Ok(code) => {
                confirm_join(app, cmd_tx, code);
            }
            Err(error) => {
                app.error_message = Some(error.to_string());
            }
        },
        KeyCode::Char(c) => {
            let combined = format!("{}{}", app.join_input, c);
            if let Some(code) = sanitize::sanitize_code(&combined) {
                app.join_input = code;
            }
        }
        KeyCode::Backspace => {
            app.join_input.pop();
        }
        KeyCode::Esc => {
            app.show_join_room = false;
        }
        _ => {}
    }
    KeyOutcome::Handled
}

fn handle_bird_profile_key(
    app: &mut App,
    k: &KeyEvent,
    cmd_tx: &mpsc::UnboundedSender<Command>,
) -> KeyOutcome {
    match k.code {
        KeyCode::Enter => {
            confirm_bird_call(app, cmd_tx);
        }
        KeyCode::Esc => {
            app.show_bird_profile = false;
            app.bird_profile_peer = None;
        }
        _ => {}
    }
    KeyOutcome::Handled
}

/// Confirm the "Create a Flock" popup. Shared by Enter and the Create button.
fn confirm_create_flock(app: &mut App, cmd_tx: &mpsc::UnboundedSender<Command>) {
    if let Some(code) = app.create_flock_code.take() {
        // The local user created this flock, so the management menu
        // shows edit/delete for it even before anyone else joins.
        app.flock_owners.insert(code.clone(), true);
        let since = app.newest_ts(&code).unwrap_or(0);
        let _ = cmd_tx.send(Command::Join { code, since });
    }
    app.create_flock_secret = None;
    app.show_create_room = false;
}

/// Confirm the "Create a Roost" popup. Shared by Enter and the Create button.
fn confirm_create_roost(app: &mut App, cmd_tx: &mpsc::UnboundedSender<Command>) {
    let name = std::mem::take(&mut app.create_roost_input);
    let _ = cmd_tx.send(Command::CreateRoost { name });
    app.show_create_roost = false;
}

/// Confirm the "Add Channel" popup. Shared by Enter and the Create button.
fn confirm_add_channel(app: &mut App, cmd_tx: &mpsc::UnboundedSender<Command>) {
    let channel = std::mem::take(&mut app.add_channel_input);
    if let Some(roost_ep) = app.context_menu_roost_endpoint() {
        let _ = cmd_tx.send(Command::AddChannel {
            roost: roost_ep,
            channel,
        });
    }
    app.show_add_channel = false;
}

/// Confirm the "Edit Flock" popup. Shared by Enter and the Save button.
fn confirm_edit_flock(app: &mut App, cmd_tx: &mpsc::UnboundedSender<Command>) {
    let code = std::mem::take(&mut app.edit_flock_code);
    let name = std::mem::take(&mut app.edit_flock_name);
    app.show_edit_flock = false;
    if !name.is_empty() {
        // Update the legacy FlockView name
        if let Some(fv) = app.flocks.iter_mut().find(|f| f.code == code) {
            fv.name = name.clone();
        }
        // Update the typed ContextView title
        if let Some(ctx) = app
            .contexts
            .values_mut()
            .find(|c| c.secret.as_deref() == Some(&code))
        {
            ctx.title = name.clone();
        }
        // Update roost name if this code belongs to a roost
        if let Some(rv) = app.roosts.iter_mut().find(|r| r.code == code) {
            rv.name = name.clone();
            // Persist the rename on the roost server so it survives
            // restarts and propagates to other members.
            if let Some(roost_ep) = starling::net::decode_typed_code(&rv.code)
                .and_then(|t| starling::net::typed_code_node_id(&t))
            {
                let _ = cmd_tx.send(Command::RenameRoost {
                    roost: roost_ep,
                    name,
                });
            }
        }
    }
}

/// Confirm the "Join" popup. Shared by Enter and the Join button.
fn confirm_join(app: &mut App, cmd_tx: &mpsc::UnboundedSender<Command>, code: String) {
    let since = app.newest_ts(&code).unwrap_or(0);
    let _ = cmd_tx.send(Command::Join {
        code: code.clone(),
        since,
    });
    app.joining = Some(code);
    app.join_input.clear();
    app.show_join_room = false;
    app.error_message = None;
}

/// Start a real call to the given peers and post a chat message announcing it.
fn start_call_with_message(
    app: &mut App,
    cmd_tx: &mpsc::UnboundedSender<Command>,
    targets: Vec<EndpointId>,
) {
    if targets.is_empty() {
        app.error_message = Some("No one is online in this context".into());
        return;
    }
    #[cfg(feature = "audio")]
    {
        let _ = cmd_tx.send(Command::StartCall(targets));
        app.error_message = Some("Connecting call...".into());
        // Announce the call in the active space, like a chat message.
        let flock = app
            .active_send_code()
            .or_else(|| app.active_code().map(str::to_owned));
        if let Some(flock) = flock {
            let msg = starling::event::ChatMessage {
                id: uuid::Uuid::new_v4().to_string(),
                author: app.name.clone(),
                body: "started a call.".into(),
                ts: chrono::Utc::now().timestamp_millis(),
            };
            app.receive_message(&flock, msg, false);
        }
    }
    #[cfg(not(feature = "audio"))]
    {
        let _ = app;
        let _ = cmd_tx;
        app.error_message = Some("Audio support not compiled in".into());
    }
}

/// Confirm the bird-profile call. Shared by Enter and the Call button.
fn confirm_bird_call(app: &mut App, cmd_tx: &mpsc::UnboundedSender<Command>) {
    if let Some(peer) = app.bird_profile_peer {
        start_call_with_message(app, cmd_tx, vec![peer]);
    }
    app.show_bird_profile = false;
    app.bird_profile_peer = None;
}

fn handle_context_menu_key(
    app: &mut App,
    k: &KeyEvent,
    cmd_tx: &mpsc::UnboundedSender<Command>,
) -> anyhow::Result<()> {
    match k.code {
        KeyCode::Up => {
            app.context_menu_selection = app.context_menu_selection.saturating_sub(1);
        }
        KeyCode::Down => {
            let max = app.context_menu_items.len().saturating_sub(1);
            app.context_menu_selection = app
                .context_menu_selection
                .min(max)
                .saturating_add(1)
                .min(max);
        }
        KeyCode::Enter => {
            activate_context_menu_item(app, cmd_tx, app.context_menu_selection)?;
        }
        KeyCode::Esc => {
            app.show_context_menu = false;
        }
        _ => {}
    }
    Ok(())
}

/// Run the context menu action at `index` (mouse or keyboard).
fn activate_context_menu_item(
    app: &mut App,
    cmd_tx: &mpsc::UnboundedSender<Command>,
    index: usize,
) -> anyhow::Result<()> {
    let Some(item) = app.context_menu_items.get(index) else {
        return Ok(());
    };
    if !item.enabled {
        return Ok(());
    }
    match &item.action {
        ContextMenuAction::SetRole => {
            if let Some(ContextMenuTarget::Bird(endpoint)) = &app.context_menu_target {
                app.role_submenu_target = Some(*endpoint);
                app.role_submenu_selection = 0;
                app.show_role_submenu = true;
                app.show_context_menu = false;
            }
        }
        _ => {
            let action = item.action.clone();
            execute_context_action(app, cmd_tx, &action)?;
            app.show_context_menu = false;
        }
    }
    Ok(())
}

/// Map a click to a role submenu item index, or `None` when outside.
fn role_submenu_item_at(app: &App, col: u16, row: u16, term_w: u16, term_h: u16) -> Option<usize> {
    let popup = ui::active_popup_rect(app, term_w, term_h)?;
    let width = popup.width;
    let height = popup.height;
    let popup_x = popup.x;
    let popup_y = popup.y;
    if col < popup_x || col >= popup_x + width {
        return None;
    }
    let inner_row = row.checked_sub(popup_y)?;
    if inner_row == 0 || inner_row >= height.saturating_sub(1) {
        return None;
    }
    let idx = (inner_row - 1) as usize;
    (idx < 2).then_some(idx)
}

/// Run the role submenu action at `index` (mouse or keyboard).
fn activate_role_submenu_item(
    app: &mut App,
    cmd_tx: &mpsc::UnboundedSender<Command>,
    index: usize,
) -> anyhow::Result<()> {
    let target = app.role_submenu_target;
    if let Some(endpoint) = target
        && let Some(roost_ep) = app.context_menu_roost_endpoint()
    {
        let _ = cmd_tx.send(Command::SetRole {
            roost: roost_ep,
            target: endpoint,
            role_index: Some(index),
        });
    }
    app.show_role_submenu = false;
    app.show_context_menu = false;
    Ok(())
}

/// Map a click to a context menu item index, or `None` when outside the menu.
fn context_menu_item_at(app: &App, col: u16, row: u16, term_w: u16, term_h: u16) -> Option<usize> {
    let popup = ui::active_popup_rect(app, term_w, term_h)?;
    let width = popup.width;
    let height = popup.height;
    let popup_x = popup.x;
    let popup_y = popup.y;
    if col < popup_x || col >= popup_x + width {
        return None;
    }
    let inner_row = row.checked_sub(popup_y)?;
    if inner_row == 0 || inner_row >= height.saturating_sub(1) {
        return None;
    }
    let idx = (inner_row - 1) as usize;
    (idx < app.context_menu_items.len()).then_some(idx)
}

fn handle_role_submenu_key(
    app: &mut App,
    k: &KeyEvent,
    cmd_tx: &mpsc::UnboundedSender<Command>,
) -> anyhow::Result<()> {
    match k.code {
        KeyCode::Up => {
            app.role_submenu_selection = app.role_submenu_selection.saturating_sub(1);
        }
        KeyCode::Down => {
            app.role_submenu_selection = app.role_submenu_selection.saturating_add(1);
        }
        KeyCode::Enter => {
            activate_role_submenu_item(app, cmd_tx, app.role_submenu_selection)?;
        }
        KeyCode::Esc => {
            app.show_role_submenu = false;
            app.show_context_menu = true;
        }
        _ => {}
    }
    Ok(())
}

fn execute_context_action(
    app: &mut App,
    cmd_tx: &mpsc::UnboundedSender<Command>,
    action: &ContextMenuAction,
) -> anyhow::Result<()> {
    // Space management actions apply to flocks and roosts alike and never need
    // a roost endpoint, so handle them before the endpoint guard.
    match action {
        ContextMenuAction::LeaveSpace => {
            let Some(code) = app.context_menu_code() else {
                return Ok(());
            };
            app.show_context_menu = false;
            let is_roost = app.roosts.iter().any(|r| r.code == code);
            if is_roost {
                app.roosts.retain(|r| r.code != code);
            } else {
                app.flocks.retain(|f| f.code != code);
            }
            app.contexts
                .retain(|_, c| c.base_invite_display.as_deref() != Some(code.as_str()));
            app.context_order.retain(|id| app.contexts.contains_key(id));
            app.presence
                .contexts
                .retain(|id, _| app.contexts.contains_key(id));
            app.active = app.context_order.first().copied();
            let _ = cmd_tx.send(Command::Leave { code: code.clone() });
            let _ = at_rest::remove(
                &at_rest::history_dir(&starling::config::Profile::config_dir()),
                &code,
            );
            return Ok(());
        }
        ContextMenuAction::DeleteSpace => {
            let Some(code) = app.context_menu_code() else {
                return Ok(());
            };
            app.show_context_menu = false;
            let is_local_roost = app.roosts.iter().any(|r| r.code == code);
            if is_local_roost {
                // Destroy the local roost server data if we host it.
                let _ = starling::roost::server::destroy_by_code(&code);
            }
            app.roosts.retain(|r| r.code != code);
            app.flocks.retain(|f| f.code != code);
            app.contexts
                .retain(|_, c| c.base_invite_display.as_deref() != Some(code.as_str()));
            app.context_order.retain(|id| app.contexts.contains_key(id));
            app.presence
                .contexts
                .retain(|id, _| app.contexts.contains_key(id));
            app.active = app.context_order.first().copied();
            let _ = cmd_tx.send(Command::Leave { code: code.clone() });
            let _ = at_rest::remove(
                &at_rest::history_dir(&starling::config::Profile::config_dir()),
                &code,
            );
            return Ok(());
        }
        ContextMenuAction::EditSpace => {
            let Some(code) = app.context_menu_code() else {
                return Ok(());
            };
            app.show_context_menu = false;
            app.edit_flock_code = code.clone();
            app.edit_flock_name = app
                .flocks
                .iter()
                .find(|f| f.code == code)
                .map(|f| f.name.clone())
                .or_else(|| {
                    app.roosts
                        .iter()
                        .find(|r| r.code == code)
                        .map(|r| r.name.clone())
                })
                .unwrap_or_default();
            app.show_edit_flock = true;
            return Ok(());
        }
        _ => {}
    }

    let Some(roost_ep) = app.context_menu_roost_endpoint() else {
        return Ok(());
    };

    match action {
        ContextMenuAction::Ban => {
            if let Some(ContextMenuTarget::Bird(endpoint)) = &app.context_menu_target {
                let _ = cmd_tx.send(Command::Ban {
                    roost: roost_ep,
                    target: *endpoint,
                });
            }
        }
        ContextMenuAction::Kick => {
            if let Some(ContextMenuTarget::Bird(endpoint)) = &app.context_menu_target {
                let _ = cmd_tx.send(Command::Kick {
                    roost: roost_ep,
                    target: *endpoint,
                });
            }
        }
        ContextMenuAction::Invite => {
            if let Some(ContextMenuTarget::Bird(endpoint)) = &app.context_menu_target {
                let _ = cmd_tx.send(Command::Invite {
                    roost: roost_ep,
                    target: *endpoint,
                });
            }
        }
        ContextMenuAction::AddChannel => {
            app.show_context_menu = false;
            app.add_channel_input.clear();
            app.show_add_channel = true;
        }
        ContextMenuAction::RemoveChannel => {
            if let Some(ContextMenuTarget::RoostChannel(ri, ci)) = &app.context_menu_target
                && let Some(channel) = app.roosts.get(*ri).and_then(|r| r.channels.get(*ci))
            {
                let _ = cmd_tx.send(Command::RemoveChannel {
                    roost: roost_ep,
                    channel: channel.name.clone(),
                });
            }
        }
        ContextMenuAction::DeleteMessage => {
            if let Some(ContextMenuTarget::RoostChannel(ri, ci)) = &app.context_menu_target
                && let Some(channel) = app.roosts.get(*ri).and_then(|r| r.channels.get(*ci))
                && let Some(last) = channel.messages.last()
            {
                let _ = cmd_tx.send(Command::DeleteMessage {
                    roost: roost_ep,
                    channel: channel.name.clone(),
                    id: last.msg.id.clone(),
                });
            }
        }
        ContextMenuAction::SetRole => {}
        ContextMenuAction::RemoveRoles => {
            if let Some(ContextMenuTarget::Bird(endpoint)) = &app.context_menu_target {
                let _ = cmd_tx.send(Command::SetRole {
                    roost: roost_ep,
                    target: *endpoint,
                    role_index: None,
                });
            }
        }
        ContextMenuAction::TransferOwnership => {
            if let Some(ContextMenuTarget::Bird(endpoint)) = &app.context_menu_target {
                let _ = cmd_tx.send(Command::TransferOwnership {
                    roost: roost_ep,
                    target: *endpoint,
                });
            }
        }
        // Handled above the endpoint guard; unreachable here.
        ContextMenuAction::LeaveSpace
        | ContextMenuAction::EditSpace
        | ContextMenuAction::DeleteSpace => {}
    }
    Ok(())
}

fn handle_menu_key(
    app: &mut App,
    k: &KeyEvent,
    cmd_tx: &mpsc::UnboundedSender<Command>,
    term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<KeyOutcome> {
    match k.code {
        KeyCode::Up => {
            app.menu_selection = app.menu_selection.saturating_sub(1);
        }
        KeyCode::Down => {
            app.menu_selection = (app.menu_selection + 1).min(MENU_ITEMS.len() - 1);
        }
        KeyCode::Enter => {
            activate_menu_item(app, cmd_tx, term)?;
        }
        KeyCode::Esc => {
            app.show_menu = false;
        }
        _ => {}
    }
    Ok(KeyOutcome::Handled)
}
/// Send the composer contents: slash commands (/join, /chirp) or a plain
/// message to the active space. Shared by the Enter key and the composer's
/// send button.
fn send_input(app: &mut App, cmd_tx: &mpsc::UnboundedSender<Command>) {
    let text = std::mem::take(&mut app.input);
    app.input_focus = false;
    if let Some(code) = text
        .strip_prefix("/join ")
        .or_else(|| text.strip_prefix("/join-roost "))
    {
        let code = code.trim();
        match sanitize::invite(code) {
            Ok(normalized) => {
                let since = app.newest_ts(&normalized).unwrap_or(0);
                let _ = cmd_tx.send(Command::Join {
                    code: normalized,
                    since,
                });
            }
            Err(error) => {
                app.error_message = Some(format!("Invalid join code: {error}"));
            }
        }
    } else if let Some(rest) = text.strip_prefix("/chirp ") {
        let rest = rest.trim();
        let (name, body) = match rest.split_once(' ') {
            Some((name, body)) if !name.is_empty() => (name.trim(), body),
            _ => {
                app.error_message = Some("Usage: /chirp <name> <message>".into());
                return;
            }
        };
        // Match the destination by display name (set by an authenticated
        // profile announcement) and resolve it to a verified endpoint.
        let to = app
            .peer_names
            .iter()
            .find(|(_, peer_name)| peer_name.trim() == name)
            .and_then(|(id, _)| app.peer_dm_keys.get(id).map(|_| *id));
        let to = match to {
            Some(to) => to,
            None => {
                app.error_message = Some(format!("{name:?} hasn't published a DM key yet"));
                return;
            }
        };
        let Some(code) = app.active_send_code() else {
            app.error_message = Some("Select a flock first".into());
            return;
        };
        let their_pk = match app.peer_dm_keys.get(&to).cloned() {
            Some(pk) => pk,
            None => return,
        };
        let _ = cmd_tx.send(Command::SendChirp {
            flock: code,
            to,
            their_pk,
            body: body.to_string(),
        });
    } else if let Some(_space) = app.active {
        if let Some(code) = app.active_send_code() {
            let _ = cmd_tx.send(Command::SendText {
                flock: code.to_string(),
                body: text,
            });
        }
    } else if let Some(code) = app.active_code() {
        let _ = cmd_tx.send(Command::SendText {
            flock: code.to_string(),
            body: text,
        });
    }
}

fn handle_normal_key(
    app: &mut App,
    k: &KeyEvent,
    cmd_tx: &mpsc::UnboundedSender<Command>,
) -> anyhow::Result<KeyOutcome> {
    match k.code {
        KeyCode::Enter if k.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.in_call {
                #[cfg(feature = "audio")]
                let _ = cmd_tx.send(Command::HangUp);
                app.in_call = false;
                app.show_video = false;
            } else {
                start_call_with_message(
                    app,
                    cmd_tx,
                    app.selected_peer_id()
                        .map_or_else(|| app.active_peers(), |peer| vec![peer]),
                );
            }
        }

        KeyCode::Enter if app.input_focus && !app.input.is_empty() => {
            send_input(app, cmd_tx);
        }

        KeyCode::Up if matches!(app.v2_view, ui::V2View::Home) && !app.input_focus => {
            app.reference_dm_selected = app.reference_dm_selected.saturating_sub(1);
        }
        KeyCode::Down if matches!(app.v2_view, ui::V2View::Home) && !app.input_focus => {
            app.reference_dm_selected = (app.reference_dm_selected + 1).min(4);
        }
        KeyCode::Enter if matches!(app.v2_view, ui::V2View::Home) && !app.input_focus => {}

        KeyCode::Up if k.modifiers.contains(KeyModifiers::ALT) => {
            let nav = nav_items(app);
            if let Some(pos) = nav.iter().position(|s| *s == app.selection)
                && pos > 0
            {
                app.select(nav[pos - 1]);
            }
        }
        KeyCode::Down if k.modifiers.contains(KeyModifiers::ALT) => {
            let nav = nav_items(app);
            if let Some(pos) = nav.iter().position(|s| *s == app.selection)
                && pos + 1 < nav.len()
            {
                app.select(nav[pos + 1]);
            }
        }
        KeyCode::Right if k.modifiers.contains(KeyModifiers::ALT) => match app.selection {
            Selection::Flock(_) => {}
            Selection::Channel(ri, _) => {
                app.toggle_expand(ri);
            }
        },
        KeyCode::Left if k.modifiers.contains(KeyModifiers::ALT) => match app.selection {
            Selection::Flock(_) => {}
            Selection::Channel(ri, _) => {
                app.toggle_expand(ri);
            }
        },

        KeyCode::PageUp => {
            page_scroll(app, -1.0, crossterm::terminal::size()?.1);
        }
        KeyCode::PageDown => {
            page_scroll(app, 1.0, crossterm::terminal::size()?.1);
        }

        KeyCode::Esc if app.in_call => {
            #[cfg(feature = "audio")]
            let _ = cmd_tx.send(Command::HangUp);
            app.in_call = false;
            app.show_video = false;
        }

        KeyCode::Esc if app.input_focus => {
            app.input_focus = false;
            app.input.clear();
        }

        KeyCode::Esc if app.show_pinned => {
            app.show_pinned = false;
        }
        KeyCode::Esc if app.show_notifications => {
            app.show_notifications = false;
        }
        KeyCode::Esc => {
            app.show_menu = true;
            app.menu_selection = 0;
        }

        KeyCode::Tab if !app.peers.is_empty() => {
            app.select_next_peer();
            app.scroll_focus = ScrollPanel::Birds;
        }
        KeyCode::Up if !app.peers.is_empty() => {
            if app.selected_peer > 0 {
                app.selected_peer -= 1;
            } else {
                app.selected_peer = app.peers.len() - 1;
            }
            app.scroll_focus = ScrollPanel::Birds;
        }
        KeyCode::Down if !app.peers.is_empty() => {
            app.selected_peer = (app.selected_peer + 1) % app.peers.len();
            app.scroll_focus = ScrollPanel::Birds;
        }

        KeyCode::Char('b')
            if k.modifiers.contains(KeyModifiers::CONTROL)
                && app.my_perms.contains(starling::roost::perms::Perm::BAN) =>
        {
            if let (Some(roost_id), Some(target)) =
                (app.selected_roost_endpoint_id(), app.selected_peer_id())
            {
                let _ = cmd_tx.send(Command::Ban {
                    roost: roost_id,
                    target,
                });
            } else {
                app.error_message = Some("Select a bird to ban".into());
            }
        }

        KeyCode::Char('k')
            if k.modifiers.contains(KeyModifiers::CONTROL)
                && app.my_perms.contains(starling::roost::perms::Perm::KICK) =>
        {
            if let (Some(roost_id), Some(target)) =
                (app.selected_roost_endpoint_id(), app.selected_peer_id())
            {
                let _ = cmd_tx.send(Command::Kick {
                    roost: roost_id,
                    target,
                });
            }
        }

        KeyCode::Char('c' | 'C')
            if !app.input_focus
                && k.modifiers.contains(KeyModifiers::CONTROL)
                && k.modifiers.contains(KeyModifiers::SHIFT) =>
        {
            start_call_with_message(
                app,
                cmd_tx,
                app.selected_peer_id()
                    .map_or_else(|| app.active_peers(), |peer| vec![peer]),
            );
        }

        KeyCode::Char(',') if !app.input_focus => {
            app.accent_input = match app.palette.accent {
                ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02X}{g:02X}{b:02X}"),
                _ => "#6FAE9D".to_string(),
            };
            app.settings_open = true;
        }

        KeyCode::Char(c) if app.input_focus => {
            app.input = sanitize::sanitize_message(&format!("{}{}", app.input, c));
        }

        KeyCode::Char(c) if !app.input_focus => {
            app.input_focus = true;
            app.input = sanitize::sanitize_message(&format!("{}{}", app.input, c));
        }

        KeyCode::Backspace if app.input_focus => {
            app.input.pop();
        }

        _ => {}
    }
    Ok(KeyOutcome::Fallthrough)
}

fn parse_join_arg(args: &[String]) -> Result<Option<String>, &'static str> {
    let Some(first) = args.get(1).map(String::as_str) else {
        return Ok(None);
    };
    // Deep link: the OS hands us the whole `starling://join/<code>` URL as a
    // single argument. Strip the scheme prefix and route the remainder through
    // the same validation path as `starling-tui join <code>`.
    let code = if let Some(rest) = first.strip_prefix("starling://join/") {
        rest
    } else if first == "join" {
        match args.get(2).map(|code| code.trim()) {
            Some(code) => code,
            None => return Err("Usage: starling-tui join <code>"),
        }
    } else {
        return Ok(None);
    };
    let code = code.trim();
    if starling::net::decode_typed_code(code).is_none() {
        return Err("Invalid or unsupported join code.");
    }
    Ok(Some(code.to_ascii_uppercase()))
}

/// Validate and normalize a leave code exactly the way [`parse_join_arg`]
/// normalizes a join code, so the value we match against persisted
/// `ContextDescriptor::secret` values is comparable. Returns the uppercased
/// code when valid, `None` when the code is not a recognized typed code.
fn normalize_leave_code(raw: &str) -> Option<String> {
    let code = raw.trim();
    starling::net::decode_typed_code(code)?;
    Some(code.to_ascii_uppercase())
}

/// Headless `starling-tui leave <code>` entry point. Removes every persisted
/// flock/roost context whose stored join secret matches `code` (a roost stores
/// one context per channel, all keyed by the same roost code, so a single leave
/// drops the whole roost) and drops any credential keyed by the same code.
///
/// This operates only on the on-disk state files. If the TUI is currently
/// running it will overwrite these files on exit, so close the TUI first.
fn run_leave(args: &[String]) -> anyhow::Result<()> {
    let raw = args.get(2).map(String::as_str).unwrap_or_else(|| {
        eprintln!("Usage: starling-tui leave <code>");
        std::process::exit(1);
    });
    let code = normalize_leave_code(raw).unwrap_or_else(|| {
        eprintln!("Invalid or unsupported join code.");
        std::process::exit(1);
    });
    let config_dir = starling::config::Profile::config_dir();
    let state_path = config_dir.join("public").join("contexts.bin");
    let protected_path = config_dir.join("protected").join("credentials.bin");
    let report = leave_context(&state_path, &protected_path, &code)?;
    if report.removed == 0 {
        println!("No saved flock or roost matched that code.");
    } else {
        println!(
            "✓ Left {} saved context(s) matching that code.",
            report.removed
        );
    }
    Ok(())
}

struct LeaveReport {
    removed: usize,
}

/// Remove persisted contexts and credentials matching `code`. Pure and
/// path-parameterized so it can be unit-tested without touching the real
/// config directory.
fn leave_context(
    state_path: &std::path::Path,
    protected_path: &std::path::Path,
    code: &str,
) -> anyhow::Result<LeaveReport> {
    let mut removed = 0;
    if state_path.exists() {
        let mut state = persistence::load_public(state_path)?;
        let removed_spaces: Vec<starling::protocol::SpaceId> = state
            .contexts
            .iter()
            .filter(|ctx| ctx.secret.as_deref() == Some(code))
            .map(|ctx| ctx.space)
            .collect();
        removed = removed_spaces.len();
        if removed > 0 {
            state
                .contexts
                .retain(|ctx| ctx.secret.as_deref() != Some(code));
            if let Some(active) = state.active_space
                && removed_spaces.contains(&active)
            {
                state.active_space = None;
            }
            persistence::save_public(state_path, &state)?;
        }
    }
    if protected_path.exists() {
        let mut protected = persistence::load_protected(protected_path)?;
        let before = protected.credentials.len();
        protected
            .credentials
            .retain(|credential| credential.name != code);
        if protected.credentials.len() != before {
            persistence::save_protected(protected_path, &protected)?;
        }
    }
    Ok(LeaveReport { removed })
}

fn copy_active_invite<C: Clipboard + ?Sized>(
    app: &mut App,
    clipboard: Option<&mut C>,
    now: Instant,
) {
    let Some(invite) = app.active_code().map(str::to_owned) else {
        app.error_message = Some("No public invite is available for this context".into());
        return;
    };
    let Some(clipboard) = clipboard else {
        app.error_message = Some("Clipboard unavailable on this system".into());
        return;
    };
    match clipboard.set_text(&format!("starling://join/{invite}")) {
        Ok(()) => {
            app.error_message = None;
            app.show_status_notice("Invite copied", now);
        }
        Err(error) => app.error_message = Some(error.to_string()),
    }
}

fn save_context_state(path: &std::path::Path, app: &App) -> anyhow::Result<()> {
    let contexts = app
        .context_order
        .iter()
        .filter_map(|space| {
            app.contexts
                .get(space)
                .map(|context| persistence::ContextDescriptor {
                    space: *space,
                    label: context.title.clone(),
                    secret: context.secret.clone(),
                })
        })
        .collect();
    persistence::save_public(
        path,
        &persistence::PublicState {
            contexts,
            active_space: app.active,
        },
    )
}

fn panel_geometry(term_h: u16) -> (u16, u16, u16, u16, u16) {
    let body_top = 2;
    let body_h = term_h.saturating_sub(6);
    let flocks_pct = (body_h * 33) / 100;
    // Roosts uses Min(3) so it always gets at least 1 content row + 2 border rows.
    let roosts_h = (body_h.saturating_sub(flocks_pct)).max(3);
    let flocks_h = body_h.saturating_sub(roosts_h);
    let roosts_top = body_top + flocks_h;
    (body_top, flocks_h, roosts_top, roosts_h, body_h)
}

fn page_scroll(app: &mut App, direction: f32, term_h: u16) {
    let (_, flocks_h, _, roosts_h, birds_h) = panel_geometry(term_h);
    let page = match app.scroll_focus {
        ScrollPanel::Flocks => flocks_h.saturating_sub(2),
        ScrollPanel::Roosts => roosts_h.saturating_sub(2),
        ScrollPanel::Birds => birds_h.saturating_sub(2),
    }
    .max(1) as f32;
    app.scroll_mut(app.scroll_focus).scroll(direction * page);
}

fn handle_mouse_scroll(app: &mut App, col: u16, row: u16, delta: f32) -> anyhow::Result<()> {
    let (term_w, term_h) = crossterm::terminal::size()?;
    let (flocks_top, flocks_h, roosts_top, roosts_h, birds_h) = panel_geometry(term_h);
    let panel = if col < 26 && row >= flocks_top && row < flocks_top + flocks_h {
        Some(ScrollPanel::Flocks)
    } else if col < 26 && row >= roosts_top && row < roosts_top + roosts_h {
        Some(ScrollPanel::Roosts)
    } else if col >= term_w.saturating_sub(27) && row >= flocks_top && row < flocks_top + birds_h {
        Some(ScrollPanel::Birds)
    } else {
        None
    };

    if let Some(panel) = panel {
        app.scroll_focus = panel;
        app.scroll_mut(panel).scroll(delta);
    }
    Ok(())
}

fn handle_settings_mouse_click(app: &mut App, modal: Rect, col: u16, row: u16) -> bool {
    let tabs = [
        SettingsTab::Account,
        ui::SettingsTab::Voice,
        SettingsTab::Appearance,
        ui::SettingsTab::Notifications,
        ui::SettingsTab::Keybinds,
    ];
    // Nav column: 22 wide, starts one cell inside the modal; each tab is one row.
    let nav_x = modal.x + 1;
    let nav_w = 22u16;
    if col >= nav_x && col < nav_x + nav_w {
        let tab_row = row.saturating_sub(modal.y + 1) as usize;
        if let Some(tab) = tabs.get(tab_row) {
            app.settings_tab = *tab;
            return true;
        }
    }
    // Click anywhere else inside the modal: consume so it does not leak to
    // the underlying UI handlers.
    true
}

/// Header icon row on narrow terminals: channel/friends toggle (when the
/// sidebar is hidden), members, bell, pin, call. Mirrors the desktop chat-header
/// hit regions (right-aligned, no rail column).
fn handle_mobile_header_click(
    app: &mut App,
    cmd_tx: &mpsc::UnboundedSender<Command>,
    col: u16,
    row: u16,
    term_w: u16,
) -> bool {
    if row > 1 {
        return false;
    }
    match ui::header_icon_at(app, col, term_w) {
        Some(ui::HeaderIcon::Call) => {
            start_call_with_message(
                app,
                cmd_tx,
                app.selected_peer_id()
                    .map_or_else(|| app.active_peers(), |peer| vec![peer]),
            );
        }
        Some(ui::HeaderIcon::Pin) => {
            app.show_pinned = !app.show_pinned;
        }
        Some(ui::HeaderIcon::Bell) => {
            app.show_notifications = !app.show_notifications;
        }
        Some(ui::HeaderIcon::Members) => {
            app.show_members = !app.show_members;
        }
        Some(ui::HeaderIcon::Menu) => {
            // Very narrow screens: the channel/friends list is hidden; this
            // button restores it (Space view) or the Friends list (Home view).
            app.sidebar_hidden = false;
        }
        None => {}
    }
    true
}

/// Bottom rail clicks on narrow terminals: home pill first, then one pill per
/// roost, mirroring the desktop left rail geometry.
fn handle_mobile_rail_click(app: &mut App, col: u16, row: u16, term_w: u16, term_h: u16) -> bool {
    if term_w >= ui::MOBILE_BREAKPOINT {
        return false;
    }
    let rail_h = 3u16;
    if row < term_h.saturating_sub(rail_h) || row > term_h.saturating_sub(1) {
        return false;
    }
    let pill_w = 6u16;
    let gap = 1u16;
    let mut x = 1u16;
    if col >= x && col < x + pill_w {
        app.open_home();
        return true;
    }
    x += pill_w + gap;
    for (index, _roost) in app.roosts.iter().enumerate() {
        if x + pill_w > term_w {
            break;
        }
        if col >= x && col < x + pill_w {
            app.select(Selection::Channel(index, 0));
            return true;
        }
        x += pill_w + gap;
    }
    false
}

/// Sidebar clicks on narrow terminals: the Friends list (Home) or the channel
/// list (Space) occupies the left 30 columns when visible. Mirrors the desktop
/// sidebar hit-testing.
fn handle_mobile_sidebar_click(
    app: &mut App,
    col: u16,
    row: u16,
    term_w: u16,
    term_h: u16,
) -> bool {
    if term_w >= ui::MOBILE_BREAKPOINT || col >= 30 || row < 2 || row >= term_h.saturating_sub(3) {
        return false;
    }
    let members_open = app.show_members
        && (matches!(app.v2_view, ui::V2View::Space)
            || (matches!(app.v2_view, ui::V2View::Home)
                && matches!(app.selection, Selection::Flock(_))
                && app.selected_dm.is_none()));
    if members_open {
        return false; // members panel occupies the right side, not the left
    }
    if matches!(app.v2_view, ui::V2View::Home) {
        let list_index = (row - 2) as usize;
        if list_index == 0 {
            return true; // DIRECT MESSAGES header
        }
        let peer_count = app.peers.len();
        if list_index <= peer_count {
            if let Some(peer) = app.peers.get(list_index - 1).copied() {
                app.selected_dm = Some(peer);
                return true;
            }
        } else {
            let flock_index = list_index - peer_count - 1;
            if flock_index < app.flocks.len() {
                app.select_flock(flock_index);
                return true;
            }
        }
    } else if let Selection::Channel(ri, _) = app.selection {
        let channel_index = (row - 2).saturating_sub(1) as usize;
        if app
            .roosts
            .get(ri)
            .and_then(|roost| roost.channels.get(channel_index))
            .is_some()
        {
            app.select(Selection::Channel(ri, channel_index));
            return true;
        }
    }
    false
}

fn handle_reference_mouse_click(
    app: &mut App,
    cmd_tx: &mpsc::UnboundedSender<Command>,
    col: u16,
    row: u16,
    term_w: u16,
    term_h: u16,
) -> bool {
    if term_w < ui::MOBILE_BREAKPOINT {
        return false;
    }
    let chat_left = 39u16;
    // Sidebar header row: open the menu.
    if row == 0 && (9..chat_left).contains(&col) {
        app.show_menu = true;
        app.menu_selection = 0;
        return true;
    }
    // Chat header row: right-side icons (members, bell, pin, call).
    if row <= 1 && col >= chat_left {
        match ui::header_icon_at(app, col, term_w) {
            Some(ui::HeaderIcon::Call) => {
                start_call_with_message(
                    app,
                    cmd_tx,
                    app.selected_peer_id()
                        .map_or_else(|| app.active_peers(), |peer| vec![peer]),
                );
            }
            Some(ui::HeaderIcon::Pin) => {
                app.show_pinned = !app.show_pinned;
            }
            Some(ui::HeaderIcon::Bell) => {
                app.show_notifications = !app.show_notifications;
            }
            Some(ui::HeaderIcon::Members) => {
                app.show_members = !app.show_members;
            }
            _ => {}
        }
        return true;
    }
    // Server rail: home pill (rows 1-3) then roosts (rows 5+).
    if col < 9 && row >= 1 && row < term_h.saturating_sub(4) {
        if row <= 3 {
            app.open_home();
            return true;
        }
        let roost_index = (row.saturating_sub(5) / 4) as usize;
        if app.roosts.get(roost_index).is_some() {
            app.select(Selection::Channel(roost_index, 0));
        }
        return true;
    }
    // Sidebar list: DM row 0 (header), then peers and flocks merged; channels in Space.
    if (9..chat_left).contains(&col) && row >= 2 && row < term_h.saturating_sub(4) {
        let list_index = (row - 2) as usize;
        if matches!(app.v2_view, ui::V2View::Home) {
            if list_index == 0 {
                return true; // DIRECT MESSAGES header
            }
            let peer_count = app.peers.len();
            if list_index <= peer_count {
                if let Some(peer) = app.peers.get(list_index - 1).copied() {
                    app.selected_dm = Some(peer);
                    return true;
                }
            } else {
                let flock_index = list_index - peer_count - 1;
                if flock_index < app.flocks.len() {
                    app.select_flock(flock_index);
                    return true;
                }
            }
        } else if let Selection::Channel(ri, _) = app.selection {
            let channel_index = list_index.saturating_sub(1);
            if app
                .roosts
                .get(ri)
                .and_then(|roost| roost.channels.get(channel_index))
                .is_some()
            {
                app.select(Selection::Channel(ri, channel_index));
                return true;
            }
        }
        return true;
    }
    // Sidebar footer: profile row, then controls (mic, headset, settings).
    if (9..39).contains(&col) && row >= term_h.saturating_sub(4) {
        if row == term_h.saturating_sub(3) {
            app.profile_panel.open = true;
            return true;
        }
        if row == term_h.saturating_sub(2) {
            let rel = col.saturating_sub(9);
            if rel < 3 {
                app.muted = !app.muted;
            } else if rel < 6 {
                app.deafened = !app.deafened;
                app.muted = app.deafened;
            } else {
                app.settings_open = true;
            }
            return true;
        }
        return true;
    }
    // Composer / chat body: focus the input.
    if row >= term_h.saturating_sub(3) && col >= chat_left {
        app.input_focus = true;
        return true;
    }
    false
}

/// Which action button (if any) a click inside a popup hits, mirroring the
/// button rows drawn by `draw_popup_buttons`.
fn popup_action_button(app: &App, popup: Rect, col: u16, row: u16) -> Option<usize> {
    if app.show_create_room {
        return ui::popup_button_at(popup, col, row, &["Cancel", "Create"]);
    }
    if app.show_create_roost {
        return ui::popup_button_at(popup, col, row, &["Cancel", "Create"]);
    }
    if app.show_add_channel {
        return ui::popup_button_at(popup, col, row, &["Cancel", "Create"]);
    }
    if app.show_edit_flock {
        return ui::popup_button_at(popup, col, row, &["Cancel", "Save"]);
    }
    if app.show_join_room {
        return ui::popup_button_at(popup, col, row, &["Cancel", "Join"]);
    }
    if app.show_delete_confirm {
        return ui::popup_button_at(popup, col, row, &["Cancel", "Delete"]);
    }
    if app.show_bird_profile {
        // The Call row is the third content row of the 40x7 popup.
        #[cfg(feature = "audio")]
        {
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
            if row == rows[2].y && col >= rows[2].x && col < rows[2].right() {
                return Some(0);
            }
        }
        return None;
    }
    if app.profile_panel.open {
        let buttons: &[&str] = if app.profile_panel.editing {
            &["Cancel", "Save"]
        } else {
            &["Edit"]
        };
        return ui::popup_button_at(popup, col, row, buttons);
    }
    if app.settings_open && app.settings_tab == ui::SettingsTab::Appearance {
        return ui::popup_button_at(popup, col, row, &["Cycle icons", "Apply accent"]);
    }
    None
}

/// Run the action behind a popup button click. Index 0 is always the
/// cancel/close action; index 1 (when present) is the primary action.
fn activate_popup_action(
    app: &mut App,
    cmd_tx: &mpsc::UnboundedSender<Command>,
    idx: usize,
) -> anyhow::Result<()> {
    if app.show_create_room {
        if idx == 0 {
            app.create_flock_code = None;
            app.create_flock_secret = None;
            app.create_flock_name.clear();
            app.show_create_room = false;
        } else {
            confirm_create_flock(app, cmd_tx);
        }
        return Ok(());
    }
    if app.show_create_roost {
        if idx == 0 {
            app.show_create_roost = false;
        } else {
            confirm_create_roost(app, cmd_tx);
        }
        return Ok(());
    }
    if app.show_add_channel {
        if idx == 0 {
            app.show_add_channel = false;
        } else {
            confirm_add_channel(app, cmd_tx);
        }
        return Ok(());
    }
    if app.show_edit_flock {
        if idx == 0 {
            app.show_edit_flock = false;
        } else {
            confirm_edit_flock(app, cmd_tx);
        }
        return Ok(());
    }
    if app.show_join_room {
        if idx == 0 {
            app.show_join_room = false;
        } else {
            match sanitize::invite(app.join_input.trim()) {
                Ok(code) => confirm_join(app, cmd_tx, code),
                Err(error) => {
                    app.error_message = Some(error.to_string());
                }
            }
        }
        return Ok(());
    }
    if app.show_delete_confirm {
        if idx == 0 {
            app.show_delete_confirm = false;
            app.delete_confirm_input.clear();
        } else {
            confirm_delete(app);
        }
        return Ok(());
    }
    if app.show_bird_profile {
        confirm_bird_call(app, cmd_tx);
        return Ok(());
    }
    if app.profile_panel.open {
        if app.profile_panel.editing {
            if idx == 0 {
                app.profile_panel.editing = false;
            } else {
                save_profile(
                    app,
                    &mut starling::config::Profile::load().unwrap_or_default(),
                )?;
            }
        } else {
            open_profile(app);
            app.profile_panel.editing = true;
        }
        return Ok(());
    }
    if app.settings_open && app.settings_tab == ui::SettingsTab::Appearance {
        if idx == 0 {
            app.icon_style = app.icon_style.next();
        } else {
            let value = app.accent_input.clone();
            if ui::apply_accent_color(app, &value) {
                if let Some(mut profile) = starling::config::Profile::load() {
                    profile.accent_color = value;
                    let _ = profile.save();
                }
            } else {
                app.error_message = Some("Use #RRGGBB".into());
            }
        }
        return Ok(());
    }
    Ok(())
}

fn handle_mouse_click(
    app: &mut App,
    cmd_tx: &mpsc::UnboundedSender<Command>,
    muted_flag: &Arc<AtomicBool>,
    _clipboard: Option<&mut clipboard::SystemClipboard>,
    term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    col: u16,
    row: u16,
) -> anyhow::Result<()> {
    let (term_w, term_h) = crossterm::terminal::size()?;

    // Popup dismissal: X button in the top-right, or click outside the popup.
    if let Some(popup) = ui::active_popup_rect(app, term_w, term_h) {
        let x_glyph = ui::TerminalIcon::Close.glyph(app.icon_style);
        let x_w = x_glyph.chars().count() as u16;
        let x_rect = Rect {
            x: popup.right().saturating_sub(x_w),
            y: popup.y,
            width: x_w,
            height: 1,
        };
        if x_rect.contains(Position::new(col, row)) {
            ui::dismiss_active_popup(app);
            return Ok(());
        }
        if !popup.contains(Position::new(col, row)) {
            ui::dismiss_active_popup(app);
            return Ok(());
        }
        // Action buttons on the popup's bottom row (Cancel / Create / Save /
        // Join / Delete / Edit / Call / Apply accent / Cycle icons).
        if let Some(idx) = popup_action_button(app, popup, col, row) {
            activate_popup_action(app, cmd_tx, idx)?;
            return Ok(());
        }
        // Clicks inside a modal that are not on a button are consumed so they
        // never leak to the UI underneath. The settings modal still routes
        // nav-tab clicks through handle_settings_mouse_click below.
        if app.profile_panel.open {
            return Ok(());
        }
    }

    if app.show_menu {
        if let Some(popup) = ui::active_popup_rect(app, term_w, term_h) {
            if let Some(idx) = menu_item_at_size(popup, col, row) {
                app.menu_selection = idx;
                activate_menu_item(app, cmd_tx, term)?;
            } else if !popup.contains(Position::new(col, row)) {
                app.show_menu = false;
            }
        }
        return Ok(());
    }

    // Call overlay: click the control row (mute / video / disconnect).
    if app.in_call
        && let Some(overlay) = ui::active_popup_rect(app, term_w, term_h)
        && overlay.contains(Position::new(col, row))
    {
        match ui::call_control_at(overlay, col, row) {
            Some(ui::CallControl::Mute) => {
                app.muted = !app.muted;
                muted_flag.store(app.muted, Ordering::Relaxed);
            }
            Some(ui::CallControl::Video) => {
                toggle_call_video(app, cmd_tx);
            }
            Some(ui::CallControl::Disconnect) => {
                #[cfg(feature = "audio")]
                let _ = cmd_tx.send(Command::HangUp);
                app.in_call = false;
                app.show_video = false;
            }
            None => {}
        }
        return Ok(());
    }

    // Context menu / role submenu: mouse-first. Click an item to activate,
    // click outside to dismiss (submenu returns to the parent menu).
    if app.show_context_menu {
        if let Some(idx) = context_menu_item_at(app, col, row, term_w, term_h) {
            app.context_menu_selection = idx;
            activate_context_menu_item(app, cmd_tx, idx)?;
        } else {
            app.show_context_menu = false;
        }
        return Ok(());
    }
    if app.show_role_submenu {
        if let Some(idx) = role_submenu_item_at(app, col, row, term_w, term_h) {
            app.role_submenu_selection = idx;
            activate_role_submenu_item(app, cmd_tx, idx)?;
        } else {
            app.show_role_submenu = false;
            app.show_context_menu = true;
        }
        return Ok(());
    }

    // Settings modal: click a nav tab to switch, or click inside the content
    // area to focus the accent input on the Appearance tab.
    if app.settings_open
        && let Some(modal) = ui::active_popup_rect(app, term_w, term_h)
    {
        if handle_settings_mouse_click(app, modal, col, row) {
            return Ok(());
        }
        return Ok(());
    }

    if handle_mobile_rail_click(app, col, row, term_w, term_h) {
        return Ok(());
    }

    if term_w < ui::MOBILE_BREAKPOINT && handle_mobile_header_click(app, cmd_tx, col, row, term_w) {
        return Ok(());
    }

    if term_w < ui::MOBILE_BREAKPOINT && handle_mobile_sidebar_click(app, col, row, term_w, term_h)
    {
        return Ok(());
    }

    // Composer: click the input to focus, the send button to send, or the
    // emoji button (placeholder for now).
    if let Some(hit) = ui::composer_hit_at(app, col, row, term_w, term_h) {
        match hit {
            ui::ComposerHit::Send => {
                if !app.input.is_empty() {
                    send_input(app, cmd_tx);
                }
            }
            ui::ComposerHit::Emoji => {
                app.error_message = Some("Emoji picker coming soon".into());
            }
            ui::ComposerHit::Input => {
                app.input_focus = true;
            }
        }
        return Ok(());
    }

    if handle_reference_mouse_click(app, cmd_tx, col, row, term_w, term_h) {
        return Ok(());
    }
    if handle_v2_mouse_click(app, col, row, term_w, term_h) {
        return Ok(());
    }

    if app.show_edit_flock {
        return Ok(());
    }

    if app.show_join_room {
        return Ok(());
    }

    let (flocks_top, flocks_h, roosts_top, roosts_h, birds_h) = panel_geometry(term_h);
    if col < 26 && row > flocks_top && row < flocks_top + flocks_h.saturating_sub(1) {
        app.scroll_focus = ScrollPanel::Flocks;
        let visible_row = (row - flocks_top - 1) as usize;
        if let Some(idx) = app.flock_scroll.row_index(visible_row) {
            let typed_count = app.context_order.len();
            if idx < typed_count {
                let space = app.context_order[idx];
                app.select_context(space);
                let _ = cmd_tx.send(Command::SelectContext(space));
            } else {
                // Skip legacy flocks that already have a typed context so
                // the click target matches what draw_flocks actually renders.
                let remaining = idx - typed_count;
                if let Some(flock_idx) = app
                    .flocks
                    .iter()
                    .enumerate()
                    .filter(|(_, fv)| {
                        !app.contexts
                            .values()
                            .any(|ctx| ctx.secret.as_deref() == Some(fv.code.as_str()))
                    })
                    .nth(remaining)
                    .map(|(i, _)| i)
                {
                    app.select(Selection::Flock(flock_idx));
                }
            }
        }
    } else if col < 26 && row >= roosts_top && row < roosts_top + roosts_h.saturating_sub(1) {
        app.scroll_focus = ScrollPanel::Roosts;
        let visible_row = (row - roosts_top - 1) as usize;
        if let Some(content_row) = app.roost_scroll.row_index(visible_row) {
            let mut cursor = 0usize;
            for (ri, rv) in app.roosts.iter().enumerate() {
                if cursor == content_row {
                    app.toggle_expand(ri);
                    return Ok(());
                }
                cursor += 1;
                if app.expanded.contains(&ri) {
                    for ci in 0..rv.channels.len() {
                        if cursor == content_row {
                            app.select(Selection::Channel(ri, ci));
                            return Ok(());
                        }
                        cursor += 1;
                    }
                }
            }
        }
    } else if col >= term_w.saturating_sub(27)
        && row > flocks_top
        && row < flocks_top + birds_h.saturating_sub(1)
    {
        app.scroll_focus = ScrollPanel::Birds;
        let visible_row = (row - flocks_top - 1) as usize;
        if let Some(content_row) = app.bird_scroll.row_index(visible_row) {
            if content_row == 0 {
                open_profile(app);
                return Ok(());
            }
            if content_row <= app.active_peers().len() {
                app.selected_peer = content_row - 1;
                app.bird_profile_peer = app.selected_peer_id();
                app.show_bird_profile = true;
            }
        }
    }

    Ok(())
}

fn handle_v2_mouse_click(app: &mut App, col: u16, row: u16, term_w: u16, term_h: u16) -> bool {
    if term_w < ui::MOBILE_BREAKPOINT {
        return false;
    }
    let body_top = 2;
    let body_bottom = term_h.saturating_sub(4);
    if row <= body_top || row >= body_bottom || col >= term_w.saturating_sub(27) {
        return false;
    }

    if col < 12 {
        let index = row.saturating_sub(body_top + 1) as usize;
        if index == 0 {
            app.open_home();
            return true;
        }
        let flock_count = app.flocks.len();
        if index <= flock_count {
            app.select(Selection::Flock(index - 1));
            return true;
        }
        let roost_index = index - 1 - flock_count;
        if app.roosts.get(roost_index).is_some() {
            app.select(Selection::Channel(roost_index, 0));
            return true;
        }
    }

    if (12..40).contains(&col) {
        if matches!(app.v2_view, ui::V2View::Home) {
            let peer_index = row.saturating_sub(body_top + 2) as usize;
            if let Some(peer) = app.peers.get(peer_index).copied() {
                app.selected_dm = Some(peer);
                return true;
            }
        } else if let Selection::Channel(roost_index, _) = app.selection {
            let sidebar_row = row.saturating_sub(body_top + 1) as usize;
            if sidebar_row > 0 {
                let channel_index = sidebar_row - 1;
                if app
                    .roosts
                    .get(roost_index)
                    .and_then(|roost| roost.channels.get(channel_index))
                    .is_some()
                {
                    app.select(Selection::Channel(roost_index, channel_index));
                    return true;
                }
            }
        }
    }
    false
}

fn toggle_call_video(app: &mut App, cmd_tx: &mpsc::UnboundedSender<Command>) {
    app.show_video = !app.show_video;
    #[cfg(feature = "video")]
    if app.show_video {
        let _ = cmd_tx.send(Command::StartVideo(app.peers.clone()));
    } else {
        let _ = cmd_tx.send(Command::StopVideo);
    }
}

fn handle_right_click(app: &mut App, col: u16, row: u16) -> anyhow::Result<()> {
    // Right-click on an open popup: back one level in submenus, else dismiss.
    if let Ok((term_w, term_h)) = crossterm::terminal::size()
        && let Some(popup) = ui::active_popup_rect(app, term_w, term_h)
        && popup.contains(Position::new(col, row))
    {
        ui::dismiss_active_popup_one_level(app);
        return Ok(());
    }

    app.show_context_menu = false;
    app.show_role_submenu = false;

    // Headless environments (CI, tests) have no TTY; fall back to a default
    // grid so right-click still works and tests don't panic on size().
    let (term_w, term_h) = crossterm::terminal::size().unwrap_or((120, 30));

    // Narrow terminals: the rail sits at the bottom; right-click a pill to
    // open that space's context menu.
    if term_w < ui::MOBILE_BREAKPOINT {
        let rail_h = 3u16;
        if row >= term_h.saturating_sub(rail_h) && row <= term_h.saturating_sub(1) {
            let pill_w = 6u16;
            let gap = 1u16;
            let mut x = 1u16;
            x += pill_w + gap; // home pill: no menu
            for (index, _roost) in app.roosts.iter().enumerate() {
                if x + pill_w > term_w {
                    break;
                }
                if col >= x && col < x + pill_w {
                    app.build_context_menu(ContextMenuTarget::Roost(index));
                    app.show_context_menu = !app.context_menu_items.is_empty();
                    return Ok(());
                }
                x += pill_w + gap;
            }
        }
        return Ok(());
    }

    // Current layout: rail is 9 wide (home pill rows 1-3, roosts from row 5);
    // sidebar is 9..39 (DM header row 2, peers, then flocks).
    if col < 9 && row >= 1 && row < term_h.saturating_sub(4) {
        if row <= 3 {
            return Ok(()); // home pill: no menu
        }
        let roost_index = (row.saturating_sub(5) / 4) as usize;
        if app.roosts.get(roost_index).is_some() {
            app.build_context_menu(ContextMenuTarget::Roost(roost_index));
            app.show_context_menu = !app.context_menu_items.is_empty();
            return Ok(());
        }
    }
    if (9..39).contains(&col) && row >= 2 && row < term_h.saturating_sub(4) {
        let list_index = (row - 2) as usize;
        if matches!(app.v2_view, ui::V2View::Home) {
            let peer_count = app.peers.len();
            if list_index > peer_count {
                let flock_index = list_index - peer_count - 1;
                if flock_index < app.flocks.len() {
                    app.build_context_menu(ContextMenuTarget::Flock(flock_index));
                    app.show_context_menu = !app.context_menu_items.is_empty();
                    return Ok(());
                }
            }
        }
    }

    let (flocks_top, _flocks_h, roosts_top, roosts_h, birds_h) = panel_geometry(term_h);

    // Hit-test: right side = birds panel
    if col >= term_w.saturating_sub(27)
        && row > flocks_top
        && row < flocks_top + birds_h.saturating_sub(1)
    {
        app.scroll_focus = ScrollPanel::Birds;
        let visible_row = (row - flocks_top - 1) as usize;
        if let Some(content_row) = app.bird_scroll.row_index(visible_row)
            && content_row > 0
            && content_row <= app.active_peers().len()
        {
            let peer_id = app.active_peers()[content_row - 1];
            app.build_context_menu(ContextMenuTarget::Bird(peer_id));
            app.show_context_menu = !app.context_menu_items.is_empty();
            return Ok(());
        }
    }

    // Hit-test: left side = roosts panel
    if col < 26 && row >= roosts_top && row < roosts_top + roosts_h.saturating_sub(1) {
        app.scroll_focus = ScrollPanel::Roosts;
        let visible_row = (row - roosts_top - 1) as usize;
        if let Some(content_row) = app.roost_scroll.row_index(visible_row) {
            let mut cursor = 0usize;
            for (ri, rv) in app.roosts.iter().enumerate() {
                if cursor == content_row {
                    app.build_context_menu(ContextMenuTarget::Roost(ri));
                    app.show_context_menu = !app.context_menu_items.is_empty();
                    return Ok(());
                }
                cursor += 1;
                if app.expanded.contains(&ri) {
                    for ci in 0..rv.channels.len() {
                        if cursor == content_row {
                            app.build_context_menu(ContextMenuTarget::RoostChannel(ri, ci));
                            app.show_context_menu = !app.context_menu_items.is_empty();
                            return Ok(());
                        }
                        cursor += 1;
                    }
                }
            }
        }
    }

    Ok(())
}

fn update_menu_hover(app: &mut App, term_w: u16, term_h: u16, col: u16, row: u16) {
    if let Some(popup) = ui::active_popup_rect(app, term_w, term_h)
        && let Some(index) = menu_item_at_size(popup, col, row)
    {
        app.menu_selection = index;
    }
}

fn menu_item_at_size(popup: Rect, col: u16, row: u16) -> Option<usize> {
    let popup_x = popup.x;
    let popup_y = popup.y;
    let popup_w = popup.width;
    let popup_h = popup.height;
    if col < popup_x || col >= popup_x + popup_w {
        return None;
    }

    let inner_row = row.checked_sub(popup_y)?;
    if inner_row == 0 || inner_row >= popup_h.saturating_sub(1) {
        return None;
    }

    let idx = (inner_row - 1) as usize;
    (idx < MENU_ITEMS.len()).then_some(idx)
}

#[allow(unused_variables)]
fn activate_menu_item(
    app: &mut App,
    cmd_tx: &mpsc::UnboundedSender<Command>,
    term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<()> {
    let i = app.menu_selection;
    if i >= MENU_ITEMS.len() {
        return Ok(());
    }

    app.show_menu = false;

    match i {
        0 => {
            open_create_room(app);
        }
        1 => {
            app.create_roost_input.clear();
            app.show_create_roost = true;
        }
        2 => {
            if !open_edit_flock(app) {
                app.show_menu = true;
            }
        }
        3 => {
            app.join_input.clear();
            app.show_join_room = true;
        }
        4 => {
            open_profile(app);
        }
        5 => {
            app.settings_open = true;
        }
        6 => {
            app.show_menu = false;
            app.show_delete_confirm = true;
        }
        7 => {
            app.quit_requested = true;
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::{
        App, MENU_ITEMS, SettingsTab, handle_reference_mouse_click, handle_right_click,
        handle_settings_mouse_click, leave_context, menu_item_at_size, normalize_leave_code,
        open_create_room, parse_join_arg, popup_action_button, refresh_create_flock_code,
        update_menu_hover,
    };
    use crate::ui::{FlockView, RoostView, Selection, V2View};
    use ratatui::layout::{Margin, Rect};

    #[test]
    fn every_rendered_menu_row_is_clickable() {
        let (width, height) = (100, 40);
        let popup_y = (height - (MENU_ITEMS.len() as u16 + 2)) / 2;
        let popup_x = (width - 28) / 2;
        let popup = Rect::new(popup_x, popup_y, 28, MENU_ITEMS.len() as u16 + 2);

        for index in 0..MENU_ITEMS.len() {
            assert_eq!(
                menu_item_at_size(popup, popup_x + 2, popup_y + 1 + index as u16),
                Some(index)
            );
        }
        assert_eq!(menu_item_at_size(popup, popup_x + 2, popup_y), None);
    }

    #[test]
    fn mouse_coordinates_update_menu_highlight() {
        let (width, height) = (100, 40);
        let popup_y = (height - (MENU_ITEMS.len() as u16 + 2)) / 2;
        let popup_x = (width - 28) / 2;
        let mut app = App::default();
        app.show_menu = true;

        update_menu_hover(&mut app, width, height, popup_x + 3, popup_y + 3);

        assert_eq!(app.menu_selection, 2);
    }

    #[test]
    fn create_room_generates_a_new_flock_code_each_time() {
        let mut app = App {
            node_id: Some(iroh::SecretKey::generate().public()),
            ..App::default()
        };

        open_create_room(&mut app);
        app.create_flock_name = "Night Birds".into();
        refresh_create_flock_code(&mut app);
        let first = app.create_flock_code.clone().expect("first code");
        open_create_room(&mut app);
        app.create_flock_name = "Night Birds".into();
        refresh_create_flock_code(&mut app);
        let second = app.create_flock_code.clone().expect("second code");

        assert_ne!(first, second);
        assert_eq!(
            starling::net::decode_typed_code(&first).map(|code| code.code_type),
            Some(starling::net::CodeType::Flock)
        );
    }

    fn sample_join_code() -> String {
        let mut app = App {
            node_id: Some(iroh::SecretKey::generate().public()),
            ..App::default()
        };
        open_create_room(&mut app);
        app.create_flock_name = "Night Birds".into();
        refresh_create_flock_code(&mut app);
        app.create_flock_code.expect("a flock code")
    }

    #[test]
    fn join_subcommand_accepts_bare_code() {
        let code = sample_join_code();
        let args = ["starling-tui".into(), "join".into(), code.clone()];
        assert_eq!(parse_join_arg(&args).unwrap(), Some(code));
    }

    #[test]
    fn join_subcommand_without_code_reports_usage() {
        let args = ["starling-tui".into(), "join".into()];
        assert_eq!(
            parse_join_arg(&args).unwrap_err(),
            "Usage: starling-tui join <code>"
        );
    }

    #[test]
    fn deep_link_strips_scheme_and_routes_to_join() {
        let code = sample_join_code();
        let url = format!("starling://join/{code}");
        let args = ["starling-tui".into(), url];
        assert_eq!(parse_join_arg(&args).unwrap(), Some(code));
    }

    #[test]
    fn deep_link_with_invalid_code_is_rejected() {
        let args = ["starling-tui".into(), "starling://join/NOT-A-CODE".into()];
        assert!(parse_join_arg(&args).is_err());
    }

    #[test]
    fn unrelated_first_argument_returns_none() {
        let args = ["starling-tui".into(), "--version".into()];
        assert_eq!(parse_join_arg(&args).unwrap(), None);
    }

    #[test]
    fn leave_code_normalizes_like_a_join_code() {
        let code = sample_join_code();
        // Same uppercased form that `parse_join_arg` produces and that the TUI
        // stores as `ContextDescriptor::secret`.
        assert_eq!(normalize_leave_code(&code), Some(code.clone()));
        assert_eq!(normalize_leave_code(&code.to_lowercase()), Some(code));
        assert_eq!(normalize_leave_code("NOT-A-CODE"), None);
        assert_eq!(normalize_leave_code(""), None);
    }

    #[test]
    fn leave_removes_matching_contexts_and_clears_active_space() {
        use crate::persistence::{ContextDescriptor, PublicState, save_public};
        use starling::protocol::{FlockId, SpaceId};

        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("contexts.bin");
        let protected_path = dir.path().join("credentials.bin");

        let code = sample_join_code();
        let other_code = sample_join_code();
        let leaving_space = SpaceId::Flock(FlockId([1; 32]));
        let second_leaving_space = SpaceId::Flock(FlockId([2; 32]));
        let kept_space = SpaceId::Flock(FlockId([3; 32]));

        // Two contexts share the same join code (e.g. a roost's channels), one
        // carries a different code, and one legacy context has no secret.
        save_public(
            &state_path,
            &PublicState {
                contexts: vec![
                    ContextDescriptor {
                        space: leaving_space,
                        label: "leaving-a".into(),
                        secret: Some(code.clone()),
                    },
                    ContextDescriptor {
                        space: second_leaving_space,
                        label: "leaving-b".into(),
                        secret: Some(code.clone()),
                    },
                    ContextDescriptor {
                        space: kept_space,
                        label: "kept".into(),
                        secret: Some(other_code.clone()),
                    },
                    ContextDescriptor {
                        space: SpaceId::Flock(FlockId([4; 32])),
                        label: "legacy".into(),
                        secret: None,
                    },
                ],
                active_space: Some(leaving_space),
            },
        )
        .unwrap();

        let report = leave_context(&state_path, &protected_path, &code).unwrap();
        assert_eq!(report.removed, 2);

        let after = crate::persistence::load_public(&state_path).unwrap();
        assert_eq!(after.contexts.len(), 2);
        assert!(
            after
                .contexts
                .iter()
                .all(|ctx| ctx.secret.as_deref() != Some(code.as_str()))
        );
        assert!(after.contexts.iter().any(
            |ctx| ctx.space == kept_space && ctx.secret.as_deref() == Some(other_code.as_str())
        ));
        // active_space pointed at a removed context and must be cleared.
        assert_eq!(after.active_space, None);
    }

    #[test]
    fn leave_with_no_state_file_reports_zero() {
        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("contexts.bin");
        let protected_path = dir.path().join("credentials.bin");
        let code = sample_join_code();

        let report = leave_context(&state_path, &protected_path, &code).unwrap();
        assert_eq!(report.removed, 0);
        assert!(!state_path.exists());
    }

    #[test]
    fn leave_drops_credentials_keyed_by_the_code() {
        use crate::persistence::{
            Credential, ProtectedSecretState, load_protected, save_protected,
        };

        let dir = tempfile::tempdir().unwrap();
        let state_path = dir.path().join("contexts.bin");
        let protected_path = dir.path().join("credentials.bin");
        let code = sample_join_code();

        save_protected(
            &protected_path,
            &ProtectedSecretState {
                credentials: vec![
                    Credential {
                        name: code.clone(),
                        secret: vec![1; 32],
                    },
                    Credential {
                        name: "unrelated".into(),
                        secret: vec![2; 32],
                    },
                ],
            },
        )
        .unwrap();

        leave_context(&state_path, &protected_path, &code).unwrap();

        let after = load_protected(&protected_path).unwrap();
        assert_eq!(after.credentials.len(), 1);
        assert_eq!(after.credentials[0].name, "unrelated");
    }

    #[test]
    fn settings_mouse_click_selects_tabs_and_consumes_clicks() {
        let mut app = App {
            settings_open: true,
            ..App::default()
        };
        // Settings modal: 80x20 centered on a 120x30 terminal.
        let modal = ratatui::layout::Rect {
            x: 20,
            y: 5,
            width: 80,
            height: 20,
        };
        // Click the third nav row -> Appearance tab.
        assert!(handle_settings_mouse_click(
            &mut app,
            modal,
            modal.x + 2,
            modal.y + 3
        ));
        assert_eq!(app.settings_tab, SettingsTab::Appearance);
        // Click the first nav row -> Account tab.
        assert!(handle_settings_mouse_click(
            &mut app,
            modal,
            modal.x + 2,
            modal.y + 1
        ));
        assert_eq!(app.settings_tab, SettingsTab::Account);
        // Any click inside the modal is consumed (returns true).
        assert!(handle_settings_mouse_click(
            &mut app,
            modal,
            modal.x + 40,
            modal.y + 10
        ));
    }
    #[test]
    fn right_click_on_rail_roost_opens_menu() {
        let mut app = App::default();
        app.node_id = Some(iroh::SecretKey::generate().public());
        app.roosts.push(RoostView {
            code: "R1".into(),
            name: "Hearthhome".into(),
            channels: vec![FlockView {
                code: "R1/general".into(),
                name: "general".into(),
                messages: vec![],
                unread: 0,
            }],
            unread: 0,
            icon_path: None,
        });
        app.roost_owners.insert("R1".into(), app.node_id.unwrap());
        // Rail: home pill rows 1-3, roost 0 at rows 5-7.
        handle_right_click(&mut app, 4, 6).unwrap();
        assert!(app.show_context_menu, "right-click on roost must open menu");
        let labels: Vec<&str> = app
            .context_menu_items
            .iter()
            .map(|i| i.label.as_str())
            .collect();
        assert!(
            labels.contains(&"Leave Roost"),
            "menu missing Leave: {labels:?}"
        );
    }

    #[test]
    fn left_click_on_rail_roost_selects_it() {
        let mut app = App::default();
        app.roosts.push(RoostView {
            code: "R1".into(),
            name: "Hearthhome".into(),
            channels: vec![FlockView {
                code: "R1/general".into(),
                name: "general".into(),
                messages: vec![],
                unread: 0,
            }],
            unread: 0,
            icon_path: None,
        });
        // Rail: roost 0 at rows 5-7, col 4.
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let handled = handle_reference_mouse_click(&mut app, &tx, 4, 6, 120, 30);
        assert!(handled, "rail click must be handled");
        assert!(
            matches!(app.selection, Selection::Channel(0, 0)),
            "roost must be selected"
        );
        assert!(
            matches!(app.v2_view, V2View::Space),
            "view must switch to Space"
        );
    }

    #[test]
    fn popups_are_draggable_by_their_title_row() {
        let mut app = App::default();
        app.show_menu = true;
        // Centered menu on 100x40, then dragged +10/+5.
        let centered = app.popup_rect(100, 40, 28, MENU_ITEMS.len() as u16 + 2);
        app.popup_offset = (10, 5);
        let moved = app.popup_rect(100, 40, 28, MENU_ITEMS.len() as u16 + 2);
        assert_eq!(moved.x, centered.x + 10);
        assert_eq!(moved.y, centered.y + 5);
        // The title row (excluding the X button) is the drag handle.
        assert!(app.popup_title_row(100, 40, moved.x + 2, moved.y));
        assert!(!app.popup_title_row(100, 40, moved.x + 2, moved.y + 1));
        // The X button region is not a drag handle.
        assert!(!app.popup_title_row(100, 40, moved.right() - 1, moved.y));
        // Offsets clamp so the title row stays on screen.
        app.popup_offset = (1000, 1000);
        let clamped = app.popup_rect(100, 40, 28, MENU_ITEMS.len() as u16 + 2);
        assert_eq!(clamped.x, 100 - 28);
        assert_eq!(clamped.y, 40 - (MENU_ITEMS.len() as u16 + 2));
        // Negative offsets (dragging left/up) must not overflow u16.
        app.popup_offset = (-1000, -1000);
        let clamped = app.popup_rect(100, 40, 28, MENU_ITEMS.len() as u16 + 2);
        assert_eq!(clamped.x, 0);
        assert_eq!(clamped.y, 0);
    }

    #[test]
    fn notifications_and_profile_windows_are_draggable() {
        let mut app = App::default();
        // Notifications: title row is a drag handle at the reported rect.
        app.show_notifications = true;
        app.notifications.push(crate::ui::NotificationItem {
            space_name: "general".into(),
            author: "Wren".into(),
            body: "hi".into(),
            ts: 1,
        });
        let rect = crate::ui::active_popup_rect(&app, 100, 40).unwrap();
        assert!(app.popup_title_row(100, 40, rect.x + 2, rect.y));
        assert!(!app.popup_title_row(100, 40, rect.x + 2, rect.y + 1));
        // Profile modal: same, and the drag offset moves it.
        app.show_notifications = false;
        app.profile_panel.open = true;
        let rect = crate::ui::active_popup_rect(&app, 100, 40).unwrap();
        assert!(app.popup_title_row(100, 40, rect.x + 2, rect.y));
        app.popup_offset = (7, 3);
        let moved = crate::ui::active_popup_rect(&app, 100, 40).unwrap();
        assert_eq!(moved.x, rect.x + 7);
        assert_eq!(moved.y, rect.y + 3);
    }

    #[test]
    fn popup_action_buttons_are_clickable() {
        let mut app = App::default();
        app.show_create_room = true;
        app.create_flock_name = "crew".into();
        let popup = Rect::new((100 - 60) / 2, (40 - 8) / 2, 60, 8);
        // The primary (Create) button is the rightmost button on the bottom row.
        let inner = popup.inner(Margin {
            vertical: 1,
            horizontal: 2,
        });
        let y = inner.bottom().saturating_sub(1);
        let create_x = inner.right().saturating_sub(8); // " Create "
        assert_eq!(
            popup_action_button(&app, popup, create_x, y),
            Some(1),
            "Create button must be clickable"
        );
        // Cancel sits left of Create with a 1-cell gap.
        let cancel_x = create_x.saturating_sub(9);
        assert_eq!(
            popup_action_button(&app, popup, cancel_x, y),
            Some(0),
            "Cancel button must be clickable"
        );
        assert_eq!(
            popup_action_button(&app, popup, inner.x + 1, y - 1),
            None,
            "clicks above the button row are not buttons"
        );
    }

    #[test]
    fn composer_send_button_is_clickable() {
        let mut app = App::default();
        app.v2_view = V2View::Space;
        app.input = "hello".into();
        // Send is the rightmost control: right-aligned at the composer's
        // right edge (col 117 for a 120-wide terminal).
        let hit = crate::ui::composer_hit_at(&app, 117, 28, 120, 30);
        assert_eq!(hit, Some(crate::ui::ComposerHit::Send));
        let input = crate::ui::composer_hit_at(&app, 50, 28, 120, 30);
        assert_eq!(input, Some(crate::ui::ComposerHit::Input));
        // Above the composer: not a composer hit.
        assert_eq!(crate::ui::composer_hit_at(&app, 50, 20, 120, 30), None);
    }
}
