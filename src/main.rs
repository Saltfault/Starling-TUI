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
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use ui::{App, FlockView, MENU_ITEMS, RoostView, ScrollPanel, Selection, ToolbarAction};

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

fn open_create_room(app: &mut App) {
    app.create_flock_code = app.node_id.map(|opener| {
        let secret = iroh::SecretKey::generate().to_bytes();
        starling::net::encode_flock_code(&secret, &opener)
    });
    app.show_create_room = true;
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

    if first == Some("settings") {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen, ct_event::EnableBracketedPaste)?;
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
            if starling::net::decode_typed_code(code).is_none() {
                eprintln!("Invalid or unsupported join code.");
                return Ok(());
            }
            Some(code.to_ascii_uppercase())
        }
        _ => None,
    };

    let profile = starling::config::Profile::load();

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        ct_event::EnableMouseCapture,
        ct_event::EnableBracketedPaste
    )?;
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

    let mut last_frame = Instant::now();
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
                AppEvent::Ticket(node_id) => {
                    app.node_id = Some(node_id);
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

            if matches!(event, Event::Paste(_)) {
                continue;
            }

            if let Event::Key(k) = &event {
                if k.kind != KeyEventKind::Press {
                    continue;
                }

                if app.show_create_room {
                    match k.code {
                        KeyCode::Enter => {
                            if let Some(code) = app.create_flock_code.take() {
                                let _ = cmd_tx.send(Command::Join { code });
                            }
                            app.show_create_room = false;
                        }
                        KeyCode::Esc => {
                            app.create_flock_code = None;
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
                            if starling::net::decode_typed_code(code).is_some() {
                                let _ = cmd_tx.send(Command::Join {
                                    code: code.to_ascii_uppercase(),
                                });
                                app.join_input.clear();
                                app.show_join_room = false;
                                app.error_message = None;
                            } else {
                                app.error_message = Some("Invalid or unsupported join code".into());
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

                if app.show_menu {
                    match k.code {
                        KeyCode::Up => {
                            app.menu_selection = app.menu_selection.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            app.menu_selection = (app.menu_selection + 1).min(MENU_ITEMS.len() - 1);
                        }
                        KeyCode::Enter => {
                            activate_menu_item(&mut app, &cmd_tx)?;
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
                            if starling::net::decode_typed_code(code).is_some() {
                                let _ = cmd_tx.send(Command::Join {
                                    code: code.to_ascii_uppercase(),
                                });
                            } else {
                                app.error_message = Some("Invalid or unsupported join code".into());
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
        ct_event::DisableMouseCapture,
        ct_event::DisableBracketedPaste
    )?;
    Ok(())
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
    _term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    col: u16,
    row: u16,
) -> anyhow::Result<()> {
    let (term_w, term_h) = crossterm::terminal::size()?;

    if app.show_menu {
        if let Some(idx) = menu_item_at_size(term_w, term_h, col, row) {
            app.menu_selection = idx;
            activate_menu_item(app, cmd_tx)?;
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
        let btns = ui::toolbar_buttons(app);
        for (action, _label, bx, bw) in btns {
            if col >= bx && col < bx + bw {
                match action {
                    ToolbarAction::Create => {
                        open_create_room(app);
                    }
                    ToolbarAction::Join => {
                        app.join_input.clear();
                        app.show_join_room = true;
                    }
                    ToolbarAction::Menu => {
                        app.show_menu = true;
                        app.menu_selection = 0;
                    }
                    #[cfg(feature = "audio")]
                    ToolbarAction::Call => {
                        if app.in_call {
                            let _ = cmd_tx.send(Command::HangUp);
                            app.in_call = false;
                        } else if let Some(addr) = app.selected_peer_addr() {
                            let _ = cmd_tx.send(Command::StartCall(addr));
                            app.in_call = true;
                        }
                    }
                    #[cfg(feature = "audio")]
                    ToolbarAction::Mute => {
                        app.muted = !app.muted;
                        muted_flag.store(app.muted, Ordering::Relaxed);
                    }
                    #[cfg(feature = "video")]
                    ToolbarAction::Video => {
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
                    ToolbarAction::Quit => {
                        app.quit_requested = true;
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
        if let Some(idx) = app.flock_scroll.row_index(visible_row)
            && app.flocks.get(idx).is_some()
        {
            app.select(Selection::Flock(idx));
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
            app.join_input.clear();
            app.show_join_room = true;
        }
        2 => {
            disable_raw_mode()?;
            execute!(
                std::io::stdout(),
                LeaveAlternateScreen,
                ct_event::DisableMouseCapture,
                ct_event::DisableBracketedPaste
            )?;
            let editor_result = std::process::Command::new(std::env::current_exe()?)
                .arg("settings")
                .status();
            execute!(
                std::io::stdout(),
                EnterAlternateScreen,
                ct_event::EnableMouseCapture,
                ct_event::EnableBracketedPaste
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
        3 => {
            app.quit_requested = true;
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{App, MENU_ITEMS, menu_item_at_size, open_create_room};

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
    fn create_room_generates_a_new_flock_code_each_time() {
        let mut app = App::default();
        app.node_id = Some(iroh::SecretKey::generate().public());

        open_create_room(&mut app);
        let first = app.create_flock_code.clone().expect("first code");
        open_create_room(&mut app);
        let second = app.create_flock_code.clone().expect("second code");

        assert_ne!(first, second);
        assert_eq!(
            starling::net::decode_typed_code(&first).map(|code| code.code_type),
            Some(starling::net::CodeType::Flock)
        );
    }
}
