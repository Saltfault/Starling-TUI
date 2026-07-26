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
        self as ct_event, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    style::Print,
    terminal::*,
};
use event::{AppEvent, Command};
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::mpsc;
use ui::{App, FlockView, MENU_ITEMS, RoostView, ScrollPanel, Selection, ToolbarAction};

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
        None
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
        app.error_message = Some("Select a flock to edit".into());
        return false;
    };
    app.edit_flock_code = code;
    app.edit_flock_name = app
        .flocks
        .iter()
        .find(|f| f.code == app.edit_flock_code)
        .map(|f| f.name.clone())
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
    let state_path = starling::config::Profile::config_dir()
        .join("public")
        .join("contexts.bin");
    let protected_path = starling::config::Profile::config_dir()
        .join("protected")
        .join("credentials.bin");
    if let Err(error) = persistence::recover(&state_path) {
        app.error_message = Some(format!("Could not recover saved contexts: {error}"));
    } else if state_path.exists() {
        match persistence::load_public(&state_path) {
            Ok(saved) => {
                for descriptor in saved.contexts {
                    app.insert_context(ui::ContextView {
                        id: descriptor.space,
                        title: descriptor.label,
                        roost: match descriptor.space {
                            starling::protocol::SpaceId::RoostChannel { roost, .. } => Some(roost),
                            starling::protocol::SpaceId::Flock(_) => None,
                        },
                        base_invite_display: None,
                        messages: Vec::new(),
                        unread: 0,
                        state: ui::ContextState::Restoring,
                        secret: descriptor.secret,
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

    let secret = starling::config::Profile::load_or_create_secret();
    let my_node_id: iroh::EndpointId = secret.public();

    let profile = match profile {
        Some(profile) => profile,
        None => match setup::run_setup(&mut term)? {
            Some(profile) => profile,
            None => return Ok(()),
        },
    };

    let name = profile.name.clone();
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
            let (_, flock_h, _, roost_h, bird_h) = panel_geometry(crossterm::terminal::size()?.1);
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
                        let is_current = app.active_code().is_some_and(|code| code == flock);
                        if let Some(fv) =
                            app.flocks
                                .iter_mut()
                                .find(|fv| fv.code == flock)
                                .or_else(|| {
                                    app.roosts
                                        .iter_mut()
                                        .flat_map(|roost| roost.channels.iter_mut())
                                        .find(|channel| channel.code == flock)
                                })
                        {
                            fv.messages.push(ui::MessageView { msg, private });
                            if !is_current {
                                fv.unread += 1;
                            }
                        }
                        for roost in &mut app.roosts {
                            roost.unread = roost.channels.iter().map(|channel| channel.unread).sum();
                        }
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
                            app.insert_context(ui::ContextView {
                                id: space_id,
                                title: flock_code.name.clone(),
                                roost: None,
                                base_invite_display: None,
                                messages: Vec::new(),
                                unread: 0,
                                state: ui::ContextState::Ready,
                                secret: Some(code.clone()),
                            });
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
                        if app.roosts.iter().any(|roost| roost.code == code) {
                            continue;
                        }
                        app.apply_roost_perms(&perms);
                        let roost_channels: Vec<(String, String)> = channels
                            .into_iter()
                            .map(|channel| {
                                let channel_code = format!("{code}/{channel}");
                                (channel, channel_code)
                            })
                            .collect();
                        app.roosts.push(RoostView {
                            code: code.clone(),
                            name,
                            channels: roost_channels
                                .iter()
                                .map(|(name, code)| FlockView {
                                    code: code.clone(),
                                    name: name.clone(),
                                    messages: vec![],
                                    unread: 0,
                                })
                                .collect(),
                            unread: 0,
                        });
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
                                    base_invite_display: None,
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
                        app.apply_roost_perms(&perms);
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
                    AppEvent::PeerConnected(id) => {
                        if !app.peers.contains(&id) {
                            app.peers.push(id);
                        }
                    }
                    AppEvent::PeerConnectivityHintDown(id) => {
                        // Signed presence leases, not transport neighbors, determine liveness.
                        app.presence.neighbor_down(id);
                    }

                    AppEvent::PeerNamed(id, name) => {
                        if id != my_node_id && !app.peers.contains(&id) {
                            app.peers.push(id);
                        }
                        app.peer_names.insert(id, name.clone());
                        // Track profile for presence display across all contexts.
                        for ctx in app.presence.contexts.values_mut() {
                            ctx.set_profile(ui::MemberProfile {
                                endpoint: id,
                                name: name.clone(),
                                pronouns: String::new(),
                            });
                        }
                    }
                    AppEvent::PeerStatus(id, s) => {
                        if id != my_node_id && !app.peers.contains(&id) {
                            app.peers.push(id);
                        }
                        app.peer_status.insert(id, s);
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

                    if app.show_delete_confirm {
                        match k.code {
                            KeyCode::Enter => {
                                if app.delete_confirm_input.trim() == "DELETE" {
                                    app.show_delete_confirm = false;
                                    let dir = starling::config::Profile::config_dir();
                                    if dir.exists() {
                                        if let Err(error) = std::fs::remove_dir_all(&dir) {
                                            app.error_message =
                                                Some(format!("Failed to delete data: {error}"));
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
                        continue;
                    }

                    if app.show_create_room {
                        match k.code {
                            KeyCode::Enter if app.create_flock_code.is_some() => {
                                if let Some(code) = app.create_flock_code.take() {
                                    let since = app.newest_ts(&code).unwrap_or(0);
                                    let _ = cmd_tx.send(Command::Join { code, since });
                                }
                                app.create_flock_secret = None;
                                app.show_create_room = false;
                            }
                            KeyCode::Char(c) => {
                                app.create_flock_name =
                                    sanitize::sanitize_name(&format!("{}{}", app.create_flock_name, c));
                                refresh_create_flock_code(&mut app);
                            }
                            KeyCode::Backspace => {
                                app.create_flock_name.pop();
                                refresh_create_flock_code(&mut app);
                            }
                            KeyCode::Esc => {
                                app.create_flock_code = None;
                                app.create_flock_secret = None;
                                app.create_flock_name.clear();
                                app.show_create_room = false;
                            }
                            _ => {}
                        }
                        continue;
                    }

                    if app.show_edit_flock {
                        match k.code {
                            KeyCode::Enter => {
                                let name = std::mem::take(&mut app.edit_flock_name);
                                let code = std::mem::take(&mut app.edit_flock_code);
                                app.show_edit_flock = false;
                                if !name.is_empty() {
                                    let _ = cmd_tx.send(Command::UpdateProfile {
                                        name: format!("flock:{code}:{name}"),
                                        input_device: None,
                                        camera_index: None,
                                    });
                                }
                            }
                            KeyCode::Backspace => {
                                app.edit_flock_name.pop();
                            }
                            KeyCode::Char(c) => {
                                app.edit_flock_name =
                                    sanitize::sanitize_name(&format!("{}{}", app.edit_flock_name, c));
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
                        continue;
                    }

                    if app.show_join_room {
                        match k.code {
                            KeyCode::Enter => match sanitize::invite(app.join_input.trim()) {
                                Ok(code) => {
                                    let since = app.newest_ts(&code).unwrap_or(0);
                                    let _ = cmd_tx.send(Command::Join { code: code.clone(), since });
                                    app.joining = Some(code);
                                    app.join_input.clear();
                                    app.show_join_room = false;
                                    app.error_message = None;
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
                        continue;
                    }

                    if app.show_menu {
                        match k.code {
                            KeyCode::Up => {
                                app.menu_selection = app.menu_selection.saturating_sub(1);
                            }
                            KeyCode::Down => {
                                app.menu_selection = (app.menu_selection + 1).min(MENU_ITEMS.len() - 1);
                            }
                            KeyCode::Enter => {
                                activate_menu_item(&mut app, &cmd_tx, &mut term)?;
                            }
                            KeyCode::Esc => {
                                app.show_menu = false;
                            }
                            _ => {}
                        }
                        continue;
                    }

                    match k.code {
                        KeyCode::Enter if !app.input.is_empty() => {
                            let text = std::mem::take(&mut app.input);
                            if let Some(code) = text
                                .strip_prefix("/join ")
                                .or_else(|| text.strip_prefix("/join-roost "))
                            {
                                let code = code.trim();
                                match sanitize::invite(code) {
                                    Ok(normalized) => {
                                        let since = app.newest_ts(&normalized).unwrap_or(0);
                                        let _ = cmd_tx.send(Command::Join { code: normalized, since });
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
                                        app.error_message =
                                            Some("Usage: /chirp <name> <message>".into());
                                        continue;
                                    }
                                };
                                // Match the destination by display name (set by an
                                // authenticated profile announcement) and resolve it
                                // to a verified endpoint. Without the endpoint's
                                // published DM public key, we can't seal a chirp —
                                // the recipient hasn't yet published a Phase 9
                                // Profile with `dm_pk`.
                                let to = app
                                    .peer_names
                                    .iter()
                                    .find(|(_, peer_name)| peer_name.trim() == name)
                                    .and_then(|(id, _)| app.peer_dm_keys.get(id).map(|_| *id));
                                let to = match to {
                                    Some(to) => to,
                                    None => {
                                        app.error_message =
                                            Some(format!("{name:?} hasn't published a DM key yet"));
                                        continue;
                                    }
                                };
                                let Some(code) = app.active_code() else {
                                    app.error_message = Some("Select a flock first".into());
                                    continue;
                                };
                                let their_pk = match app.peer_dm_keys.get(&to).cloned() {
                                    Some(pk) => pk,
                                    None => continue,
                                };
                                let _ = cmd_tx.send(Command::SendChirp {
                                    flock: code.to_string(),
                                    to,
                                    their_pk,
                                    body: body.to_string(),
                                });
                            } else if let Some(_space) = app.active {
                                // V1 typed-context send isn't wired yet — don't revoke, don't clear input.
                                app.input = text;
                                app.error_message = Some("V1 messaging isn't available yet".into());
                            } else if let Some(code) = app.active_code() {
                                let _ = cmd_tx.send(Command::SendText {
                                    flock: code.to_string(),
                                    body: text,
                                });
                            }
                        }

                        KeyCode::Up if k.modifiers.contains(KeyModifiers::ALT) => {
                            let nav = nav_items(&app);
                            if let Some(pos) = nav.iter().position(|s| *s == app.selection)
                                && pos > 0
                            {
                                app.select(nav[pos - 1]);
                            }
                        }
                        KeyCode::Down if k.modifiers.contains(KeyModifiers::ALT) => {
                            let nav = nav_items(&app);
                            if let Some(pos) = nav.iter().position(|s| *s == app.selection)
                                && pos + 1 < nav.len()
                            {
                                app.select(nav[pos + 1]);
                            }
                        }
                        KeyCode::Right if k.modifiers.contains(KeyModifiers::ALT) => {
                            match app.selection {
                                Selection::Flock(_) => {}
                                Selection::Channel(ri, _) => {
                                    app.toggle_expand(ri);
                                }
                            }
                        }
                        KeyCode::Left if k.modifiers.contains(KeyModifiers::ALT) => {
                            match app.selection {
                                Selection::Flock(_) => {}
                                Selection::Channel(ri, _) => {
                                    app.toggle_expand(ri);
                                }
                            }
                        }

                        KeyCode::PageUp => {
                            page_scroll(&mut app, -1.0, crossterm::terminal::size()?.1);
                        }
                        KeyCode::PageDown => {
                            page_scroll(&mut app, 1.0, crossterm::terminal::size()?.1);
                        }

                        KeyCode::Esc => {
                            app.show_menu = true;
                            app.menu_selection = 0;
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

                        KeyCode::Char(c) => {
                            app.input = sanitize::sanitize_message(&format!("{}{}", app.input, c));
                        }

                        KeyCode::Backspace => {
                            app.input.pop();
                        }

                        _ => {}
                    }
                } else if let Event::Mouse(m) = event {
                    if app.show_menu {
                        let (term_w, term_h) = crossterm::terminal::size()?;
                        update_menu_hover(&mut app, term_w, term_h, m.column, m.row);
                    }
                    match m.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
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
                        MouseEventKind::Moved => {}
                        MouseEventKind::ScrollUp if !app.show_menu => {
                            handle_mouse_scroll(&mut app, m.column, m.row, -3.0)?;
                        }
                        MouseEventKind::ScrollDown if !app.show_menu => {
                            handle_mouse_scroll(&mut app, m.column, m.row, 3.0)?;
                        }
                        MouseEventKind::Down(MouseButton::Right)
                        | MouseEventKind::Up(MouseButton::Right)
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
    match clipboard.set_text(&invite) {
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
    let flocks_h = (body_h * 33) / 100;
    let roosts_top = body_top + flocks_h;
    let roosts_h = body_h.saturating_sub(flocks_h);
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
    } else if col >= term_w.saturating_sub(24) && row >= flocks_top && row < flocks_top + birds_h {
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

#[allow(unused_variables)]
fn handle_mouse_click(
    app: &mut App,
    cmd_tx: &mpsc::UnboundedSender<Command>,
    muted_flag: &Arc<AtomicBool>,
    clipboard: Option<&mut clipboard::SystemClipboard>,
    term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    col: u16,
    row: u16,
) -> anyhow::Result<()> {
    let (term_w, term_h) = crossterm::terminal::size()?;

    if app.show_menu {
        if let Some(idx) = menu_item_at_size(term_w, term_h, col, row) {
            app.menu_selection = idx;
            activate_menu_item(app, cmd_tx, term)?;
        } else {
            let popup_w = 28u16.min(term_w);
            let popup_h = (MENU_ITEMS.len() as u16 + 2).min(term_h);
            let popup_x = (term_w.saturating_sub(popup_w)) / 2;
            let popup_y = (term_h.saturating_sub(popup_h)) / 2;

            if col < popup_x
                || col >= popup_x + popup_w
                || row < popup_y
                || row >= popup_y + popup_h
            {
                app.show_menu = false;
            }
        }
        return Ok(());
    }

    if app.show_create_room {
        return Ok(());
    }

    if app.show_edit_flock {
        return Ok(());
    }

    if app.show_join_room {
        return Ok(());
    }

    if row <= 1 && app.active_code().is_some() {
        copy_active_invite(app, clipboard, Instant::now());
        return Ok(());
    }

    let button_bar_y = term_h.saturating_sub(4);
    if row == button_bar_y {
        let btns = ui::toolbar_buttons(app);
        for (action, _label, bx, bw) in btns {
            if col >= bx && col < bx + bw {
                match action {
                    ToolbarAction::Menu => {
                        app.show_menu = true;
                        app.menu_selection = 0;
                    }
                    #[cfg(feature = "audio")]
                    ToolbarAction::Call => {
                        if app.in_call {
                            if cmd_tx.send(Command::HangUp).is_ok() {
                                app.in_call = false;
                            }
                        } else if let Some(peer) = app.selected_peer_id() {
                            if cmd_tx.send(Command::StartCall(peer)).is_err() {
                                app.error_message = Some("Call service is unavailable".into());
                            } else {
                                app.error_message = Some("Connecting call...".into());
                            }
                        }
                    }
                    #[cfg(feature = "audio")]
                    ToolbarAction::Mute => {
                        app.muted = !app.muted;
                        muted_flag.store(app.muted, Ordering::Relaxed);
                    }
                    #[cfg(feature = "video")]
                    ToolbarAction::Video => {
                        if app.show_video {
                            if cmd_tx.send(Command::StopVideo).is_ok() {
                                app.show_video = false;
                                app.local_video_frame = None;
                            }
                        } else if cmd_tx.send(Command::StartVideo(app.peers.clone())).is_ok() {
                            app.show_video = true;
                            app.error_message = Some("Starting camera...".into());
                        } else {
                            app.error_message = Some("Video service is unavailable".into());
                        }
                    }
                }
                return Ok(());
            }
        }
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
            } else if app.flocks.get(idx - typed_count).is_some() {
                app.select(Selection::Flock(idx - typed_count));
            }
        }
    } else if col < 26 && row > roosts_top && row < roosts_top + roosts_h.saturating_sub(1) {
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
    } else if col >= term_w.saturating_sub(24)
        && row > flocks_top
        && row < flocks_top + birds_h.saturating_sub(1)
    {
        app.scroll_focus = ScrollPanel::Birds;
        let visible_row = (row - flocks_top - 1) as usize;
        if let Some(content_row) = app.bird_scroll.row_index(visible_row)
            && content_row > 0
            && content_row <= app.peers.len()
        {
            app.selected_peer = content_row - 1;
        }
    }

    Ok(())
}

fn update_menu_hover(app: &mut App, term_w: u16, term_h: u16, col: u16, row: u16) {
    if let Some(index) = menu_item_at_size(term_w, term_h, col, row) {
        app.menu_selection = index;
    }
}

fn menu_item_at_size(term_w: u16, term_h: u16, col: u16, row: u16) -> Option<usize> {
    let popup_w = 28u16.min(term_w);
    let popup_h = (MENU_ITEMS.len() as u16 + 2).min(term_h);
    let popup_x = (term_w.saturating_sub(popup_w)) / 2;
    let popup_y = (term_h.saturating_sub(popup_h)) / 2;

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

fn open_editor(
    app: &mut App,
    cmd_tx: &mpsc::UnboundedSender<Command>,
    term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    editor: &str,
) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(
        std::io::stdout(),
        Print(MOUSE_TRACKING_OFF),
        LeaveAlternateScreen,
        ct_event::DisableMouseCapture,
        ct_event::DisableBracketedPaste
    )?;
    let result = std::process::Command::new(std::env::current_exe()?)
        .arg(editor)
        .status();
    execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        ct_event::EnableMouseCapture,
        ct_event::EnableBracketedPaste,
        Print(MOUSE_TRACKING_ON)
    )?;
    enable_raw_mode()?;
    term.clear()?;
    if result.is_ok_and(|status| status.success()) {
        if let Some(profile) = starling::config::Profile::load() {
            apply_profile(app, &profile);
            let _ = cmd_tx.send(Command::UpdateProfile {
                name: profile.name,
                input_device: profile.input_device,
                camera_index: profile.camera_index,
            });
        }
    } else {
        app.error_message = Some(format!("{editor} editor failed"));
    }
    Ok(())
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
            if !open_edit_flock(app) {
                app.show_menu = true;
            }
        }
        2 => {
            app.join_input.clear();
            app.show_join_room = true;
        }
        3 => {
            open_editor(app, cmd_tx, term, "profile")?;
            app.show_menu = true;
        }
        4 => {
            open_editor(app, cmd_tx, term, "settings")?;
            app.show_menu = true;
        }
        5 => {
            app.show_menu = false;
            app.show_delete_confirm = true;
            app.delete_confirm_input.clear();
        }
        6 => {
            app.quit_requested = true;
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        App, MENU_ITEMS, menu_item_at_size, open_create_room, parse_join_arg,
        refresh_create_flock_code, update_menu_hover,
    };

    #[test]
    fn every_rendered_menu_row_is_clickable() {
        let (width, height) = (100, 40);
        let popup_y = (height - (MENU_ITEMS.len() as u16 + 2)) / 2;
        let popup_x = (width - 28) / 2;

        for index in 0..MENU_ITEMS.len() {
            assert_eq!(
                menu_item_at_size(width, height, popup_x + 2, popup_y + 1 + index as u16),
                Some(index)
            );
        }
        assert_eq!(menu_item_at_size(width, height, popup_x + 2, popup_y), None);
    }

    #[test]
    fn mouse_coordinates_update_menu_highlight() {
        let (width, height) = (100, 40);
        let popup_y = (height - (MENU_ITEMS.len() as u16 + 2)) / 2;
        let popup_x = (width - 28) / 2;
        let mut app = App::default();

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
}
