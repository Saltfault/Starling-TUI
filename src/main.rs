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
    event::{self as ct_event, Event, KeyCode, KeyEventKind, KeyModifiers},
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
use ui::{App, FlockView};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logger::init();

    let args: Vec<String> = std::env::args().collect();

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

    // ── TUI-only commands ────────────────────────────────────────────
    // `starling leave <code>` — print leave instructions (no-op in TUI).
    // The user can just close the app to "leave" a flock.
    if args.get(1).map(String::as_str) == Some("leave") {
        let _code = args.get(2).cloned().unwrap_or_default();
        println!("To leave a flock, simply close the app (Esc).");
        println!("A roost can be stopped with: starling-server roost close <name>");
        return Ok(());
    }

    // `starling list` — list known roosts on disk.
    if args.get(1).map(String::as_str) == Some("list") {
        let roosts_dir = config::Profile::roosts_dir();
        if !roosts_dir.exists() {
            println!("No roosts found. Create one with: starling-server roost create <name>");
            return Ok(());
        }
        let mut count = 0;
        for entry in std::fs::read_dir(&roosts_dir).map_err(|e| {
            eprintln!("Error reading roosts directory: {e}");
            std::process::exit(1);
        })? {
            if let Ok(entry) = entry {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    println!("  roost: {name}");
                    count += 1;
                }
            }
        }
        if count == 0 {
            println!("No roosts found. Create one with: starling-server roost create <name>");
        }
        return Ok(());
    }

    // `starling doctor` — diagnose setup.
    if args.get(1).map(String::as_str) == Some("doctor") {
        println!("Starling Doctor");
        println!("---------------");

        let config_dir = config::Profile::config_dir();
        if config_dir.exists() {
            println!("  ✓ config directory: {}", config_dir.display());
        } else {
            println!("  ✗ config directory missing — run `starling setup`");
            return Ok(());
        }

        let identity = config_dir.join("identity.key");
        if identity.exists() {
            println!("  ✓ identity key: {}", identity.display());
        } else {
            println!("  ✗ identity key missing — will be created on first launch");
        }

        let profile = config_dir.join("profile.bin");
        if profile.exists() {
            println!("  ✓ profile: {}", profile.display());
        } else {
            println!("  ✗ profile not configured — run `starling setup`");
        }

        let roosts_dir = config::Profile::roosts_dir();
        if roosts_dir.exists() {
            let count = std::fs::read_dir(&roosts_dir)
                .map(|d| {
                    d.filter_map(|e| e.ok())
                        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                        .count()
                })
                .unwrap_or(0);
            println!("  ✓ roosts on disk: {count}");
            println!("    ({})", roosts_dir.display());
        } else {
            println!("  ○ no roosts directory (none created yet)");
        }

        println!();
        println!("System dependencies:");
        if std::process::Command::new("cargo")
            .arg("--version")
            .output()
            .is_ok()
        {
            println!("  ✓ cargo installed");
        } else {
            println!("  ✗ cargo not found — install Rust: https://rustup.rs");
        }

        return Ok(());
    }

    // `starling logs` — show the log file path.
    if args.get(1).map(String::as_str) == Some("logs") {
        println!("Starling TUI logs:");
        println!("  logs/latest.log  (in the working directory)");
        return Ok(());
    }

    // `starling tui version|update|uninstall` — TUI self-management.
    if args.get(1).map(String::as_str) == Some("tui") {
        match args.get(2).map(String::as_str) {
            Some("version") => {
                println!("Starling TUI v{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            Some("update") => {
                println!("To update Starling TUI:");
                println!(
                    "  cargo install starling-tui --git https://forgejo.hearthhome.lol/Saltfault/Starling-TUI.git"
                );
                println!();
                println!("Or if you cloned the repo: git pull && cargo build --release");
                return Ok(());
            }
            Some("uninstall") => {
                println!("To uninstall Starling TUI:");
                println!("  1. Remove the binary: cargo uninstall starling-tui");
                println!(
                    "  2. Remove config: rm -rf {}",
                    config::Profile::config_dir().display()
                );
                println!();
                println!("If you installed from source, just delete the repository.");
                return Ok(());
            }
            _ => {
                eprintln!("Usage: starling tui <version|update|uninstall>");
                std::process::exit(1);
            }
        }
    }

    // `starling help` — print usage.
    if matches!(
        args.get(1).map(String::as_str),
        Some("help" | "--help" | "-h")
    ) {
        println!(
            "Starling TUI v{} — federated p2p communications",
            env!("CARGO_PKG_VERSION")
        );
        println!();
        println!("Usage:");
        println!("  starling join <code>                    join a flock or roost");
        println!("  starling open                           open the TUI");
        println!("  starling leave <code>                   leave a flock or roost");
        println!("  starling list                           list flocks and roosts");
        println!("  starling doctor                         diagnose setup");
        println!("  starling logs                           show log file path");
        println!("  starling tui version                    print version");
        println!("  starling tui update                     print update instructions");
        println!("  starling tui uninstall                  print uninstall instructions");
        return Ok(());
    }

    let bootstrap = match args.get(1).map(String::as_str) {
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
    execute!(stdout, EnterAlternateScreen)?;
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
                        .flocks
                        .get(app.current_flock)
                        .is_some_and(|fv| fv.code == flock);
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
                        KeyCode::Char(c) => app.join_input.push(c),
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

                match k.code {
                    KeyCode::Enter if !app.input.is_empty() => {
                        let text = std::mem::take(&mut app.input);
                        if let Some(code) = text.strip_prefix("/join ") {
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
                        let max = app.flocks.len().saturating_sub(1);
                        app.current_flock = app.current_flock.saturating_sub(1).min(max);
                    }
                    KeyCode::Down if k.modifiers.contains(KeyModifiers::ALT) => {
                        app.current_flock =
                            (app.current_flock + 1).min(app.flocks.len().saturating_sub(1));
                    }

                    KeyCode::Char('n') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.show_create_room = true;
                    }
                    KeyCode::Char('j') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.join_input.clear();
                        app.show_join_room = true;
                    }

                    KeyCode::Char('i') => {
                        app.show_invite = app.flocks.get(app.current_flock).is_some();
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

                    KeyCode::Char(c) => app.input.push(c),

                    KeyCode::Backspace => {
                        app.input.pop();
                    }

                    KeyCode::Esc => {
                        let _ = cmd_tx.send(Command::Quit);
                        // Give the network task time to drain the Quit
                        // command and shut down before the terminal exits.
                        tokio::time::sleep(Duration::from_millis(500)).await;
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
