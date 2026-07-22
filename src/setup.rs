//! Setup wizard — a separate TUI for configuring the user profile and
//! installing system dependencies.
//!
//! Run with `starling setup`. Guides the user through:
//!
//! 1. System dependency check (installs missing packages)
//! 2. WSL2 audio setup (if on WSL2 and not yet configured)
//! 3. Optional: load a profile from a 32-digit code
//! 4. Enter display name
//! 5. Select input (microphone) device
//! 6. Select output (speaker) device
//! 7. Review summary, save, and show the profile code

use crate::config::Profile;
use crate::util::suppress_stderr;
use cpal::traits::HostTrait;
use crossterm::event::{self as ct_event, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

/// Which step of the setup wizard we're on.
enum Phase {
    DependencyCheck,
    WslAudio,
    CodeEntry,
    NameEntry,
    InputDevice,
    OutputDevice,
    Summary,
}

/// Setup wizard state.
struct SetupApp {
    phase: Phase,
    profile: Profile,
    name_input: String,
    code_input: String,
    input_devices: Vec<String>,
    output_devices: Vec<String>,
    selected_input: usize,
    selected_output: usize,
    /// List of missing system dependencies.
    missing_deps: Vec<String>,
    /// Command to install missing dependencies.
    install_cmd: Option<String>,
    /// Whether WSL2 audio needs setup.
    needs_wsl_audio: bool,
    /// Status message after running an install command.
    install_status: String,
}

impl SetupApp {
    fn new() -> Self {
        let input_devices = suppress_stderr(list_input_devices);
        let output_devices = suppress_stderr(list_output_devices);
        let profile = Profile::load().unwrap_or_default();

        let missing_deps = check_dependencies();
        let install_cmd = if missing_deps.is_empty() {
            None
        } else {
            install_command()
        };

        let needs_wsl_audio = cfg!(target_os = "linux")
            && std::path::Path::new("/mnt/wslg").exists()
            && !std::path::Path::new("/etc/asound.conf").exists();

        let phase = if !missing_deps.is_empty() {
            Phase::DependencyCheck
        } else if needs_wsl_audio {
            Phase::WslAudio
        } else {
            Phase::CodeEntry
        };

        Self {
            phase,
            name_input: profile.name.clone(),
            code_input: String::new(),
            input_devices,
            output_devices,
            selected_input: 0,
            selected_output: 0,
            profile,
            missing_deps,
            install_cmd,
            needs_wsl_audio,
            install_status: String::new(),
        }
    }
}

// ── Dependency checking ─────────────────────────────────────────────────

fn command_exists(cmd: &str) -> bool {
    std::process::Command::new("sh")
        .args(["-c", &format!("command -v {cmd}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn pkg_config_exists(lib: &str) -> bool {
    std::process::Command::new("pkg-config")
        .args(["--exists", lib])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn check_dependencies() -> Vec<String> {
    let mut missing = Vec::new();

    if !command_exists("cc") {
        missing.push("C compiler (gcc/cc)".into());
    }

    if cfg!(target_os = "linux") {
        if !command_exists("pkg-config") {
            missing.push("pkg-config".into());
        }
        if !pkg_config_exists("alsa") {
            missing.push("libasound2-dev (ALSA headers)".into());
        }
        if !pkg_config_exists("libpulse") {
            missing.push("libpulse-dev (PulseAudio headers)".into());
        }
    }

    missing
}

fn install_command() -> Option<String> {
    if command_exists("apt-get") {
        Some("sudo apt-get update && sudo apt-get install -y build-essential pkg-config libasound2-dev libpulse-dev".into())
    } else if command_exists("dnf") {
        Some(
            "sudo dnf install -y gcc pkgconf-pkg-config alsa-lib-devel pulseaudio-libs-devel"
                .into(),
        )
    } else if command_exists("pacman") {
        Some("sudo pacman -S --noconfirm base-devel pkgconf alsa-lib pulseaudio".into())
    } else if command_exists("brew") {
        Some("brew install pkg-config".into())
    } else {
        None
    }
}

/// Run a shell command, dropping out of the TUI so the user can see output
/// and enter a sudo password. Returns true on success.
fn run_shell_command(
    term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    cmd: &str,
) -> bool {
    let _ = disable_raw_mode();
    let _ = execute!(
        term.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen
    );

    println!("> {cmd}\n");
    let status = std::process::Command::new("sh").args(["-c", cmd]).status();

    let success = status.map(|s| s.success()).unwrap_or(false);

    println!("\n{}", if success { "Done." } else { "Failed." });
    println!("Press Enter to continue...");
    let _ = std::io::stdin().read_line(&mut String::new());

    let _ = execute!(
        term.backend_mut(),
        crossterm::terminal::EnterAlternateScreen
    );
    let _ = enable_raw_mode();
    success
}

// ── Audio device listing ────────────────────────────────────────────────

fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    let mut devices = vec!["System Default".to_string()];
    if let Ok(iter) = host.input_devices() {
        for device in iter {
            let name = device.to_string();
            if !name.is_empty() {
                devices.push(name);
            }
        }
    }
    devices
}

fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    let mut devices = vec!["System Default".to_string()];
    if let Ok(iter) = host.output_devices() {
        for device in iter {
            let name = device.to_string();
            if !name.is_empty() {
                devices.push(name);
            }
        }
    }
    devices
}

// ── Main setup loop ─────────────────────────────────────────────────────

pub fn run_setup(
    term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<Option<Profile>> {
    let mut app = SetupApp::new();

    loop {
        term.draw(|f| draw(f, &app))?;

        if !ct_event::poll(std::time::Duration::from_millis(50))? {
            continue;
        }
        if let Event::Key(k) = ct_event::read()? {
            if k.kind != KeyEventKind::Press {
                continue;
            }
            match app.phase {
                Phase::DependencyCheck => match k.code {
                    KeyCode::Enter => {
                        if let Some(cmd) = &app.install_cmd {
                            let success = run_shell_command(term, cmd);
                            if success {
                                app.missing_deps.clear();
                                app.install_status = "Dependencies installed successfully.".into();
                            } else {
                                app.install_status =
                                    "Installation failed. See output above.".into();
                            }
                        } else {
                            app.install_status = "No supported package manager found.".into();
                        }
                        app.phase = if app.needs_wsl_audio {
                            Phase::WslAudio
                        } else {
                            Phase::CodeEntry
                        };
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },

                Phase::WslAudio => match k.code {
                    KeyCode::Enter => {
                        let cmd = "sudo apt-get update && sudo apt-get install -y libasound2-plugins && echo 'pcm.!default pulse' | sudo tee /etc/asound.conf > /dev/null && echo 'ctl.!default pulse' | sudo tee -a /etc/asound.conf > /dev/null";
                        let success = run_shell_command(term, cmd);
                        if success {
                            app.needs_wsl_audio = false;
                            app.install_status = "WSL2 audio configured successfully.".into();
                        } else {
                            app.install_status = "WSL2 audio setup failed.".into();
                        }
                        app.phase = Phase::CodeEntry;
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },

                Phase::CodeEntry => match k.code {
                    KeyCode::Enter => {
                        if !app.code_input.is_empty() {
                            if let Some(p) = Profile::from_code(&app.code_input) {
                                app.name_input = p.name.clone();
                                app.profile = p;
                            }
                        }
                        app.phase = Phase::NameEntry;
                    }
                    KeyCode::Char(c) => app.code_input.push(c),
                    KeyCode::Backspace => {
                        app.code_input.pop();
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },

                Phase::NameEntry => match k.code {
                    KeyCode::Enter if !app.name_input.is_empty() => {
                        app.profile.name = app.name_input.clone();
                        app.phase = Phase::InputDevice;
                    }
                    KeyCode::Char(c) => app.name_input.push(c),
                    KeyCode::Backspace => {
                        app.name_input.pop();
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },

                Phase::InputDevice => match k.code {
                    KeyCode::Enter => {
                        app.profile.input_device = if app.selected_input == 0 {
                            None
                        } else {
                            Some(app.input_devices[app.selected_input].clone())
                        };
                        app.phase = Phase::OutputDevice;
                    }
                    KeyCode::Up => {
                        if app.selected_input > 0 {
                            app.selected_input -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if app.selected_input + 1 < app.input_devices.len() {
                            app.selected_input += 1;
                        }
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },

                Phase::OutputDevice => match k.code {
                    KeyCode::Enter => {
                        app.profile.output_device = if app.selected_output == 0 {
                            None
                        } else {
                            Some(app.output_devices[app.selected_output].clone())
                        };
                        app.phase = Phase::Summary;
                    }
                    KeyCode::Up => {
                        if app.selected_output > 0 {
                            app.selected_output -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if app.selected_output + 1 < app.output_devices.len() {
                            app.selected_output += 1;
                        }
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },

                Phase::Summary => match k.code {
                    KeyCode::Enter => {
                        app.profile.save()?;
                        return Ok(Some(app.profile));
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },
            }
        }
    }
}

// ── Rendering ───────────────────────────────────────────────────────────

fn draw(f: &mut Frame, app: &SetupApp) {
    let area = f.area();
    f.render_widget(Clear, area);

    let width = 60.min(area.width);
    let height = 20.min(area.height);
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );

    f.render_widget(Clear, popup);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(" Starling Setup "),
        popup,
    );

    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });

    match app.phase {
        Phase::DependencyCheck => draw_dependency_check(f, inner, app),
        Phase::WslAudio => draw_wsl_audio(f, inner, app),
        Phase::CodeEntry => draw_code_entry(f, inner, app),
        Phase::NameEntry => draw_name_entry(f, inner, app),
        Phase::InputDevice => draw_device_list(
            f,
            inner,
            "Input Device (Microphone)",
            &app.input_devices,
            app.selected_input,
        ),
        Phase::OutputDevice => draw_device_list(
            f,
            inner,
            "Output Device (Speaker)",
            &app.output_devices,
            app.selected_output,
        ),
        Phase::Summary => draw_summary(f, inner, app),
    }
}

fn draw_dependency_check(f: &mut Frame, area: Rect, app: &SetupApp) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    f.render_widget(Paragraph::new("Checking system dependencies..."), chunks[0]);

    let mut lines: Vec<Line> = Vec::new();
    if app.missing_deps.is_empty() {
        lines.push(Line::styled(
            "  All dependencies installed.",
            Style::new().fg(Color::Green),
        ));
    } else {
        lines.push(Line::raw("  Missing:"));
        for dep in &app.missing_deps {
            lines.push(Line::styled(
                format!("    x {}", dep),
                Style::new().fg(Color::Red),
            ));
        }
        lines.push(Line::raw(""));
        if let Some(cmd) = &app.install_cmd {
            lines.push(Line::raw("  Press Enter to install automatically."));
            lines.push(Line::styled(
                format!("  $ {}", cmd),
                Style::new().fg(Color::DarkGray),
            ));
        } else {
            lines.push(Line::styled(
                "  No supported package manager found.",
                Style::new().fg(Color::Red),
            ));
            lines.push(Line::raw(
                "  Please install manually, then run setup again.",
            ));
        }
    }
    f.render_widget(Paragraph::new(lines), chunks[1]);

    if !app.install_status.is_empty() {
        f.render_widget(
            Paragraph::new(format!(" {}", app.install_status)).style(Style::new().fg(Color::Green)),
            chunks[3],
        );
    }

    f.render_widget(
        Paragraph::new(" Enter = install/continue . Esc = cancel")
            .style(Style::new().fg(Color::DarkGray)),
        chunks[4],
    );
}

fn draw_wsl_audio(f: &mut Frame, area: Rect, app: &SetupApp) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    f.render_widget(Paragraph::new("WSL2 Audio Setup"), chunks[0]);
    f.render_widget(Paragraph::new(""), chunks[1]);
    f.render_widget(
        Paragraph::new("Voice calls need the ALSA-PulseAudio bridge."),
        chunks[2],
    );
    f.render_widget(
        Paragraph::new("This installs libasound2-plugins and writes /etc/asound.conf."),
        chunks[3],
    );

    if !app.install_status.is_empty() {
        f.render_widget(
            Paragraph::new(format!(" {}", app.install_status)).style(Style::new().fg(Color::Green)),
            chunks[4],
        );
    }

    f.render_widget(
        Paragraph::new(" Enter = install . Esc = skip").style(Style::new().fg(Color::DarkGray)),
        chunks[5],
    );
}

fn draw_code_entry(f: &mut Frame, area: Rect, app: &SetupApp) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new("Load a profile from a 32-digit code,"),
        chunks[0],
    );
    f.render_widget(Paragraph::new("or press Enter to start fresh."), chunks[1]);
    f.render_widget(
        Paragraph::new(format!(" Code: {}_", app.code_input)).style(Style::new().fg(Color::Yellow)),
        chunks[3],
    );
    f.render_widget(
        Paragraph::new(" Enter = continue . Esc = cancel").style(Style::new().fg(Color::DarkGray)),
        chunks[5],
    );
}

fn draw_name_entry(f: &mut Frame, area: Rect, app: &SetupApp) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new("Enter your display name . the name"),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new("other birds see next to your messages."),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(format!(" Name: {}_", app.name_input)).style(Style::new().fg(Color::Yellow)),
        chunks[3],
    );
    f.render_widget(
        Paragraph::new(" Enter = continue . Esc = cancel").style(Style::new().fg(Color::DarkGray)),
        chunks[4],
    );
}

fn draw_device_list(f: &mut Frame, area: Rect, title: &str, devices: &[String], selected: usize) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    f.render_widget(Paragraph::new(title), chunks[0]);
    f.render_widget(Paragraph::new(""), chunks[1]);

    let items: Vec<ListItem> = devices
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let prefix = if i == selected { "> " } else { "  " };
            ListItem::new(format!("{prefix}{name}"))
        })
        .collect();

    f.render_widget(
        List::new(items).style(Style::new().fg(Color::White)),
        chunks[2],
    );

    f.render_widget(
        Paragraph::new(" Up/Down = navigate . Enter = select . Esc = cancel")
            .style(Style::new().fg(Color::DarkGray)),
        chunks[3],
    );
}

fn draw_summary(f: &mut Frame, area: Rect, app: &SetupApp) {
    let input_name = app
        .profile
        .input_device
        .as_deref()
        .unwrap_or("System Default");
    let output_name = app
        .profile
        .output_device
        .as_deref()
        .unwrap_or("System Default");
    let code = app.profile.to_code();

    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::raw("  Name:   "),
            Span::styled(&app.profile.name, Style::new().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::raw("  Input:  "),
            Span::styled(input_name, Style::new().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("  Output: "),
            Span::styled(output_name, Style::new().fg(Color::Cyan)),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::raw("  Profile code: "),
            Span::styled(code, Style::new().fg(Color::Green).bold()),
        ]),
        Line::raw(""),
        Line::raw("  Save this code to restore your name on"),
        Line::raw("  another machine with: starling setup"),
        Line::raw(""),
        Line::styled(
            "  Enter = save & exit . Esc = cancel",
            Style::new().fg(Color::DarkGray),
        ),
    ];

    f.render_widget(Paragraph::new(lines), area);
}
