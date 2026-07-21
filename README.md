# Starling

A federated peer-to-peer communications platform where peers — known as
**birds** — can communicate from anywhere in the world thanks to a
peer-to-peer network called **the murmuration**.

Starling runs in the terminal and provides text chat via gossip protocol
and voice calls via direct QUIC streams. Birds discover each other through
the murmuration using iroh's relay and discovery infrastructure — no central
server required. A room code is all a new bird needs to join a flock.

---

## Getting Starling

There are two ways to get the app. Either way, you'll need [Rust](#installing-rust)
and [system dependencies](#system-dependencies) installed first.

### Option A: Clone with git (recommended)

Gives you the full project including the `justfile` for automated setup:

```bash
git clone https://forgejo.hearthhome.lol/Saltfault/Starling.git
cd Starling
just install-deps    # installs system packages (Linux/macOS)
just run             # builds and starts the app
```

### Option B: Install with cargo (binary only)

Cargo can clone and build the binary in one step — no git clone needed.
The `starling` binary is installed to `~/.cargo/bin/`:

```bash
cargo install --git https://forgejo.hearthhome.lol/Saltfault/Starling.git
```

Then run it directly:

```bash
starling open            # start a new flock
starling join BIRD324524   # join an existing flock
```

> **Note:** You still need to install [system dependencies](#system-dependencies)
> manually with this method since you won't have the `justfile`.

---

## Quick start

Once you have [Rust](#installing-rust), [`just`](#installing-just), and
[system dependencies](#system-dependencies) installed:

```bash
just run             # start a new session (you are the flock opener)
```

Share the room code (shown in the header) with another bird. They join with:

```bash
just join BIRD324524   # join an existing flock
```

When the app starts, a popup asks for your display name (the name other
birds see next to your messages). Type it and press Enter.

Or without `just`:

```bash
cargo run -- open            # start a new session
cargo run -- join BIRD324524   # join an existing session
```

---

## Installing Rust

Starling requires the Rust toolchain (compiler + cargo). Install it with
**rustup**, the official Rust installer.

### Linux / WSL2

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"    # or restart your shell
```

Verify the installation:

```bash
rustc --version
cargo --version
```

### macOS

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Or if you use [Homebrew](https://brew.sh):

```bash
brew install rustup-init
rustup-init
```

### Windows

Download and run [rustup-init.exe](https://win.rustup.rs/x86_64) from the
official site, or use PowerShell:

```powershell
winget install Rustlang.Rustup
```

This installs the MSVC toolchain target by default. You'll also need the
Visual Studio C++ Build Tools — see [System dependencies → Windows](#windows-1)
below.

Verify the installation:

```powershell
rustc --version
cargo --version
```

---

## Installing `just`

The [`justfile`](https://github.com/casey/just) automates dependency
installation and builds. Install `just` with cargo:

```bash
cargo install just
```

Or via your system package manager:

```bash
# Linux (Debian/Ubuntu)
sudo apt install just

# macOS
brew install just

# Windows
winget install Casey.Just
```

You can run `just --list` at any time to see all available commands.

---

## System dependencies

Several Rust crates Starling depends on compile native C/C code (Opus codec,
crypto, audio I/O). These require system-level tools and libraries that cargo
can't install automatically.

### Linux / WSL2

```bash
just install-deps
```

Or manually by distro:

```bash
# Debian / Ubuntu
sudo apt install build-essential cmake pkg-config libasound2-dev libpulse-dev

# Fedora
sudo dnf install gcc cmake pkgconf-pkg-config alsa-lib-devel pulseaudio-libs-devel

# Arch Linux
sudo pacman -S base-devel cmake pkgconf alsa-lib pulseaudio
```

| Package | Why it's needed |
|---------|----------------|
| `build-essential` / `base-devel` | C/C compiler (gcc) for native code |
| `cmake` | Building libopus from source (`audiopus_sys` crate) |
| `pkg-config` | Locating ALSA and PulseAudio libraries at build time |
| `libasound2-dev` | ALSA headers — cpal compiles the ALSA backend on Linux |
| `libpulse-dev` | PulseAudio headers — cpal uses PulseAudio at runtime |

**WSL2 audio:** No extra setup needed. WSLg (Windows 11) provides a
PulseAudio server automatically at `/mnt/wslg/PulseServer`. The app connects
to it directly — no `libasound2-plugins` or `/etc/asound.conf` required.

If you're on an older Windows 10 build without WSLg, you'll need to set up
PulseAudio forwarding manually or use a native Windows build instead.

### Windows

Install the following:

1. **Visual Studio Build Tools 2022** — provides the MSVC C/C compiler.
   Download from [visualstudio.microsoft.com](https://visualstudio.microsoft.com/visual-cpp-build-tools/).
   In the installer, select "Desktop development with C++".

2. **CMake** — required to build the Opus codec from source.
   ```powershell
   winget install Kitware.CMake
   ```
   Or download from [cmake.org/download](https://cmake.org/download/) and add
   it to your `PATH`.

3. **pkg-config** (optional but recommended):
   ```powershell
   winget install pkgconf
   ```

Audio I/O uses WASAPI (Windows Audio Session API) — no extra audio packages
needed.

### macOS

```bash
brew install cmake pkg-config
```

Audio I/O uses CoreAudio — no extra audio packages needed. The Opus codec is
built from source via CMake (same as Linux).

---

## Running Starling

Once Rust and system dependencies are installed:

### Start a new flock

```bash
just run
# or
cargo run -- open
```

The app starts and the header shows a room code:

```
flock: BIRD324524
```

Share this code with another bird so they can join your flock.

### Join an existing flock

```bash
just join BIRD324524
# or
cargo run -- join BIRD324524
```

### Set your name

When you start Starling, a popup asks for your display name — the name
other birds see next to your messages in the flock. Type it and press
Enter to join the murmuration.

There is no need for environment variables or config files.

---

## Keybindings

| Key | Action |
|-----|--------|
| `Enter` | Send typed message |
| `Ctrl+K` | Start call with selected peer / hang up |
| `Ctrl+M` | Toggle mute |
| `Tab` | Cycle selected peer |
| `Backspace` | Delete last character |
| `Esc` | Quit |

---

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│ main.rs (UI loop)                                                │
│   keyboard → Command ──┐                                         │
│   AppEvent ←───────────┤──── mpsc channels ────┐                │
│   playback ← VoiceFrame│                       │                │
└────────────────────────┊────────────────────────┊───────────────┘
                         ▼                        ▼
┌──────────────────────────────────────────────────────────────────┐
│ net.rs (network task)                                            │
│   gossip for chat · QUIC datagrams for voice                     │
│   mic capture (voice.rs) → place_call (call.rs)                  │
└──────────────────────────────────────────────────────────────────┘
```

### Source layout

| File | Responsibility |
|------|---------------|
| `event.rs` | `Command` (UI→net) and `AppEvent` (net→UI) types |
| `net.rs` | Owns the iroh endpoint, gossip subscription, voice handler |
| `call.rs` | Opens/accepts QUIC streams for voice datagrams |
| `voice.rs` | Mic capture: cpal input → Opus encoder → channel |
| `playback.rs` | Audio output: channel → Opus decoder → ring buffer → cpal output |
| `ui.rs` | Terminal rendering and UI state (`App` struct) |
| `main.rs` | Event loop, keyboard handling, wires everything together |

### How the murmuration works

Birds connect to the murmuration through iroh's global relay network and
node discovery. No central server coordinates them:

1. A bird opens a flock by generating a random room code (e.g.
   `BIRD324524`) and subscribing to a gossip topic derived from it via
   SHA-256.
2. Other birds join by entering the same room code — they subscribe to
   the same gossip topic.
3. iroh's relay connects both peers on the topic automatically. No node
   IDs or addresses need to be exchanged.
4. Text messages broadcast over gossip reach all birds in the mesh.
5. Voice calls are direct peer-to-peer QUIC datagram streams — no relay
   needed if direct connectivity is available, with relay fallback.

Audio is encoded as 48 kHz mono Opus, 20 ms frames (960 samples per frame),
sent as QUIC datagrams. Playback uses a 2-second ring buffer to absorb
network jitter.

---

## Troubleshooting

### `cmake not found`

Install CMake (see [System dependencies](#system-dependencies) for your
platform), or run `just install-deps` on Linux.

### `pkg-config failed — alsa development headers are not installed`

Install ALSA headers: `sudo apt install libasound2-dev` (Debian/Ubuntu).

### No microphone / no audio output (WSL2)

Ensure you're running Windows 11 with WSLg enabled. WSLg provides PulseAudio
automatically. If audio doesn't work, verify the PulseAudio server is
accessible:

```bash
pactl info    # should show "Server String: unix:/mnt/wslg/PulseServer"
```

If `pactl` is not found, install it:

```bash
sudo apt install pulseaudio-utils
```

### Build is slow on first compile

The Opus codec is compiled from source via CMake on the first build.
Subsequent builds are cached. expect 2–5 minutes for the initial build.

---

## License

MIT