//! Starling — a federated p2p communications platform.

mod call;
mod config;
mod crypto;
mod event;
mod logger;
mod net;
#[cfg(feature = "audio")]
mod opus_ffi;
#[cfg(feature = "audio")]
mod playback;
mod roost;
mod setup;
mod sync;
mod ui;
mod util;
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
use ui::{App, FlockView, RoostView};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logger::init();

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

    let bootstrap = match first {
        Some("join") => {
            let code = &args[2];
            match net::decode_node_id(code) {
                Some(node_id) => vec![node_id],
                None => {
                    eprintln!("Invalid join code.");
                    return Ok(());
                }
            }
        }
        _ => vec![],
    };

    let profile = config::Profile::load();

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, ct_event::EnableMouseCapture)?;
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
    let mut app = App::default();

    // Derive our own node ID from the persistent secret key.
    // We pass it to net::run for profile announcements; the Ticket
    // event will set app.node_id once the endpoint is bound.
    let secret = config::Profile::load_or_create_secret();
    let my_node_id: iroh::EndpointId = secret.public().into();

    // No room is open by default — the user creates one with Ctrl+N
    // or joins one with Ctrl+J (or starling join from the CLI).

    // Load the profile from disk, or run the setup wizard to create one.
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
    let mut playback = match playback::Playback::new(output_device.as_deref()) {
        Ok(p) => Some(p),
        Err(e) => {
            logger::warn(&format!("audio playback unavailable: {e}"));
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
                        messages: vec![],
                        unread: 0,
                    });
                }
                AppEvent::JoinedRoost { code, name, channels } => {
                    app.roosts.push(RoostView {
                        code,
                        name,
                        channels: channels.into_iter().map(|c| FlockView {
                            code: c,
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
                            code: c,
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
                    // Our own persistent invite code — stored for use when
                    // the user creates a room (Ctrl+N).
                    app.node_id = Some(code);
                }
                #[cfg(feature = "audio")]
                AppEvent::VoiceFrame(bytes) => {
                    if let Some(p) = &mut playback {
                        p.push_opus(&bytes);
                    }
                }
                #[cfg(not(feature = "audio"))]
                AppEvent::VoiceFrame(_) => {}
                #[cfg(feature = "video")]
                AppEvent::VideoFrame(jpeg) => {
                    if let Ok(img) = image::load_from_memory(&jpeg) {
                        app.video_frame = Some(img.to_rgb8());
                    }
                }
                #[cfg(not(feature = "video"))]
                AppEvent::VideoFrame(_) => {}
                AppEvent::HistoryChunk(old) => {
                    // Prepend into the first flock, dedup by id.
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
            if let Event::Key(k) = ct_event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }

                if app.show_invite {
                    match k.code {
                        KeyCode::Char('i') | KeyCode::Esc => {
                            app.show_invite = false;
                        }
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
                        KeyCode::Esc => {
                            app.show_create_room = false;
                        }
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
                        KeyCode::Char(c) => {
                            if !c.is_control() {
                                app.join_input.push(c);
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

                if app.show_join_roost {
                    match k.code {
                        KeyCode::Enter if !app.join_roost_input.is_empty() => {
                            let code = std::mem::take(&mut app.join_roost_input);
                            let _ = cmd_tx.send(Command::JoinRoost {
                                code: code.trim().into(),
                            });
                            app.show_join_roost = false;
                        }
                        KeyCode::Char(c) => {
                            if !c.is_control() {
                                app.join_roost_input.push(c);
                            }
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
                        } else if let Some(fv) = app.active() {
                            let _ = cmd_tx.send(Command::SendText {
                                flock: fv.code.clone(),
                                body: text,
                            });
                        }
                    }

                    KeyCode::Up if k.modifiers.contains(KeyModifiers::ALT) => {
                        let max = app.rail_len().saturating_sub(1);
                        app.current_item = app.current_item.saturating_sub(1).min(max);
                    }
                    KeyCode::Down if k.modifiers.contains(KeyModifiers::ALT) => {
                        app.current_item =
                            (app.current_item + 1).min(app.rail_len().saturating_sub(1));
                    }

                    KeyCode::Char('n') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.show_create_room = true;
                    }
                    KeyCode::Char('j') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.join_input.clear();
                        app.show_join_room = true;
                    }
                    KeyCode::Char('r') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.join_roost_input.clear();
                        app.show_join_roost = true;
                    }

                    KeyCode::Char('i') => {
                        app.show_invite = app.active_code().is_some();
                    }

                    #[cfg(feature = "audio")]
                    KeyCode::Char('k') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        if app.in_call {
                            let _ = cmd_tx.send(Command::HangUp);
                            app.in_call = false;
                        } else if let Some(addr) = app.selected_peer_addr() {
                            let _ = cmd_tx.send(Command::StartCall(addr));
                            app.in_call = true;
                        }
                    }

                    #[cfg(feature = "audio")]
                    KeyCode::Char('m') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.muted = !app.muted;
                        muted_flag.store(app.muted, Ordering::Relaxed);
                    }

                    #[cfg(feature = "video")]
                    KeyCode::Char('v') if k.modifiers.contains(KeyModifiers::CONTROL) => {
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

                    KeyCode::Tab => {
                        app.select_next_peer();
                    }

                    KeyCode::Char(c) => {
                        if !c.is_control() {
                            app.input.push(c);
                        }
                    }

                    KeyCode::Backspace => {
                        app.input.pop();
                    }

                    KeyCode::Esc => {
                        let _ = cmd_tx.send(Command::Quit);
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        break;
                    }

                    _ => {}
                }
            } else if let ct_event::Event::Mouse(m) = ct_event::read()? {
                if m.kind == MouseEventKind::Down(MouseButton::Left) {
                    let col = m.column;
                    let row = m.row;
                    if col < 14 {
                        let (_, term_h) = crossterm::terminal::size()?;
                        let middle_h = term_h.saturating_sub(6);
                        let flocks_h = middle_h / 2;
                        let flocks_top = 2u16;
                        let roosts_top = flocks_top + flocks_h;

                        if row >= flocks_top + 1 && row < roosts_top {
                            let idx = (row - flocks_top - 1) as usize;
                            if idx < app.flocks.len() {
                                app.current_item = idx;
                            }
                        } else if row >= roosts_top + 1 && row < roosts_top + middle_h - flocks_h {
                            let idx = (row - roosts_top - 1) as usize;
                            if idx < app.roosts.len() {
                                app.current_item = app.flocks.len() + idx;
                            }
                        }
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen, ct_event::DisableMouseCapture)?;
    Ok(())
}
