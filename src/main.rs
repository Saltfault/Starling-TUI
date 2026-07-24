mod call;
mod event;
mod net;
#[cfg(feature = "audio")]
mod opus_ffi;
#[cfg(feature = "audio")]
mod playback;
mod setup;
mod sync;
mod ui;
mod video;
#[cfg(feature = "audio")]
mod voice;

#[allow(unused_imports)]
use crossterm::{
    event::{
        self as ct_event, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::*,
};
use event::{AppEvent, Command};
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use ui::{App, FlockView, MENU_ITEMS, RoostView, Selection};

struct TerminalCleanup {
    mouse: bool,
}

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = std::io::stdout();
        if self.mouse {
            let _ = execute!(stdout, LeaveAlternateScreen, ct_event::DisableMouseCapture);
        } else {
            let _ = execute!(stdout, LeaveAlternateScreen);
        }
    }
}

fn apply_profile(app: &mut App, profile: &starling::config::Profile) {
    app.name.clone_from(&profile.name);
    app.pronouns.clone_from(&profile.pronouns);
    if let Some(color) = ui::hex_to_color(&profile.text_color) {
        app.text_color = color;
    }
    if let Some(color) = ui::hex_to_color(&profile.border_color) {
        app.border_color = color;
    }
    app.bg_color = if profile.bg_color.is_empty() {
        None
    } else {
        ui::hex_to_color(&profile.bg_color)
    };
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
            .map(|message| message.id.clone())
            .collect();
        let mut fresh: Vec<_> = old
            .into_iter()
            .filter(|message| !known.contains(&message.id))
            .collect();
        fresh.extend(std::mem::take(&mut view.messages));
        fresh.sort_by_key(|message| message.ts);
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
    starling::logger::init();

    let args: Vec<String> = std::env::args().collect();
    let first = args.get(1).map(String::as_str);

    if first == Some("--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if first == Some("profile") {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let _cleanup = TerminalCleanup { mouse: false };
        let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
        setup::run_setup(&mut term)?;
        return Ok(());
    }

    if first == Some("settings") {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let _cleanup = TerminalCleanup { mouse: false };
        let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
        setup::run_settings(&mut term)?;
        return Ok(());
    }

    let bootstrap = match first {
        Some("join") => {
            let Some(code) = args.get(2).map(|code| code.trim()) else {
                eprintln!("Usage: starling-tui join <code>");
                return Ok(());
            };
            match starling::net::decode_node_id(code) {
                Some(node_id) => vec![node_id],
                None => {
                    eprintln!("Invalid join code.");
                    return Ok(());
                }
            }
        }
        _ => vec![],
    };

    let profile = starling::config::Profile::load();

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, ct_event::EnableMouseCapture)?;
    let _cleanup = TerminalCleanup { mouse: true };
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
    let mut app = App::default();

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
    #[allow(unused)]
    let input_device = profile.input_device.clone();
    #[allow(unused)]
    let output_device = profile.output_device.clone();
    apply_profile(&mut app, &profile);

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Command>();
    let (evt_tx, mut evt_rx) = mpsc::unbounded_channel::<AppEvent>();
    #[allow(unused)]
    let muted_flag = Arc::new(AtomicBool::new(false));

    let mut net_task = tokio::spawn(net::run(
        bootstrap,
        cmd_rx,
        evt_tx,
        muted_flag.clone(),
        my_node_id,
        name,
        input_device,
    ));

    #[cfg(feature = "audio")]
    let mut playback = match crate::playback::Playback::new(output_device.as_deref()) {
        Ok(p) => Some(p),
        Err(e) => {
            starling::logger::warn(&format!("audio playback unavailable: {e}"));
            None
        }
    };

    loop {
        if net_task.is_finished() {
            match (&mut net_task).await {
                Ok(Ok(())) if app.quit_requested => break,
                Ok(Ok(())) => anyhow::bail!("network task stopped unexpectedly"),
                Ok(Err(error)) => return Err(error.context("network task failed")),
                Err(error) => {
                    return Err(anyhow::Error::new(error).context("network task panicked"));
                }
            }
        }
        term.draw(|f| ui::draw(f, &app))?;

        while let Ok(ev) = evt_rx.try_recv() {
            match ev {
                AppEvent::Message { flock, msg } => {
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
                        fv.messages.push(msg);
                        if !is_current {
                            fv.unread += 1;
                        }
                    }
                    for roost in &mut app.roosts {
                        roost.unread = roost.channels.iter().map(|channel| channel.unread).sum();
                    }
                }
                AppEvent::JoinedFlock { code } => {
                    if app.flocks.iter().any(|flock| flock.code == code) {
                        continue;
                    }
                    app.flocks.push(FlockView {
                        code,
                        name: String::new(),
                        messages: vec![],
                        unread: 0,
                    });
                }
                AppEvent::JoinedRoost {
                    code,
                    name,
                    channels,
                } => {
                    if app.roosts.iter().any(|roost| roost.code == code) {
                        continue;
                    }
                    app.roosts.push(RoostView {
                        code: code.clone(),
                        name,
                        channels: channels
                            .into_iter()
                            .map(|channel| FlockView {
                                code: format!("{code}/{channel}"),
                                name: channel,
                                messages: vec![],
                                unread: 0,
                            })
                            .collect(),
                        unread: 0,
                    });
                }
                AppEvent::RoostUpdate {
                    code,
                    name,
                    channels,
                } => {
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
                AppEvent::PeerDisconnected(id) => {
                    app.peers.retain(|p| p != &id);
                    app.peer_names.remove(&id);
                    app.peer_status.remove(&id);
                    if !app.peers.is_empty() {
                        app.selected_peer %= app.peers.len();
                    } else {
                        app.selected_peer = 0;
                    }
                }
                AppEvent::PeerNamed(id, name) => {
                    app.peer_names.insert(id, name);
                }
                AppEvent::PeerStatus(id, s) => {
                    app.peer_status.insert(id, s);
                }
                AppEvent::Ticket(code) => {
                    app.node_id = Some(code);
                }
                AppEvent::Error(error) => {
                    starling::logger::warn(&error);
                    app.error_message = Some(error);
                    app.in_call = false;
                    app.show_video = false;
                    app.video_frame = None;
                }
                #[cfg(feature = "audio")]
                AppEvent::VoiceFrame(bytes) => {
                    if let Some(p) = &mut playback {
                        p.push_opus(&bytes);
                    }
                }
                #[cfg(feature = "video")]
                AppEvent::VideoFrame(jpeg) => {
                    if let Ok(img) = image::load_from_memory(&jpeg) {
                        app.video_frame = Some(img.to_rgb8());
                    }
                }
                AppEvent::HistoryChunk { flock, messages } => {
                    merge_history(&mut app, &flock, messages);
                }
            }
        }

        if ct_event::poll(std::time::Duration::from_millis(50))? {
            let event = ct_event::read()?;

            if let Event::Key(k) = &event {
                if k.kind != KeyEventKind::Press {
                    continue;
                }

                if app.show_invite {
                    if k.code == KeyCode::Esc {
                        app.show_invite = false;
                    }
                    continue;
                }

                if app.show_create_room {
                    match k.code {
                        KeyCode::Enter => {
                            if let Some(code) = &app.node_id {
                                let _ = cmd_tx.send(Command::JoinFlock { code: code.clone() });
                            }
                            app.show_create_room = false;
                        }
                        KeyCode::Esc => {
                            app.show_create_room = false;
                        }
                        _ => {}
                    }
                    continue;
                }

                if app.show_join_room {
                    match k.code {
                        KeyCode::Enter => {
                            let code = app.join_input.trim();
                            if starling::net::decode_node_id(code).is_some() {
                                let _ = cmd_tx.send(Command::JoinFlock { code: code.into() });
                                app.join_input.clear();
                                app.show_join_room = false;
                                app.error_message = None;
                            } else {
                                app.error_message = Some("Invalid flock code".into());
                            }
                        }
                        KeyCode::Char(c) if !c.is_control() => {
                            app.join_input.push(c);
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

                if app.show_join_roost {
                    match k.code {
                        KeyCode::Enter => {
                            let code = app.join_roost_input.trim();
                            if starling::net::decode_node_id(code).is_some() {
                                let _ = cmd_tx.send(Command::JoinRoost { code: code.into() });
                                app.join_roost_input.clear();
                                app.show_join_roost = false;
                                app.error_message = None;
                            } else {
                                app.error_message = Some("Invalid roost code".into());
                            }
                        }
                        KeyCode::Char(c) if !c.is_control() => {
                            app.join_roost_input.push(c);
                        }
                        KeyCode::Backspace => {
                            app.join_roost_input.pop();
                        }
                        KeyCode::Esc => {
                            app.show_join_roost = false;
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
                            activate_menu_item(&mut app, &cmd_tx, &muted_flag)?;
                        }
                        KeyCode::Esc => {
                            app.show_menu = false;
                        }
                        _ => {}
                    }
                    continue;
                }

                if app.show_create_roost {
                    match k.code {
                        KeyCode::Enter if !app.create_roost_input.is_empty() => {
                            let name = std::mem::take(&mut app.create_roost_input);
                            let _ = std::process::Command::new("starling")
                                .args(["roost", "create", &name])
                                .spawn()
                                .map(|mut child| {
                                    let _ = child.wait();
                                });
                            app.show_create_roost = false;
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
                    continue;
                }

                match k.code {
                    KeyCode::Enter if !app.input.is_empty() => {
                        let text = std::mem::take(&mut app.input);
                        if let Some(code) = text.strip_prefix("/join-roost ") {
                            let code = code.trim();
                            if starling::net::decode_node_id(code).is_some() {
                                let _ = cmd_tx.send(Command::JoinRoost { code: code.into() });
                            } else {
                                app.error_message = Some("Invalid roost code".into());
                            }
                        } else if let Some(code) = text.strip_prefix("/join ") {
                            let code = code.trim();
                            if starling::net::decode_node_id(code).is_some() {
                                let _ = cmd_tx.send(Command::JoinFlock { code: code.into() });
                            } else {
                                app.error_message = Some("Invalid flock code".into());
                            }
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

                    KeyCode::Esc => {
                        app.show_menu = true;
                        app.menu_selection = 0;
                    }

                    KeyCode::Char(c) if !c.is_control() => {
                        app.input.push(c);
                    }

                    KeyCode::Backspace => {
                        app.input.pop();
                    }

                    _ => {}
                }
            } else if let Event::Mouse(m) = event {
                match m.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        handle_mouse_click(
                            &mut app,
                            &cmd_tx,
                            &muted_flag,
                            &mut term,
                            m.column,
                            m.row,
                        )?;
                    }
                    MouseEventKind::Moved if app.show_menu => {
                        if let Some(idx) = menu_item_at(m.column, m.row) {
                            app.menu_selection = idx;
                        }
                    }
                    _ => {}
                }
            }
        }

        if app.quit_requested {
            let _ = cmd_tx.send(Command::Quit);
            tokio::time::sleep(Duration::from_millis(500)).await;
            break;
        }
    }

    disable_raw_mode()?;
    execute!(
        term.backend_mut(),
        LeaveAlternateScreen,
        ct_event::DisableMouseCapture
    )?;
    Ok(())
}

#[allow(unused_variables)]
fn handle_mouse_click(
    app: &mut App,
    cmd_tx: &mpsc::UnboundedSender<Command>,
    muted_flag: &Arc<AtomicBool>,
    _term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    col: u16,
    row: u16,
) -> anyhow::Result<()> {
    let (term_w, term_h) = crossterm::terminal::size()?;

    if app.show_menu {
        if let Some(idx) = menu_item_at_size(term_w, term_h, col, row) {
            app.menu_selection = idx;
            activate_menu_item(app, cmd_tx, muted_flag)?;
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

    let button_bar_y = term_h.saturating_sub(4);
    if row == button_bar_y {
        let btns = ui::toolbar_buttons();
        for (i, (_label, bx, bw)) in btns.iter().enumerate() {
            if col >= *bx && col < bx + bw {
                match i {
                    0 => {
                        app.show_create_room = true;
                    }
                    1 => {
                        app.join_input.clear();
                        app.show_join_room = true;
                    }
                    2 => {
                        app.show_menu = true;
                        app.menu_selection = 0;
                    }
                    3 => {
                        app.quit_requested = true;
                    }
                    _ => {}
                }
                return Ok(());
            }
        }
        return Ok(());
    }

    if col < 26 {
        let body_top = 2u16;
        let body_h = term_h.saturating_sub(6);
        let flocks_h = (body_h * 33) / 100;
        let roosts_h = body_h.saturating_sub(flocks_h);

        let flocks_top = body_top;
        let roosts_top = body_top + flocks_h;

        if row > flocks_top && row < flocks_top + flocks_h.saturating_sub(1) {
            let idx = (row - flocks_top - 1) as usize;
            if app.flocks.get(idx).is_some() {
                app.select(Selection::Flock(idx));
            }
        } else if row > roosts_top && row < roosts_top + roosts_h.saturating_sub(1) {
            let mut cursor = roosts_top + 1;
            for (ri, rv) in app.roosts.iter().enumerate() {
                if cursor == row {
                    app.toggle_expand(ri);
                    return Ok(());
                }
                cursor += 1;
                if app.expanded.contains(&ri) {
                    for ci in 0..rv.channels.len() {
                        if cursor == row {
                            app.select(Selection::Channel(ri, ci));
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

fn menu_item_at(col: u16, row: u16) -> Option<usize> {
    let (term_w, term_h) = crossterm::terminal::size().ok()?;
    menu_item_at_size(term_w, term_h, col, row)
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

#[allow(unused_variables)]
fn activate_menu_item(
    app: &mut App,
    cmd_tx: &mpsc::UnboundedSender<Command>,
    muted_flag: &Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let i = app.menu_selection;
    if i >= MENU_ITEMS.len() {
        return Ok(());
    }

    app.show_menu = false;

    match i {
        0 => {
            app.show_create_room = true;
        }
        1 => {
            app.join_input.clear();
            app.show_join_room = true;
        }
        2 => {
            app.join_roost_input.clear();
            app.show_join_roost = true;
        }
        3 => {
            app.create_roost_input.clear();
            app.show_create_roost = true;
        }
        4 => {
            app.show_invite = app.active_code().is_some();
        }
        5 => {
            #[cfg(feature = "audio")]
            {
                app.muted = !app.muted;
                muted_flag.store(app.muted, Ordering::Relaxed);
            }
        }
        6 => {
            #[cfg(feature = "video")]
            {
                app.show_video = !app.show_video;
                match (app.show_video, app.selected_peer_addr()) {
                    (true, Some(addr)) => {
                        let _ = cmd_tx.send(Command::StartVideo(addr));
                    }
                    _ => {
                        let _ = cmd_tx.send(Command::StopVideo);
                    }
                }
            }
        }
        7 => {
            #[cfg(feature = "audio")]
            {
                if app.in_call {
                    let _ = cmd_tx.send(Command::HangUp);
                    app.in_call = false;
                } else if let Some(addr) = app.selected_peer_addr() {
                    let _ = cmd_tx.send(Command::StartCall(addr));
                    app.in_call = true;
                }
            }
        }
        8 => {
            disable_raw_mode()?;
            execute!(
                std::io::stdout(),
                LeaveAlternateScreen,
                ct_event::DisableMouseCapture
            )?;
            let editor_result = std::process::Command::new(std::env::current_exe()?)
                .arg("profile")
                .status();
            execute!(
                std::io::stdout(),
                EnterAlternateScreen,
                ct_event::EnableMouseCapture
            )?;
            enable_raw_mode()?;
            if editor_result.is_ok_and(|status| status.success()) {
                if let Some(profile) = starling::config::Profile::load() {
                    apply_profile(app, &profile);
                    let _ = cmd_tx.send(Command::UpdateProfile {
                        name: profile.name,
                        input_device: profile.input_device,
                    });
                }
            } else {
                app.error_message = Some("Profile editor failed".into());
            }
        }
        9 => {
            disable_raw_mode()?;
            execute!(
                std::io::stdout(),
                LeaveAlternateScreen,
                ct_event::DisableMouseCapture
            )?;
            let editor_result = std::process::Command::new(std::env::current_exe()?)
                .arg("settings")
                .status();
            execute!(
                std::io::stdout(),
                EnterAlternateScreen,
                ct_event::EnableMouseCapture
            )?;
            enable_raw_mode()?;
            if editor_result.is_ok_and(|status| status.success()) {
                if let Some(profile) = starling::config::Profile::load() {
                    apply_profile(app, &profile);
                    let _ = cmd_tx.send(Command::UpdateProfile {
                        name: profile.name,
                        input_device: profile.input_device,
                    });
                }
            } else {
                app.error_message = Some("Settings editor failed".into());
            }
        }
        10 => {
            app.quit_requested = true;
        }
        _ => {}
    }

    Ok(())
}
