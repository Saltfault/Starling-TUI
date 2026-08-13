#[cfg(feature = "audio")]
use cpal::traits::HostTrait;
use crossterm::event::{
    self as ct_event, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};
use starling::config::{
    DEFAULT_ACCENT_COLOR, DEFAULT_AUTHOR_COLOR, DEFAULT_BORDER_COLOR, DEFAULT_DIM_COLOR,
    DEFAULT_SELECTION_COLOR, DEFAULT_TEXT_COLOR, Profile,
};
#[cfg(feature = "audio")]
use starling::util::suppress_stderr;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Full,
    Profile,
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
    CameraDevice,
    ColorText,
    ColorBg,
    ColorBorder,
    ColorAccent,
    ColorAuthor,
    ColorSelection,
    ColorDim,
    Settings,
    Summary,
}

struct SetupApp {
    mode: Mode,
    phase: Phase,
    profile: Profile,
    name_input: String,
    pronouns_input: String,
    code_input: String,
    input_devices: Vec<String>,
    output_devices: Vec<String>,
    camera_devices: Vec<String>,
    camera_indices: Vec<u32>,
    selected_input: usize,
    selected_output: usize,
    selected_camera: usize,
    missing_deps: Vec<String>,
    install_cmd: Option<String>,
    install_status: String,
    text_color_input: String,
    bg_color_input: String,
    border_color_input: String,
    accent_color_input: String,
    author_color_input: String,
    selection_color_input: String,
    dim_color_input: String,
    settings_focus: usize,
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

fn normalized_color(input: &str, default: &str) -> String {
    if input.is_empty() {
        default.to_string()
    } else {
        format!("#{}", input.trim_start_matches('#').to_ascii_uppercase())
    }
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
            Mode::Profile | Mode::Settings => Phase::Settings,
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

        let (camera_devices, camera_indices) = list_cameras();
        let selected_camera = profile
            .camera_index
            .and_then(|i| camera_indices.iter().position(|&idx| idx == i))
            .unwrap_or(0);

        let profile_clone = profile.clone();
        Self {
            mode,
            phase,
            name_input: profile.name.clone(),
            pronouns_input: profile.pronouns.clone(),
            code_input: String::new(),
            input_devices,
            output_devices,
            camera_devices,
            camera_indices,
            selected_input,
            selected_output,
            selected_camera,
            profile,
            missing_deps,
            install_cmd,
            install_status: String::new(),
            text_color_input: profile_clone.text_color.clone(),
            bg_color_input: profile_clone.bg_color.clone(),
            border_color_input: profile_clone.border_color.clone(),
            accent_color_input: profile_clone.accent_color.clone(),
            author_color_input: profile_clone.author_color.clone(),
            selection_color_input: profile_clone.selection_color.clone(),
            dim_color_input: profile_clone.dim_color.clone(),
            settings_focus: if mode == Mode::Settings {
                #[cfg(feature = "audio")]
                {
                    2
                }
                #[cfg(not(feature = "audio"))]
                {
                    11
                }
            } else {
                0
            },
            hex_error: String::new(),
        }
    }

    fn hex_color_name(&self, phase: &Phase) -> &str {
        match phase {
            Phase::ColorText => "Text Color",
            Phase::ColorBg => "Background Color",
            Phase::ColorBorder => "Border Color",
            Phase::ColorAccent => "Accent Color",
            Phase::ColorAuthor => "Author Color",
            Phase::ColorSelection => "Selection Color",
            Phase::ColorDim => "Dim Color",
            _ => "",
        }
    }

    fn current_hex_input(&self, phase: &Phase) -> &str {
        match phase {
            Phase::ColorText => &self.text_color_input,
            Phase::ColorBg => &self.bg_color_input,
            Phase::ColorBorder => &self.border_color_input,
            Phase::ColorAccent => &self.accent_color_input,
            Phase::ColorAuthor => &self.author_color_input,
            Phase::ColorSelection => &self.selection_color_input,
            Phase::ColorDim => &self.dim_color_input,
            _ => "",
        }
    }

    fn current_hex_input_mut(&mut self, phase: &Phase) -> &mut String {
        match phase {
            Phase::ColorText => &mut self.text_color_input,
            Phase::ColorBg => &mut self.bg_color_input,
            Phase::ColorBorder => &mut self.border_color_input,
            Phase::ColorAccent => &mut self.accent_color_input,
            Phase::ColorAuthor => &mut self.author_color_input,
            Phase::ColorSelection => &mut self.selection_color_input,
            Phase::ColorDim => &mut self.dim_color_input,
            _ => unreachable!("not a color phase"),
        }
    }

    fn finish_colors(&mut self) {
        self.profile.text_color = normalized_color(&self.text_color_input, DEFAULT_TEXT_COLOR);
        self.profile.bg_color = if self.bg_color_input.is_empty() {
            String::new()
        } else {
            normalized_color(&self.bg_color_input, "")
        };
        self.profile.border_color =
            normalized_color(&self.border_color_input, DEFAULT_BORDER_COLOR);
        self.profile.accent_color =
            normalized_color(&self.accent_color_input, DEFAULT_ACCENT_COLOR);
        self.profile.author_color =
            normalized_color(&self.author_color_input, DEFAULT_AUTHOR_COLOR);
        self.profile.selection_color =
            normalized_color(&self.selection_color_input, DEFAULT_SELECTION_COLOR);
        self.profile.dim_color = normalized_color(&self.dim_color_input, DEFAULT_DIM_COLOR);
    }

    fn settings_text_mut(&mut self) -> Option<&mut String> {
        match self.settings_focus {
            0 => Some(&mut self.name_input),
            1 => Some(&mut self.pronouns_input),
            4 => Some(&mut self.text_color_input),
            5 => Some(&mut self.bg_color_input),
            6 => Some(&mut self.border_color_input),
            7 => Some(&mut self.accent_color_input),
            8 => Some(&mut self.author_color_input),
            9 => Some(&mut self.selection_color_input),
            10 => Some(&mut self.dim_color_input),
            _ => None,
        }
    }

    fn cycle_settings_device(&mut self, direction: isize) {
        let (selected, len) = match self.settings_focus {
            2 => (&mut self.selected_input, self.input_devices.len()),
            3 => (&mut self.selected_output, self.output_devices.len()),
            11 => (&mut self.selected_camera, self.camera_devices.len()),
            _ => return,
        };
        if len > 0 {
            *selected = (*selected as isize + direction).rem_euclid(len as isize) as usize;
        }
    }

    fn focus_bounds(&self) -> (usize, usize) {
        if self.mode == Mode::Profile {
            (0, 1)
        } else {
            #[cfg(feature = "audio")]
            {
                (2, 11)
            }
            #[cfg(not(feature = "audio"))]
            {
                (4, 11)
            }
        }
    }

    fn save_editor(&mut self) -> anyhow::Result<Profile> {
        if self.mode == Mode::Profile && self.name_input.trim().is_empty() {
            anyhow::bail!("Display name cannot be empty");
        }
        if self.mode == Mode::Settings {
            for value in [
                &self.text_color_input,
                &self.border_color_input,
                &self.accent_color_input,
                &self.author_color_input,
                &self.selection_color_input,
                &self.dim_color_input,
            ] {
                if !value.is_empty() && !valid_hex(value) {
                    anyhow::bail!("Colors must use #RRGGBB");
                }
            }
            if !self.bg_color_input.is_empty() && !valid_hex(&self.bg_color_input) {
                anyhow::bail!("Background must use #RRGGBB or be blank");
            }
            self.profile.input_device =
                (self.selected_input > 0).then(|| self.input_devices[self.selected_input].clone());
            self.profile.output_device = (self.selected_output > 0)
                .then(|| self.output_devices[self.selected_output].clone());
            self.profile.camera_index =
                (self.selected_camera > 0).then(|| self.camera_indices[self.selected_camera]);
            self.finish_colors();
        } else {
            self.profile.name = self.name_input.trim().to_string();
            self.profile.pronouns = self.pronouns_input.trim().to_string();
        }
        self.profile.save()?;
        Ok(self.profile.clone())
    }
}

fn command_exists(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
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

        #[cfg(all(target_os = "linux", any(feature = "audio", feature = "video")))]
        {
            if !command_exists("pkg-config") {
                missing.push("pkg-config".into());
            }
            #[cfg(feature = "audio")]
            if !pkg_config_exists("alsa") {
                missing.push("libasound2-dev (ALSA headers)".into());
            }
            #[cfg(feature = "audio")]
            if !pkg_config_exists("libpulse") {
                missing.push("libpulse-dev (PulseAudio headers)".into());
            }
            #[cfg(feature = "video")]
            let has_libclang = std::env::var_os("LIBCLANG_PATH").is_some()
                || std::process::Command::new("llvm-config")
                    .arg("--libdir")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                || command_exists("libclang")
                || std::path::Path::new("/usr/lib/x86_64-linux-gnu/libclang.so").exists()
                || std::path::Path::new("/usr/lib/aarch64-linux-gnu/libclang.so").exists()
                || std::fs::read_dir("/usr/lib/llvm-")
                    .map(|mut d| {
                        d.any(|e| {
                            e.ok()
                                .is_some_and(|f| f.path().join("lib/libclang.so").exists())
                        })
                    })
                    .unwrap_or(false)
                || std::fs::read_dir("/usr/lib")
                    .map(|mut d| {
                        d.any(|e| {
                            e.ok().is_some_and(|f| {
                                f.file_name().to_string_lossy().starts_with("libclang.so")
                            })
                        })
                    })
                    .unwrap_or(false);
            #[cfg(feature = "video")]
            if !has_libclang {
                missing.push("libclang-dev (needed by nokhwa/V4L2 bindgen)".into());
            }
            #[cfg(feature = "video")]
            if !pkg_config_exists("libv4l2") && !pkg_config_exists("v4l-utils") {
                missing.push("libv4l-dev (needed by nokhwa/V4L2)".into());
            }
            #[cfg(feature = "audio")]
            if std::path::Path::new("/mnt/wslg").exists()
                && !std::path::Path::new("/etc/asound.conf").exists()
            {
                missing.push("libasound2-plugins + /etc/asound.conf (WSL2 audio bridge)".into());
            }
            #[cfg(feature = "video")]
            if std::path::Path::new("/mnt/wslg").exists()
                && !std::fs::read_dir("/dev")
                    .map(|mut d| {
                        d.any(|e| {
                            e.ok().is_some_and(|f| {
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
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("cmd").args(["/C", cmd]).status();
    #[cfg(not(target_os = "windows"))]
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
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| std::path::PathBuf::from(home).join(".cargo"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from(".cargo"));
    let git_dir = cargo_home.join("git/db");

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

#[cfg(feature = "video")]
fn list_cameras() -> (Vec<String>, Vec<u32>) {
    let mut names = vec!["Default Camera".to_string()];
    let mut indices = vec![0u32];
    if let Ok(devices) = nokhwa::query(nokhwa::utils::ApiBackend::Auto) {
        for info in devices {
            let idx = info.index().as_index().unwrap_or(indices.len() as u32);
            let desc = info.description();
            let name = if desc.is_empty() {
                format!("Camera {}", idx + 1)
            } else {
                desc.to_string()
            };
            names.push(name);
            indices.push(idx);
        }
    }
    (names, indices)
}

#[cfg(not(feature = "video"))]
fn list_cameras() -> (Vec<String>, Vec<u32>) {
    (vec!["Default Camera".to_string()], vec![0u32])
}

pub fn run_setup(
    term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<Option<Profile>> {
    run_wizard(term, Mode::Full)
}

pub fn run_profile(
    term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<Option<Profile>> {
    run_wizard(term, Mode::Profile)
}

pub fn run_settings(
    term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> anyhow::Result<Option<Profile>> {
    run_wizard(term, Mode::Settings)
}

fn editor_mouse(app: &mut SetupApp, col: u16, row: u16, click: bool) -> Option<bool> {
    let (term_w, term_h) = crossterm::terminal::size().ok()?;
    editor_mouse_at_size(app, term_w, term_h, col, row, click)
}

fn editor_mouse_at_size(
    app: &mut SetupApp,
    term_w: u16,
    term_h: u16,
    col: u16,
    row: u16,
    click: bool,
) -> Option<bool> {
    let width = 88.min(term_w);
    let height = 26.min(term_h);
    let popup_x = (term_w.saturating_sub(width)) / 2;
    let popup_y = (term_h.saturating_sub(height)) / 2;
    let inner_x = popup_x + 2;
    let field_y = popup_y + 3;
    let right_x = inner_x + width.saturating_sub(4) / 2;

    let focus = if app.mode == Mode::Profile && col >= inner_x && col < right_x {
        match row.checked_sub(field_y) {
            Some(0) => Some(0),
            Some(1) => Some(1),
            _ => None,
        }
    } else if app.mode == Mode::Settings {
        if col >= inner_x && col < right_x {
            match row.checked_sub(field_y) {
                Some(0) => Some(2),
                Some(1) => Some(3),
                Some(5) => Some(11),
                _ => None,
            }
        } else if col >= right_x && col < popup_x + width {
            row.checked_sub(field_y)
                .filter(|offset| *offset <= 6)
                .map(|offset| 4 + offset as usize)
        } else {
            None
        }
    } else {
        None
    };
    if let Some(focus) = focus {
        app.settings_focus = focus;
        if click && matches!(focus, 2 | 3 | 11) {
            app.cycle_settings_device(1);
        }
    }

    let footer_y = popup_y + height.saturating_sub(2);
    if click && row == footer_y {
        if col >= inner_x && col < inner_x + 8 {
            return Some(true);
        }
        if col >= inner_x + 9 && col < inner_x + 19 {
            return Some(false);
        }
    }
    None
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
        let event = ct_event::read()?;
        if let Event::Mouse(mouse) = event
            && matches!(app.phase, Phase::Settings)
        {
            let click = mouse.kind == MouseEventKind::Down(MouseButton::Left);
            if matches!(
                mouse.kind,
                MouseEventKind::Moved | MouseEventKind::Down(MouseButton::Left)
            ) && let Some(save) = editor_mouse(&mut app, mouse.column, mouse.row, click)
            {
                if save {
                    match app.save_editor() {
                        Ok(profile) => return Ok(Some(profile)),
                        Err(error) => app.hex_error = error.to_string(),
                    }
                } else {
                    return Ok(None);
                }
            }
            continue;
        }
        if let Event::Key(k) = event {
            if k.kind != KeyEventKind::Press {
                continue;
            }
            match app.phase {
                Phase::Settings => {
                    if k.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(k.code, KeyCode::Char('s' | 'S'))
                    {
                        match app.save_editor() {
                            Ok(profile) => return Ok(Some(profile)),
                            Err(error) => app.hex_error = error.to_string(),
                        }
                        continue;
                    }
                    match k.code {
                        KeyCode::Up | KeyCode::BackTab => {
                            let (min, _) = app.focus_bounds();
                            app.settings_focus = app.settings_focus.saturating_sub(1).max(min);
                        }
                        KeyCode::Down | KeyCode::Tab | KeyCode::Enter => {
                            let (_, max) = app.focus_bounds();
                            app.settings_focus = (app.settings_focus + 1).min(max);
                        }
                        KeyCode::Left => app.cycle_settings_device(-1),
                        KeyCode::Right => app.cycle_settings_device(1),
                        KeyCode::Backspace => {
                            if let Some(value) = app.settings_text_mut() {
                                value.pop();
                            }
                            app.hex_error.clear();
                        }
                        KeyCode::Char(c) if !c.is_control() => {
                            let focus = app.settings_focus;
                            if let Some(value) = app.settings_text_mut() {
                                if focus >= 4 {
                                    let upper = c.to_ascii_uppercase();
                                    if "0123456789ABCDEF#".contains(upper) && value.len() < 7 {
                                        value.push(upper);
                                    }
                                } else if value.len() < 64 {
                                    value.push(c);
                                }
                            }
                            app.hex_error.clear();
                        }
                        KeyCode::Esc => return Ok(None),
                        _ => {}
                    }
                }

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
                        if app.code_input.is_empty() {
                            app.install_status.clear();
                            app.phase = Phase::NameEntry;
                        } else if let Some(profile) = Profile::from_code(app.code_input.trim()) {
                            app.name_input = profile.name.clone();
                            app.profile = profile;
                            app.install_status.clear();
                            app.phase = Phase::NameEntry;
                        } else {
                            app.install_status =
                                "Invalid profile code (expected 32 hex digits).".into();
                        }
                    }
                    KeyCode::Char(c) if !c.is_control() && app.code_input.len() < 256 => {
                        app.code_input.push(c)
                    }
                    KeyCode::Backspace => {
                        app.code_input.pop();
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },

                Phase::NameEntry => match k.code {
                    KeyCode::Enter if !app.name_input.trim().is_empty() => {
                        app.profile.name = app.name_input.trim().to_string();
                        app.phase = Phase::PronounsEntry;
                    }
                    KeyCode::Char(c) if !c.is_control() && app.name_input.len() < 64 => {
                        app.name_input.push(c)
                    }
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
                    KeyCode::Char(c) if !c.is_control() && app.pronouns_input.len() < 64 => {
                        app.pronouns_input.push(c)
                    }
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
                        app.phase = Phase::CameraDevice;
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

                Phase::CameraDevice => match k.code {
                    KeyCode::Enter => {
                        app.profile.camera_index = (app.selected_camera > 0)
                            .then(|| app.camera_indices[app.selected_camera]);
                        app.phase = Phase::ColorText;
                    }
                    KeyCode::Up => {
                        if app.selected_camera > 0 {
                            app.selected_camera -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if app.selected_camera + 1 < app.camera_devices.len() {
                            app.selected_camera += 1;
                        }
                    }
                    KeyCode::Esc => return Ok(None),
                    _ => {}
                },

                Phase::ColorText
                | Phase::ColorBg
                | Phase::ColorBorder
                | Phase::ColorAccent
                | Phase::ColorAuthor
                | Phase::ColorSelection
                | Phase::ColorDim => {
                    let cur = app.phase.clone();
                    let next = match cur {
                        Phase::ColorText => Phase::ColorBg,
                        Phase::ColorBg => Phase::ColorBorder,
                        Phase::ColorBorder => Phase::ColorAccent,
                        Phase::ColorAccent => Phase::ColorAuthor,
                        Phase::ColorAuthor => Phase::ColorSelection,
                        Phase::ColorSelection => Phase::ColorDim,
                        Phase::ColorDim => Phase::Summary,
                        _ => unreachable!(),
                    };
                    match k.code {
                        KeyCode::Enter => {
                            let val = app.current_hex_input(&cur);
                            if val.is_empty() || valid_hex(val) {
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
                            if "0123456789ABCDEF#".contains(upper)
                                && app.current_hex_input(&cur).len() < 7
                            {
                                app.current_hex_input_mut(&cur).push(upper);
                                app.hex_error.clear();
                            }
                        }
                        KeyCode::Backspace => {
                            app.current_hex_input_mut(&cur).pop();
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
    }
}

fn draw(f: &mut Frame, app: &SetupApp) {
    let area = f.area();
    f.render_widget(Clear, area);

    let width = 88.min(area.width);
    let height = 26.min(area.height);
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );

    f.render_widget(Clear, popup);
    let title = match app.mode {
        Mode::Profile => " Starling Profile ",
        Mode::Settings => " Starling Settings ",
        Mode::Full => " Starling Setup ",
    };
    f.render_widget(Block::default().borders(Borders::ALL).title(title), popup);

    let inner = popup.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });

    match app.phase {
        Phase::Settings => draw_settings(f, inner, app),
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
        Phase::CameraDevice => draw_device_list(
            f,
            inner,
            "Camera (Webcam)",
            &app.camera_devices,
            app.selected_camera,
        ),
        Phase::ColorText
        | Phase::ColorBg
        | Phase::ColorBorder
        | Phase::ColorAccent
        | Phase::ColorAuthor
        | Phase::ColorSelection
        | Phase::ColorDim => draw_color_entry(f, inner, app),
        Phase::Summary => draw_summary(f, inner, app),
    }
}

fn settings_line(
    focused: bool,
    label: &str,
    value: String,
    preview: Option<Color>,
) -> Line<'static> {
    let marker = if focused { ">" } else { " " };
    let label_style = if focused {
        Style::new().fg(Color::Yellow).bold()
    } else {
        Style::new().fg(Color::DarkGray)
    };
    let value_style = preview.map_or(Style::new().fg(Color::White), |color| {
        Style::new().fg(color)
    });
    Line::from(vec![
        Span::styled(format!("{marker} {label:<11}"), label_style),
        Span::styled(value, value_style),
    ])
}

fn draw_settings(f: &mut Frame, area: Rect, app: &SetupApp) {
    let rows = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);
    let columns =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[0]);

    #[cfg(feature = "audio")]
    let input = app
        .input_devices
        .get(app.selected_input)
        .cloned()
        .unwrap_or_else(|| "System Default".into());
    #[cfg(feature = "audio")]
    let output = app
        .output_devices
        .get(app.selected_output)
        .cloned()
        .unwrap_or_else(|| "System Default".into());
    let left = if app.mode == Mode::Profile {
        vec![
            Line::styled(" Profile", Style::new().fg(Color::Cyan).bold()),
            Line::raw(""),
            settings_line(
                app.settings_focus == 0,
                "Name",
                app.name_input.clone(),
                None,
            ),
            settings_line(
                app.settings_focus == 1,
                "Pronouns",
                app.pronouns_input.clone(),
                None,
            ),
        ]
    } else {
        let camera = app
            .camera_devices
            .get(app.selected_camera)
            .cloned()
            .unwrap_or_else(|| "Default Camera".into());
        let mut left_lines = Vec::new();
        #[cfg(feature = "audio")]
        {
            left_lines.push(Line::styled(" Audio", Style::new().fg(Color::Cyan).bold()));
            left_lines.push(Line::raw(""));
            left_lines.push(settings_line(app.settings_focus == 2, "Input", input, None));
            left_lines.push(settings_line(
                app.settings_focus == 3,
                "Output",
                output,
                None,
            ));
            left_lines.push(Line::raw(""));
        }
        left_lines.push(Line::styled(" Video", Style::new().fg(Color::Cyan).bold()));
        left_lines.push(Line::raw(""));
        left_lines.push(settings_line(
            app.settings_focus == 11,
            "Camera",
            camera,
            None,
        ));
        left_lines.push(Line::raw(""));
        left_lines.push(Line::styled(
            " Click or Left/Right to change",
            Style::new().fg(Color::DarkGray),
        ));
        left_lines
    };
    let right = vec![
        Line::styled(" Theme", Style::new().fg(Color::Cyan).bold()),
        Line::raw(""),
        settings_line(
            app.settings_focus == 4,
            "Text",
            app.text_color_input.clone(),
            hex_preview(&app.text_color_input),
        ),
        settings_line(
            app.settings_focus == 5,
            "Background",
            if app.bg_color_input.is_empty() {
                "none".into()
            } else {
                app.bg_color_input.clone()
            },
            hex_preview(&app.bg_color_input),
        ),
        settings_line(
            app.settings_focus == 6,
            "Border",
            app.border_color_input.clone(),
            hex_preview(&app.border_color_input),
        ),
        settings_line(
            app.settings_focus == 7,
            "Accent",
            app.accent_color_input.clone(),
            hex_preview(&app.accent_color_input),
        ),
        settings_line(
            app.settings_focus == 8,
            "Author",
            app.author_color_input.clone(),
            hex_preview(&app.author_color_input),
        ),
        settings_line(
            app.settings_focus == 9,
            "Selection",
            app.selection_color_input.clone(),
            hex_preview(&app.selection_color_input),
        ),
        settings_line(
            app.settings_focus == 10,
            "Dim",
            app.dim_color_input.clone(),
            hex_preview(&app.dim_color_input),
        ),
    ];

    f.render_widget(Paragraph::new(left), columns[0]);
    if app.mode == Mode::Settings {
        f.render_widget(Paragraph::new(right), columns[1]);
    }
    if !app.hex_error.is_empty() {
        f.render_widget(
            Paragraph::new(app.hex_error.as_str()).style(Style::new().fg(Color::Red)),
            rows[1],
        );
    }
    f.render_widget(
        Paragraph::new("[ Save ]  [ Cancel ]   Mouse or Tab = focus . Ctrl+S = save")
            .style(Style::new().fg(Color::DarkGray)),
        rows[2],
    );
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
    if !app.install_status.is_empty() {
        f.render_widget(
            Paragraph::new(app.install_status.as_str()).style(Style::new().fg(Color::Red)),
            chunks[4],
        );
    }
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

    f.render_widget(Paragraph::new("Enter your pronouns (optional)"), chunks[0]);
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
            Paragraph::new(Line::from(vec![Span::styled(
                &preview_str,
                Style::new().bg(c),
            )])),
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
        Paragraph::new(" Enter = confirm . Esc = cancel").style(Style::new().fg(Color::DarkGray)),
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
    let accent_preview = hex_preview(&app.profile.accent_color);
    let author_preview = hex_preview(&app.profile.author_color);
    let selection_preview = hex_preview(&app.profile.selection_color);
    let dim_preview = hex_preview(&app.profile.dim_color);

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
    lines.push(Line::from(vec![
        Span::raw("  Accent:  "),
        Span::styled(
            &app.profile.accent_color,
            accent_preview.map_or(Style::new().fg(Color::White), |c| Style::new().fg(c)),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  Author:  "),
        Span::styled(
            &app.profile.author_color,
            author_preview.map_or(Style::new().fg(Color::White), |c| Style::new().fg(c)),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  Select:  "),
        Span::styled(
            &app.profile.selection_color,
            selection_preview.map_or(Style::new().fg(Color::White), |c| Style::new().fg(c)),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  Dim:     "),
        Span::styled(
            &app.profile.dim_color,
            dim_preview.map_or(Style::new().fg(Color::White), |c| Style::new().fg(c)),
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

#[cfg(test)]
mod tests {
    use super::{Mode, SetupApp, editor_mouse_at_size, normalized_color};

    #[test]
    fn blank_color_restores_default() {
        assert_eq!(normalized_color("", "#123456"), "#123456");
    }

    #[test]
    fn profile_and_settings_support_mouse_focus() {
        let mut profile = SetupApp::new(Mode::Profile);
        editor_mouse_at_size(&mut profile, 100, 40, 10, 11, false);
        assert_eq!(profile.settings_focus, 1);

        let mut settings = SetupApp::new(Mode::Settings);
        editor_mouse_at_size(&mut settings, 100, 40, 55, 13, false);
        assert_eq!(settings.settings_focus, 7);
        assert_eq!(
            editor_mouse_at_size(&mut settings, 100, 40, 10, 31, true),
            Some(true)
        );
    }

    #[test]
    fn entered_color_is_normalized() {
        assert_eq!(normalized_color("abcdef", "#123456"), "#ABCDEF");
        assert_eq!(normalized_color("#ABCDEF", "#123456"), "#ABCDEF");
    }
}
