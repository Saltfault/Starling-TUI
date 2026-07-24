use starling::config::Profile;
#[cfg(feature = "audio")]
use starling::util::suppress_stderr;
#[cfg(feature = "audio")]
use cpal::traits::HostTrait;
use crossterm::event::{self as ct_event, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

enum Mode {
    Full,
    Settings,
}

#[derive(Clone)]
enum Phase {
    DependencyCheck,
    CodeEntry,
    NameEntry,
    PronounsEntry,
    InputDevice,
    OutputDevice,
    ColorText,
    ColorBg,
    ColorBorder,
    Summary,
}

struct SetupApp {
    phase: Phase,
    profile: Profile,
    name_input: String,
    pronouns_input: String,
    code_input: String,
    input_devices: Vec<String>,
    output_devices: Vec<String>,
    selected_input: usize,
    selected_output: usize,
    missing_deps: Vec<String>,
    install_cmd: Option<String>,
    install_status: String,
    text_color_input: String,
    bg_color_input: String,
    border_color_input: String,
    hex_error: String,
}

fn hex_preview(hex: &str) -> Option<Color> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    Some(Color::Rgb(
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ))
}

fn valid_hex(hex: &str) -> bool {
    hex_preview(hex).is_some()
}

impl SetupApp {
    fn new(mode: Mode) -> Self {
        #[cfg(feature = "audio")]
        let input_devices = suppress_stderr(|| list_devices(true));
        #[cfg(not(feature = "audio"))]
        let input_devices = vec!["System Default".to_string()];
        #[cfg(feature = "audio")]
        let output_devices = suppress_stderr(|| list_devices(false));
        #[cfg(not(feature = "audio"))]
        let output_devices = vec!["System Default".to_string()];
        let profile = Profile::load().unwrap_or_default();

        let missing_deps = check_dependencies();
        let install_cmd = if missing_deps.is_empty() {
            None
        } else {
            install_command()
        };

        let phase = match &mode {
            Mode::Full => {
                if !missing_deps.is_empty() {
                    Phase::DependencyCheck
                } else {
                    Phase::CodeEntry
                }
            }
            Mode::Settings => Phase::InputDevice,
        };

        let selected_input = profile
            .input_device
            .as_ref()
            .and_then(|d| input_devices.iter().position(|x| x == d))
            .unwrap_or(0);
        let selected_output = profile
            .output_device
            .as_ref()
            .and_then(|d| output_devices.iter().position(|x| x == d))
            .unwrap_or(0);

        let profile_clone = profile.clone();
        Self {
            phase,
            name_input: profile.name.clone(),
            pronouns_input: profile.pronouns.clone(),
            code_input: String::new(),
            input_devices,
            output_devices,
            selected_input,
            selected_output,
            profile,
            missing_deps,
            install_cmd,
            install_status: String::new(),
            text_color_input: profile_clone.text_color.clone(),
            bg_color_input: profile_clone.bg_color.clone(),
            border_color_input: profile_clone.border_color.clone(),
            hex_error: String::new(),
        }
    }

    fn hex_color_name(&self, phase: &Phase) -> &str {
        match phase {
            Phase::ColorText => "Text Color",
            Phase::ColorBg => "Background Color",
            Phase::ColorBorder => "Border Color",
            _ => "",
        }
    }

    fn current_hex_input(&self, phase: &Phase) -> &str {
        match phase {
            Phase::ColorText => &self.text_color_input,
            Phase::ColorBg => &self.bg_color_input,
            Phase::ColorBorder => &self.border_color_input,
            _ => "",
        }
    }

    fn finish_colors(&mut self) {
        if valid_hex(&self.text_color_input) {
            let h = self.text_color_input.trim_start_matches('#');
            self.profile.text_color = format!("#{h}");
        }
        if valid_hex(&self.bg_color_input) {
            let h = self.bg_color_input.trim_start_matches('#');
            self.profile.bg_color = format!("#{h}");
        }
        if valid_hex(&self.border_color_input) {
            let h = self.border_color_input.trim_start_matches('#');
            self.profile.border_color = format!("#{h}");
        }
    }
}

fn command_exists(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[allow(dead_code)]
fn pkg_config_exists(lib: &str) -> bool {
    std::process::Command::new("pkg-config")
        .args(["--exists", lib])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn check_dependencies() -> Vec<String> {
    #[cfg(target_os = "windows")]
    return Vec::new();

    #[cfg(not(target_os = "windows"))]
    {
        let mut missing = Vec::new();

        if !command_exists("cc") && !command_exists("gcc") {
            missing.push("C compiler (gcc/cc)".into());
        }

        #[cfg(target_os = "linux")]
        {
            if !command_exists("pkg-config") {
                missing.push("pkg-config".into());
            }
            if !pkg_config_exists("alsa") {
                missing.push("libasound2-dev (ALSA headers)".into());
            }
            if !pkg_config_exists("libpulse") {
                missing.push("libpulse-dev (PulseAudio headers)".into());
            }
            let has_libclang = command_exists("libclang")
                || std::path::Path::new("/usr/lib/x86_64-linux-gnu/libclang.so").exists()
                || std::path::Path::new("/usr/lib/aarch64-linux-gnu/libclang.so").exists()
                || std::fs::read_dir("/usr/lib/llvm-")
                    .map(|mut d| {
                        d.any(|e| {
                            e.ok()
                                .map_or(false, |f| f.path().join("lib/libclang.so").exists())
                        })
                    })
                    .unwrap_or(false)
                || std::fs::read_dir("/usr/lib")
                    .map(|mut d| {
                        d.any(|e| {
                            e.ok().map_or(false, |f| {
                                f.file_name().to_string_lossy().starts_with("libclang.so")
                            })
                        })
                    })
                    .unwrap_or(false);
            if !has_libclang {
                missing.push("libclang-dev (needed by nokhwa/V4L2 bindgen)".into());
            }
            if !pkg_config_exists("libv4l2") && !pkg_config_exists("v4l-utils") {
                missing.push("libv4l-dev (needed by nokhwa/V4L2)".into());
            }
            if std::path::Path::new("/mnt/wslg").exists()
                && !std::path::Path::new("/etc/asound.conf").exists()
            {
                missing.push("libasound2-plugins + /etc/asound.conf (WSL2 audio bridge)".into());
            }
            if std::path::Path::new("/mnt/wslg").exists()
                && !std::fs::read_dir("/dev")
                    .map(|mut d| {
                        d.any(|e| {
                            e.ok().map_or(false, |f| {
                                f.file_name().to_string_lossy().starts_with("video")
                            })
                        })
                    })
                    .unwrap_or(false)
            {
                missing.push(
                    "webcam (WSL2): install usbipd-win on Windows, then attach camera".into(),
                );
            }
        }

        missing
    }
}

fn install_command() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        if command_exists("winget") {
            return Some(
                "winget install Microsoft.VisualStudio.2022.BuildTools --includeRecommended".into(),
            );
        }
        if command_exists("choco") {
            return Some("choco install visualstudio2022buildtools".into());
        }
    }

    let needs_wsl_audio = std::path::Path::new("/mnt/wslg").exists()
        && !std::path::Path::new("/etc/asound.conf").exists();

    let wsl_audio_suffix = if needs_wsl_audio {
        " && sudo apt-get install -y libasound2-plugins && printf 'pcm.!default {\\n    type pulse\\n}\\nctl.!default {\\n    type pulse\\n}\\n' | sudo tee /etc/asound.conf > /dev/null"
    } else {
        ""
    };

    if command_exists("apt-get") {
        Some(format!(
            "sudo apt-get update && sudo apt-get install -y build-essential pkg-config libasound2-dev libpulse-dev libclang-dev libv4l-dev{}",
            wsl_audio_suffix
        ))
    } else if command_exists("dnf") {
        Some(
            "sudo dnf install -y gcc pkgconf-pkg-config alsa-lib-devel pulseaudio-libs-devel clang-devel"
                .into(),
        )
    } else if command_exists("pacman") {
        Some("sudo pacman -S --noconfirm base-devel pkgconf alsa-lib pulseaudio clang".into())
    } else if command_exists("brew") {
        Some("brew install pkg-config".into())
    } else {
        None
    }
}

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

fn rebuild_command() -> String {
    let git_dir = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
        .join(".cargo/git/db");

    if let Ok(entries) = std::fs::read_dir(&git_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("starling") {
                let output = std::process::Command::new("git")
                    .args(["config", "--get", "remote.origin.url"])
                    .current_dir(entry.path())
                    .output();
                if let Ok(out) = output {
                    let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !url.is_empty() {
                        return format!(
                            "cargo install --git {} --features audio,video --force",
                            url
                        );
                    }
                }
            }
        }
    }

    "cargo install Starling-TUI --features audio,video --force".into()
}

#[cfg(feature = "audio")]
fn list_devices(is_input: bool) -> Vec<String> {
    let host = cpal::default_host();
    let devices = if is_input {
        host.input_devices().map(|i| i.collect::<Vec<_>>())
    } else {
        host.output_devices().map(|i| i.collect::<Vec<_>>())
    };

    let mut names = vec!["System Default".to_string()];
    if let Ok(devices) = devices {
        for device in devices {
            let name = device.to_string();
            if !name.is_empty() {
                names.push(name);
            }
        }
    }
    names
}

pub fn run_setup(
    term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<Option<Profile>> {
    run_wizard(term, Mode::Full)
}

pub fn run_settings(
    term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<Option<Profile>> {
    run_wizard(term, Mode::Settings)
}

fn run_wizard(
    term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    mode: Mode,
) -> anyhow::Result<Option<Profile>> {
    let mut app = SetupApp::new(mode);

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
                                app.install_status = "Dependencies installed. Rebuilding...".into();
                                term.draw(|f| draw(f, &app))?;

                                let rebuild = run_shell_command(term, &rebuild_command());
                                if rebuild {
                                    app.install_status =
                                        "Installed! Restart starling to use audio/video.".into();
                                } else {
                                    app.install_status =
                                        "Deps installed but rebuild failed. Run: cargo install Starling-TUI --features audio,video --force".into();
                                }
                            } else {
                                app.install_status =
                                    "Installation failed. See output above.".into();
                            }
                        } else {
                            app.install_status = "No supported package manager found.".into();
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
                        app.phase = Phase::PronounsEntry;
                    }
                    KeyCode::Char(c) => app.name_input.push(c),
                    KeyCode::Backspace => {
                        app.name_input.pop();
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },

                Phase::PronounsEntry => match k.code {
                    KeyCode::Enter => {
                        app.profile.pronouns = app.pronouns_input.clone();
                        app.phase = Phase::InputDevice;
                    }
                    KeyCode::Char(c) => app.pronouns_input.push(c),
                    KeyCode::Backspace => {
                        app.pronouns_input.pop();
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
                        app.phase = Phase::ColorText;
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

                Phase::ColorText | Phase::ColorBg | Phase::ColorBorder => {
                    let cur = app.phase.clone();
                    let next = match cur {
                        Phase::ColorText => Phase::ColorBg,
                        Phase::ColorBg => Phase::ColorBorder,
                        _ => Phase::Summary,
                    };
                    match k.code {
                        KeyCode::Enter => {
                            let val = match cur {
                                Phase::ColorText => app.text_color_input.clone(),
                                Phase::ColorBg => app.bg_color_input.clone(),
                                _ => app.border_color_input.clone(),
                            };
                            if val.is_empty() || valid_hex(&val) {
                                app.hex_error.clear();
                                app.phase = next;
                            } else {
                                app.hex_error =
                                    "Invalid hex color. Use #RRGGBB or leave empty for default."
                                        .into();
                            }
                        }
                        KeyCode::Char(c) => {
                            let upper = c.to_ascii_uppercase();
                            if "0123456789ABCDEF#".contains(upper) {
                                match cur {
                                    Phase::ColorText => app.text_color_input.push(upper),
                                    Phase::ColorBg => app.bg_color_input.push(upper),
                                    _ => app.border_color_input.push(upper),
                                }
                                app.hex_error.clear();
                            }
                        }
                        KeyCode::Backspace => {
                            match cur {
                                Phase::ColorText => app.text_color_input.pop(),
                                Phase::ColorBg => app.bg_color_input.pop(),
                                _ => app.border_color_input.pop(),
                            };
                        }
                        KeyCode::Esc => return Ok(None),
                        _ => {}
                    }
                }

                Phase::Summary => match k.code {
                    KeyCode::Enter => {
                        app.finish_colors();
                        app.profile.save()?;
                        return Ok(Some(app.profile));
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },
            }
        }
    };
}

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
        Phase::CodeEntry => draw_code_entry(f, inner, app),
        Phase::NameEntry => draw_name_entry(f, inner, app),
        Phase::PronounsEntry => draw_pronouns_entry(f, inner, app),
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
        Phase::ColorText | Phase::ColorBg | Phase::ColorBorder => {
            draw_color_entry(f, inner, app)
        }
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
            if dep.contains("webcam") && dep.contains("WSL2") {
                lines.push(Line::styled(
                    format!("    ! {}", dep),
                    Style::new().fg(Color::Yellow),
                ));
                lines.push(Line::raw(""));
                lines.push(Line::raw(
                    "  WSL2 webcam setup (run in Windows PowerShell as Admin):",
                ));
                lines.push(Line::raw("    1. winget install usbipd"));
                lines.push(Line::raw(
                    "    2. usbipd list              # find your camera",
                ));
                lines.push(Line::raw("    3. usbipd bind --busid <X>  # share it"));
                lines.push(Line::raw("    4. usbipd attach --wsl --busid <X>"));
                lines.push(Line::raw(""));
                lines.push(Line::raw("  Then in WSL2:"));
                lines.push(Line::raw(
                    "    5. sudo apt install linux-tools-generic usbip hwdata",
                ));
                lines.push(Line::raw("    6. sudo update-usbids"));
                lines.push(Line::raw(
                    "    7. ls /dev/video*           # should show your camera",
                ));
            } else {
                lines.push(Line::styled(
                    format!("    x {}", dep),
                    Style::new().fg(Color::Red),
                ));
            }
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

fn draw_pronouns_entry(f: &mut Frame, area: Rect, app: &SetupApp) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new("Enter your pronouns (optional)"),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new("shown as a subtitle next to your name."),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(format!(" Pronouns: {}_", app.pronouns_input))
            .style(Style::new().fg(Color::Yellow)),
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

fn draw_color_entry(f: &mut Frame, area: Rect, app: &SetupApp) {
    let label = app.hex_color_name(&app.phase);
    let input = app.current_hex_input(&app.phase);

    let color_preview = hex_preview(input);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(format!("{label} (#RRGGBB)")).style(Style::new().fg(Color::White)),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(format!(" {}_", input)).style(Style::new().fg(Color::Yellow)),
        chunks[2],
    );

    if let Some(c) = color_preview {
        let preview_str = format!("  {label}: {}  ", input);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(&preview_str, Style::new().bg(c)),
            ])),
            chunks[4],
        );
    }

    if !app.hex_error.is_empty() {
        f.render_widget(
            Paragraph::new(format!(" {}", app.hex_error)).style(Style::new().fg(Color::Red)),
            chunks[3],
        );
    }

    f.render_widget(
        Paragraph::new(" Enter = confirm . Esc = cancel")
            .style(Style::new().fg(Color::DarkGray)),
        chunks[5],
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

    let text_preview = hex_preview(&app.profile.text_color);
    let border_preview = hex_preview(&app.profile.border_color);
    let bg_preview = hex_preview(&app.profile.bg_color);

    let mut lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::raw("  Name:    "),
            Span::styled(&app.profile.name, Style::new().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::raw("  Pronouns: "),
            Span::styled(&app.profile.pronouns, Style::new().fg(Color::Cyan)),
        ]),
        Line::raw(""),
    ];

    let input_style = input_name
        .parse::<String>()
        .map(|_| Style::new().fg(Color::Cyan))
        .unwrap_or_else(|_| Style::new().fg(Color::Cyan));
    lines.push(Line::from(vec![
        Span::raw("  Input:   "),
        Span::styled(input_name, input_style),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  Output:  "),
        Span::styled(output_name, Style::new().fg(Color::Cyan)),
    ]));
    lines.push(Line::raw(""));

    lines.push(Line::from(vec![
        Span::raw("  Text:    "),
        Span::styled(
            &app.profile.text_color,
            text_preview.map_or(Style::new().fg(Color::White), |c| Style::new().fg(c)),
        ),
    ]));
    if !app.profile.bg_color.is_empty() {
        lines.push(Line::from(vec![
            Span::raw("  Bg:      "),
            Span::styled(
                &app.profile.bg_color,
                bg_preview.map_or(Style::new().fg(Color::White), |c| Style::new().fg(c)),
            ),
        ]));
    }
    lines.push(Line::from(vec![
        Span::raw("  Border:  "),
        Span::styled(
            &app.profile.border_color,
            border_preview.map_or(Style::new().fg(Color::White), |c| Style::new().fg(c)),
        ),
    ]));

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::raw("  Profile code: "),
        Span::styled(code, Style::new().fg(Color::Green).bold()),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::raw("  Save this code to restore your name on"));
    lines.push(Line::raw("  another machine with: starling setup"));
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "  Enter = save & exit . Esc = cancel",
        Style::new().fg(Color::DarkGray),
    ));

    f.render_widget(Paragraph::new(lines), area);
}
