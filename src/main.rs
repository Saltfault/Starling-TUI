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
    event::{self as ct_event, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind, MouseButton},
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
use ui::{App, FlockView, RoostView, MENU_ITEMS, Selection};

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
        let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
        setup::run_setup(&mut term)?;
        disable_raw_mode()?;
        execute!(term.backend_mut(), LeaveAlternateScreen)?;
        return Ok(());
    }

    if first == Some("settings") {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
        setup::run_settings(&mut term)?;
        disable_raw_mode()?;
        execute!(term.backend_mut(), LeaveAlternateScreen)?;
        return Ok(());
    }

    let bootstrap = match first {
        Some("join") => {
            let code = &args[2];
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
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
    let mut app = App::default();

    let secret = starling::config::Profile::load_or_create_secret();
    let my_node_id: iroh::EndpointId = secret.public().into();

    let profile = match profile {
        Some(p) => p,
        None => match setup::run_setup(&mut term)? {
            Some(p) => p,
            None => {
                disable_raw_mode()?;
                execute!(term.backend_mut(), LeaveAlternateScreen)?;
                return Ok(());
            }
        },
    };

    let name = profile.name;
    #[allow(unused)]
    let input_device = profile.input_device;
    #[allow(unused)]
    let output_device = profile.output_device;
    app.name = name.clone();
    app.pronouns = profile.pronouns.clone();
    if let Some(c) = ui::hex_to_color(&profile.text_color) {
        app.text_color = c;
    }
    if let Some(c) = ui::hex_to_color(&profile.border_color) {
        app.border_color = c;
    }
    if !profile.bg_color.is_empty() {
        app.bg_color = ui::hex_to_color(&profile.bg_color);
    }

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Command>();
    let (evt_tx, mut evt_rx) = mpsc::unbounded_channel::<AppEvent>();
    #[allow(unused)]
    let muted_flag = Arc::new(AtomicBool::new(false));

    tokio::spawn(net::run(
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
        term.draw(|f| ui::draw(f, &app))?;

        while let Ok(ev) = evt_rx.try_recv() {
            match ev {
                AppEvent::Message { flock, msg } => {
                    let is_current = app
                        .active_code()
                        .is_some_and(|code| code == flock);
                    if let Some(fv) = app.flocks.iter_mut().find(|fv| fv.code == flock) {
                        fv.messages.push(msg);
                        if !is_current {
                            fv.unread += 1;
                        }
                    }
                }
                AppEvent::JoinedFlock { code } => {
                    app.flocks.push(FlockView {
                        code,
                        name: String::new(),
                        messages: vec![],
                        unread: 0,
                    });
                }
                AppEvent::JoinedRoost { code, name, channels } => {
                    app.roosts.push(RoostView {
                        code,
                        name,
                        channels: channels.into_iter().map(|c| FlockView {
                            code: c.clone(),
                            name: c,
                            messages: vec![],
                            unread: 0,
                        }).collect(),
                        unread: 0,
                    });
                }
                AppEvent::RoostUpdate { code, name, channels } => {
                    if let Some(rv) = app.roosts.iter_mut().find(|r| r.code == code) {
                        rv.name = name;
                        rv.channels = channels.into_iter().map(|c| FlockView {
                            code: c.clone(),
                            name: c,
                            messages: vec![],
                            unread: 0,
                        }).collect();
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
                AppEvent::HistoryChunk(old) => {
                    if let Some(fv) = app.flocks.first_mut() {
                        let known: std::collections::HashSet<_> =
                            fv.messages.iter().map(|m| m.id.clone()).collect();
                        let mut fresh: Vec<_> =
                            old.into_iter().filter(|m| !known.contains(&m.id)).collect();
                        fresh.extend(std::mem::take(&mut fv.messages));
                        fv.messages = fresh;
                    }
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
                    match k.code {
                        KeyCode::Esc => { app.show_invite = false; }
                        _ => {}
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
                        KeyCode::Esc => { app.show_create_room = false; }
                        _ => {}
                    }
                    continue;
                }

                if app.show_join_room {
                    match k.code {
                        KeyCode::Enter if !app.join_input.is_empty() => {
                            let code = std::mem::take(&mut app.join_input);
                            let _ = cmd_tx.send(Command::JoinFlock {
                                code: code.trim().into(),
                            });
                            app.show_join_room = false;
                        }
                        KeyCode::Char(c) if !c.is_control() => {
                            app.join_input.push(c);
                        }
                        KeyCode::Backspace => { app.join_input.pop(); }
                        KeyCode::Esc => { app.show_join_room = false; }
                        _ => {}
                    }
                    continue;
                }

                if app.show_join_roost {
                    match k.code {
                        KeyCode::Enter if !app.join_roost_input.is_empty() => {
                            let code = std::mem::take(&mut app.join_roost_input);
                            let _ = cmd_tx.send(Command::JoinRoost {
                                code: code.trim().into(),
                            });
                            app.show_join_roost = false;
                        }
                        KeyCode::Char(c) if !c.is_control() => {
                            app.join_roost_input.push(c);
                        }
                        KeyCode::Backspace => { app.join_roost_input.pop(); }
                        KeyCode::Esc => { app.show_join_roost = false; }
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
                        KeyCode::Esc => { app.show_menu = false; }
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
                                .map(|mut child| { let _ = child.wait(); });
                            app.show_create_roost = false;
                        }
                        KeyCode::Char(c) if !c.is_control() => {
                            app.create_roost_input.push(c);
                        }
                        KeyCode::Backspace => { app.create_roost_input.pop(); }
                        KeyCode::Esc => { app.show_create_roost = false; }
                        _ => {}
                    }
                    continue;
                }

                match k.code {
                    KeyCode::Enter if !app.input.is_empty() => {
                        let text = std::mem::take(&mut app.input);
                        if let Some(code) = text.strip_prefix("/join-roost ") {
                            let _ = cmd_tx.send(Command::JoinRoost {
                                code: code.trim().into(),
                            });
                        } else if let Some(code) = text.strip_prefix("/join ") {
                            let _ = cmd_tx.send(Command::JoinFlock {
                                code: code.trim().into(),
                            });
                        } else if let Some(code) = app.active_code() {
                            let _ = cmd_tx.send(Command::SendText {
                                flock: code.to_string(),
                                body: text,
                            });
                        }
                    }

                    KeyCode::Up if k.modifiers.contains(KeyModifiers::ALT) => {
                        let nav = nav_items(&app);
                        if let Some(pos) = nav.iter().position(|s| *s == app.selection) {
                            if pos > 0 {
                                app.selection = nav[pos - 1];
                            }
                        }
                    }
                    KeyCode::Down if k.modifiers.contains(KeyModifiers::ALT) => {
                        let nav = nav_items(&app);
                        if let Some(pos) = nav.iter().position(|s| *s == app.selection) {
                            if pos + 1 < nav.len() {
                                app.selection = nav[pos + 1];
                            }
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
                if m.kind == MouseEventKind::Down(MouseButton::Left) {
                    let col = m.column;
                    let row = m.row;
                    handle_mouse_click(&mut app, &cmd_tx, &muted_flag, &mut term, col, row)?;
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
    execute!(term.backend_mut(), LeaveAlternateScreen, ct_event::DisableMouseCapture)?;
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
        let popup_w = 28u16.min(term_w);
        let popup_h = (MENU_ITEMS.len() as u16 + 2).min(term_h);
        let popup_x = (term_w.saturating_sub(popup_w)) / 2;
        let popup_y = (term_h.saturating_sub(popup_h)) / 2;

        if col >= popup_x && col < popup_x + popup_w
            && row >= popup_y && row < popup_y + popup_h
        {
            let inner_row = row - popup_y;
            if inner_row >= 1 && inner_row < popup_h - 1 {
                let idx = (inner_row - 1) as usize;
                if idx < MENU_ITEMS.len() {
                    app.menu_selection = idx;
                    activate_menu_item(app, cmd_tx, muted_flag)?;
                }
            }
        } else {
            app.show_menu = false;
        }
        return Ok(());
    }

    let button_bar_y = term_h.saturating_sub(4);
    if row == button_bar_y {
        let btns = ui::toolbar_buttons();
        for (i, (_label, bx, bw)) in btns.iter().enumerate() {
            if col >= *bx && col < bx + bw {
                match i {
                    0 => { app.show_create_room = true; }
                    1 => { app.join_input.clear(); app.show_join_room = true; }
                    2 => { app.show_menu = true; app.menu_selection = 0; }
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

        if row >= flocks_top + 1 && row < flocks_top + flocks_h.saturating_sub(1) {
            let idx = (row - flocks_top - 1) as usize;
            if let Some(fv) = app.flocks.get(idx) {
                app.selection = Selection::Flock(idx);
            }
        } else if row >= roosts_top + 1 && row < roosts_top + roosts_h.saturating_sub(1) {
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
                            app.selection = Selection::Channel(ri, ci);
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
        0 => { app.show_create_room = true; }
        1 => { app.join_input.clear(); app.show_join_room = true; }
        2 => { app.join_roost_input.clear(); app.show_join_roost = true; }
        3 => { app.create_roost_input.clear(); app.show_create_roost = true; }
        4 => { app.show_invite = app.active_code().is_some(); }
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
            execute!(std::io::stdout(), LeaveAlternateScreen, ct_event::DisableMouseCapture)?;
            let _ = std::process::Command::new(std::env::current_exe()?)
                .args(["profile"])
                .spawn()
                .map(|mut c| { let _ = c.wait(); });
            execute!(std::io::stdout(), EnterAlternateScreen, ct_event::EnableMouseCapture)?;
            enable_raw_mode()?;
            let profile = starling::config::Profile::load();
            if let Some(p) = profile {
                app.name = p.name.clone();
                app.pronouns = p.pronouns.clone();
                if let Some(c) = ui::hex_to_color(&p.text_color) {
                    app.text_color = c;
                }
                if let Some(c) = ui::hex_to_color(&p.border_color) {
                    app.border_color = c;
                }
                if !p.bg_color.is_empty() {
                    app.bg_color = ui::hex_to_color(&p.bg_color);
                }
            }
        }
        9 => {
            disable_raw_mode()?;
            execute!(std::io::stdout(), LeaveAlternateScreen, ct_event::DisableMouseCapture)?;
            let _ = std::process::Command::new(std::env::current_exe()?)
                .args(["settings"])
                .spawn()
                .map(|mut c| { let _ = c.wait(); });
            execute!(std::io::stdout(), EnterAlternateScreen, ct_event::EnableMouseCapture)?;
            enable_raw_mode()?;
            let profile = starling::config::Profile::load();
            if let Some(p) = profile {
                if let Some(c) = ui::hex_to_color(&p.text_color) {
                    app.text_color = c;
                }
                if let Some(c) = ui::hex_to_color(&p.border_color) {
                    app.border_color = c;
                }
                if !p.bg_color.is_empty() {
                    app.bg_color = ui::hex_to_color(&p.bg_color);
                }
            }
        }
        10 => {
            app.quit_requested = true;
        }
        _ => {}
    }

    Ok(())
}
