//! Starling — a federated p2p communications platform where peers, known as
//! birds, communicate via the murmuration.
//!
//! Subcommands:
//! - `starling setup` — configure profile (name, audio devices, code)
//! - `starling open`  — start a new flock
//! - `starling join <code>` — join an existing flock
//!
//! If no profile exists, `open` and `join` automatically run the setup wizard.
//! Press `i` in the chat to view the invite code.

mod call;
mod config;
mod crypto;
mod event;
mod logger;
mod net;
mod opus_ffi;
mod playback;
mod setup;
mod ui;
mod util;
mod voice;

use crossterm::{
    event::{self as ct_event, Event, KeyCode, KeyModifiers},
    execute,
    terminal::*,
};
use event::{AppEvent, Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use ui::App;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logger::init();

    let args: Vec<String> = std::env::args().collect();

    // ── Subcommand: `starling setup` ──────────────────────────────────
    if args.get(1).map(String::as_str) == Some("setup") {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
        setup::run_setup(&mut term)?;
        disable_raw_mode()?;
        execute!(term.backend_mut(), LeaveAlternateScreen)?;
        return Ok(());
    }

    // ── Subcommand: `starling open` or `starling join <code>` ──────
    // The join code is a base58-encoded node ID (~44 chars).
    let bootstrap = match args.get(1).map(String::as_str) {
        Some("join") => {
            let code = &args[2];
            match net::decode_node_id(code) {
                Some(node_id) => vec![node_id],
                None => {
                    eprintln!("Invalid join code. Make sure you copied it correctly.");
                    return Ok(());
                }
            }
        }
        _ => vec![],
    };

    let profile = config::Profile::load();

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
    let mut app = App::default();

    // For "join", derive the room code from the opener's node ID immediately.
    if let Some(node_id) = bootstrap.first() {
        app.invite = Some(net::room_code_from_node_id(node_id));
    }

    // Get name and device preferences. If a profile exists, use it.
    // If not, run the setup wizard.
    let (name, input_device, output_device) = if let Some(p) = &profile {
        app.name = p.name.clone();
        (
            p.name.clone(),
            p.input_device.clone(),
            p.output_device.clone(),
        )
    } else {
        match setup::run_setup(&mut term)? {
            Some(p) => {
                app.name = p.name.clone();
                (
                    p.name.clone(),
                    p.input_device.clone(),
                    p.output_device.clone(),
                )
            }
            None => {
                disable_raw_mode()?;
                execute!(term.backend_mut(), LeaveAlternateScreen)?;
                return Ok(());
            }
        }
    };

    // Start the network task.
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Command>();
    let (evt_tx, mut evt_rx) = mpsc::unbounded_channel::<AppEvent>();
    let muted_flag = Arc::new(AtomicBool::new(false));

    tokio::spawn(net::run(
        bootstrap,
        cmd_rx,
        evt_tx,
        muted_flag.clone(),
        name,
        input_device,
    ));

    let mut playback = match playback::Playback::new(output_device.as_deref()) {
        Ok(p) => Some(p),
        Err(e) => {
            logger::warn(&format!("audio playback unavailable: {e}"));
            None
        }
    };

    // Main chat loop.
    loop {
        term.draw(|f| ui::draw(f, &app))?;

        while let Ok(ev) = evt_rx.try_recv() {
            match ev {
                AppEvent::Message(m) => app.messages.push(m),
                AppEvent::PeerConnected(id) => {
                    if !app.peers.contains(&id) {
                        app.peers.push(id);
                    }
                }
                AppEvent::PeerDisconnected(id) => {
                    app.peers.retain(|p| p != &id);
                    if !app.peers.is_empty() {
                        app.selected_peer %= app.peers.len();
                    } else {
                        app.selected_peer = 0;
                    }
                }
                AppEvent::Ticket(code) => {
                    // For "open": this is our base58-encoded node ID.
                    // Derive the room code from it for display.
                    if app.invite.is_none() {
                        if let Some(node_id) = net::decode_node_id(&code) {
                            app.invite = Some(net::room_code_from_node_id(&node_id));
                            app.node_id = Some(code);
                        }
                    }
                }
                AppEvent::VoiceFrame(bytes) => {
                    if let Some(p) = &mut playback {
                        p.push_opus(&bytes);
                    }
                }
            }
        }

        if ct_event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(k) = ct_event::read()? {
                if app.show_invite {
                    match k.code {
                        KeyCode::Char('i') | KeyCode::Esc => {
                            app.show_invite = false;
                        }
                        _ => {}
                    }
                    continue;
                }

                match k.code {
                    KeyCode::Enter if !app.input.is_empty() => {
                        let text = std::mem::take(&mut app.input);
                        let _ = cmd_tx.send(Command::SendText(text));
                    }

                    KeyCode::Char('i') => {
                        app.show_invite = true;
                    }

                    KeyCode::Char('k') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        if app.in_call {
                            let _ = cmd_tx.send(Command::HangUp);
                            app.in_call = false;
                        } else if let Some(addr) = app.selected_peer_addr() {
                            let _ = cmd_tx.send(Command::StartCall(addr));
                            app.in_call = true;
                        }
                    }

                    KeyCode::Char('m') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.muted = !app.muted;
                        muted_flag.store(app.muted, Ordering::Relaxed);
                    }

                    KeyCode::Tab => {
                        app.select_next_peer();
                    }

                    KeyCode::Char(c) => app.input.push(c),

                    KeyCode::Backspace => {
                        app.input.pop();
                    }

                    KeyCode::Esc => {
                        let _ = cmd_tx.send(Command::Quit);
                        break;
                    }

                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
