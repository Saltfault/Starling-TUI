//! Starling — a federated p2p communications platform where peers, known as
//! birds, communicate via the murmuration.
//!
//! Architecture (one task + one UI loop):
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────┐
//! │ main.rs (UI loop)                                                │
//! │   keyboard → Command ──┐                                         │
//! │   AppEvent ←───────────┤──── mpsc channels ────┐                │
//! │   playback ← VoiceFrame│                       │                │
//! └────────────────────────┊────────────────────────┊───────────────┘
//!                          ▼                        ▼
//! ┌──────────────────────────────────────────────────────────────────┐
//! │ net.rs (network task)                                            │
//! │   gossip for chat · QUIC datagrams for voice                     │
//! │   mic capture (voice.rs) → place_call (call.rs)                  │
//! └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The app starts in **name-entry mode**: a popup asks for the bird's display
//! name. Once confirmed, the network task is spawned and the chat UI begins.
//!
//! Keybindings (chat mode):
//!
//! | Key        | Action                          |
//! |------------|---------------------------------|
//! | `Enter`    | Send typed message              |
//! | `Ctrl+K`   | Start call / hang up            |
//! | `Ctrl+M`   | Toggle mute                     |
//! | `Tab`      | Cycle selected peer             |
//! | `Backspace`| Delete last character           |
//! | `Esc`      | Quit                            |

mod call;
mod event;
mod net;
mod playback;
mod ui;
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

/// Generate a random room code: "BIRD" + 6 digits (e.g. "BIRD324524").
fn generate_room_code() -> String {
    let uuid = uuid::Uuid::new_v4();
    let bytes = uuid.as_bytes();
    let digits: String = (0..6)
        .map(|i| char::from_digit((bytes[i] % 10) as u32, 10).unwrap())
        .collect();
    format!("BIRD{digits}")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Parse CLI args: `starling open` or `starling join <code>` ─────
    let args: Vec<String> = std::env::args().collect();
    let (topic, room_code) = match args.get(1).map(String::as_str) {
        Some("join") => {
            let code = args[2].clone();
            (net::topic_for(&format!("starling/flock/{code}")), code)
        }
        _ => {
            // "open" — generate a random room code
            let code = generate_room_code();
            (net::topic_for(&format!("starling/flock/{code}")), code)
        }
    };

    // ── Set up the terminal ───────────────────────────────────────────
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
    let mut app = App::default();

    // The room code is the invite — display it in the header immediately.
    app.invite = Some(room_code);

    // ── Phase 1: Name entry ───────────────────────────────────────────
    //
    // Show a popup asking for the bird's display name. The network task
    // hasn't started yet — we need the name before spawning it.
    loop {
        term.draw(|f| ui::draw(f, &app))?;

        if ct_event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(k) = ct_event::read()? {
                match k.code {
                    KeyCode::Enter if !app.name_input.is_empty() => {
                        app.name = std::mem::take(&mut app.name_input);
                        break;
                    }
                    KeyCode::Char(c) => app.name_input.push(c),
                    KeyCode::Backspace => {
                        app.name_input.pop();
                    }
                    _ => {}
                }
            }
        }
    }

    // ── Phase 2: Start the network task ───────────────────────────────
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Command>();
    let (evt_tx, mut evt_rx) = mpsc::unbounded_channel::<AppEvent>();

    // Shared mute flag (UI toggles it, mic callback reads it).
    let muted_flag = Arc::new(AtomicBool::new(false));

    tokio::spawn(net::run(
        topic,
        cmd_rx,
        evt_tx,
        muted_flag.clone(),
        app.name.clone(),
    ));

    // ── Set up audio playback (optional — app works without it) ───────
    let mut playback = match playback::Playback::new() {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("warning: audio playback unavailable: {e}");
            None
        }
    };

    // ── Phase 3: Main chat loop ───────────────────────────────────────
    loop {
        term.draw(|f| ui::draw(f, &app))?;

        // Drain any network events into UI state.
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
                AppEvent::VoiceFrame(bytes) => {
                    if let Some(p) = &mut playback {
                        p.push_opus(&bytes);
                    }
                }
            }
        }

        // Poll keyboard with a short timeout so the loop keeps spinning
        // (this lets us drain network events promptly).
        if ct_event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(k) = ct_event::read()? {
                match k.code {
                    // Send message
                    KeyCode::Enter if !app.input.is_empty() => {
                        let text = std::mem::take(&mut app.input);
                        let _ = cmd_tx.send(Command::SendText(text));
                    }

                    // Ctrl+K: start call / hang up
                    KeyCode::Char('k') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        if app.in_call {
                            let _ = cmd_tx.send(Command::HangUp);
                            app.in_call = false;
                        } else if let Some(addr) = app.selected_peer_addr() {
                            let _ = cmd_tx.send(Command::StartCall(addr));
                            app.in_call = true;
                        }
                    }

                    // Ctrl+M: toggle mute
                    KeyCode::Char('m') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.muted = !app.muted;
                        muted_flag.store(app.muted, Ordering::Relaxed);
                    }

                    // Tab: cycle selected peer
                    KeyCode::Tab => {
                        app.select_next_peer();
                    }

                    // Type a character
                    KeyCode::Char(c) => app.input.push(c),

                    // Backspace
                    KeyCode::Backspace => {
                        app.input.pop();
                    }

                    // Esc: quit
                    KeyCode::Esc => {
                        let _ = cmd_tx.send(Command::Quit);
                        break;
                    }

                    _ => {}
                }
            }
        }
    }

    // ── Restore the terminal ──────────────────────────────────────────
    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
