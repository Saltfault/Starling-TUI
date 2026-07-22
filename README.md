# Starling TUI

> This is the **terminal (TUI) client** for Starling — a federated
> peer-to-peer communications platform. For the main project, see the
> [Starling repository](https://forgejo.hearthhome.lol/Saltfault/Starling).

A federated peer-to-peer communications platform where peers — known as
**birds** — can communicate from anywhere in the world thanks to a
peer-to-peer network called **the murmuration**.

Starling TUI runs in the terminal and provides text chat, voice calls, and
video calls — all end-to-end encrypted. Birds discover each other through
the murmuration using iroh's relay and discovery infrastructure — no central
server required. A room code is all a new bird needs to join a flock.

## Platform support

| Feature | Windows | macOS | Linux | WSL2 |
|---------|:-------:|:-----:|:-----:|:----:|
| Text chat | ✓ | ✓ | ✓ | ✓ |
| Voice calls (mic + playback) | ✓ | ✓ | ✓ | ✓† |
| Video calls (webcam) | ✓ | ✓ | ✓ | — |
| Room codes | ✓ | ✓ | ✓ | ✓ |
| Persistent identity | ✓ | ✓ | ✓ | ✓ |

† WSL2 voice requires a one-time setup step (`starling setup` or
`just setup`) that installs the ALSA→PulseAudio bridge. See
[WSL2 setup](#wsl2-windows-subsystem-for-linux) below.

WSL2 does not expose webcams by default — use a native Windows build for
video calls.

---

## Getting started

**Install Starling:**

```bash
cargo install --git https://forgejo.hearthhome.lol/Saltfault/Starling-TUI.git
```

**Configure your profile (one-time):**

```bash
starling setup
```

This opens a setup wizard where you enter your display name, select your
microphone and speaker, and get a 32-digit profile code. The code encodes
your name and can be used to restore your profile on another machine. The
profile is saved to disk automatically.

**Run it:**

```bash
starling open
```

The header shows a room code like `▀▄ BIRD-00CCFF-00CCFF-...` with colored
half-block swatches. Share it with another bird — they join with:

```bash
starling join BIRD-00CCFF-00CCFF-...
```

> **Developing?** You can also clone and run from source:
> ```bash
> git clone https://forgejo.hearthhome.lol/Saltfault/Starling-TUI.git
> cd Starling-TUI
> cargo run -- open
> ```
> The `justfile` provides `just install-deps`, `just setup`, `just run`,
> and `just join <code>` as shortcuts.

---

## Platform setup

Before installing Starling, you need Rust and a C compiler. Follow the
section for your platform. Then run `starling setup` (or `just setup`) to
configure your profile, audio devices, and any platform-specific dependencies.

### Windows

**1. Install Visual Studio C++ Build Tools** (provides the MSVC compiler):

Download from [visualstudio.microsoft.com](https://visualstudio.microsoft.com/visual-cpp-build-tools/).
In the installer, select **"Desktop development with C++"**.

**2. Install Rust:**

Download and run [rustup-init.exe](https://win.rustup.rs/x86_64) from the
Rust website. This installs `rustc`, `cargo`, and everything needed to
build Rust projects.

> `winget install Rustlang.Rustup` is not recommended — it does not
> install `cargo` correctly on all systems.

**3. Install Starling:**

```powershell
cargo install --git https://forgejo.hearthhome.lol/Saltfault/Starling-TUI.git
```

> **Pre-built binaries** (no Rust or compiler required) are planned —
> see the [releases page](https://forgejo.hearthhome.lol/Saltfault/Starling-TUI/releases)
> for available downloads.

**4. Run:**

```powershell
starling open
```

Audio uses WASAPI (Windows Audio Session API) — works out of the box, no
extra audio packages needed. Video uses Windows Media Foundation via nokhwa.

### macOS

**1. Install Xcode Command Line Tools** (provides the C compiler):

```bash
xcode-select --install
```

**2. Install [Homebrew](https://brew.sh)** if you don't have it:

```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

**3. Install Rust:**

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

**4. Install Starling:**

```bash
cargo install --git https://forgejo.hearthhome.lol/Saltfault/Starling-TUI.git
```

**5. Run:**

```bash
starling open
```

Audio uses CoreAudio — works out of the box, no extra audio packages needed.
Video uses AVFoundation via nokhwa.

### Linux (native)

**1. Install system dependencies:**

```bash
# Debian / Ubuntu
sudo apt install build-essential pkg-config libasound2-dev libpulse-dev libclang-dev

# Fedora
sudo dnf install gcc pkgconf-pkg-config alsa-lib-devel pulseaudio-libs-devel clang-devel

# Arch Linux
sudo pacman -S base-devel pkgconf alsa-lib pulseaudio clang
```

**2. Install Rust:**

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

**3. Install Starling:**

```bash
cargo install --git https://forgejo.hearthhome.lol/Saltfault/Starling-TUI.git
```

**4. Run:**

```bash
starling open
```

Audio uses PulseAudio (with ALSA fallback) — works out of the box on most
Linux desktops. Video uses V4L2 via nokhwa.

| Package | Why it's needed |
|---------|----------------|
| `build-essential` / `base-devel` | C compiler (gcc) for native code |
| `pkg-config` | Locating ALSA and PulseAudio libraries at build time |
| `libasound2-dev` | ALSA headers — cpal compiles the ALSA backend on Linux |
| `libpulse-dev` | PulseAudio headers — cpal's preferred backend at runtime |
| `libclang-dev` | Required by nokhwa for V4L2 webcam support |

### WSL2 (Windows Subsystem for Linux)

WSL2 setup is identical to Linux, with one extra step for audio.

**1. Install WSL2** (if not already installed, from PowerShell):

```powershell
wsl --install
```

**2. Inside WSL, install system dependencies:**

```bash
sudo apt update
sudo apt install build-essential pkg-config libasound2-dev libpulse-dev libclang-dev
```

**3. Install Rust:**

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

**4. Install `just` and run setup:**

```bash
cargo install just
just setup
```

`just setup` (or `starling setup`) installs `libasound2-plugins` and writes
`/etc/asound.conf` to route ALSA through PulseAudio. This is needed because
the pure-Rust PulseAudio crate that cpal uses can't authenticate with WSLg's
server, but the C library (`libpulse`) that ALSA's pulse plugin uses can.

If you skip this step, text chat works but voice calls won't.

**5. Install Starling:**

```bash
cargo install --git https://forgejo.hearthhome.lol/Saltfault/Starling-TUI.git
```

**6. Run:**

```bash
starling open
```

If you're on an older Windows 10 build without WSLg, audio won't work in
WSL2 — use a [native Windows build](#windows) instead.

**Webcam on WSL2:** Webcams are not exposed by default. To use video calls
from WSL2, set up USB passthrough (run in Windows PowerShell as Admin):

```powershell
winget install usbipd
usbipd list                    # find your camera's BUSID
usbipd bind --busid <BUSID>
usbipd attach --wsl --busid <BUSID>
```

Then in WSL2:

```bash
sudo apt install linux-tools-generic usbip hwdata
sudo update-usbids
ls /dev/video*                  # should show your camera
```

Alternatively, use a [native Windows build](#windows) for video calls.

---

## Running Starling

### Start a new flock

```bash
starling open
```

The app starts and the header shows a room code with color swatches:

```
▀▄ ▀▄ ▀▄ ▀▄ ▀▄ ▀▄ ▀▄ ▀▄ ▀▄ ▀▄ ▀▄  BIRD-00CCFF-00CCFF-00CCFF-00CCFF-00CCFF-00CCFF-00CCFF-00CCFF-00CCFF-00CCFF-00CCFF
```

Share this code with another bird so they can join your flock.

### Join an existing flock

```bash
starling join BIRD-00CCFF-00CCFF-...
```

### Join multiple flocks

Once inside the app, you can join additional flocks at any time by typing
`/join <code>` in the message input and pressing Enter. A flock rail appears
on the left side of the screen showing all joined flocks. Use `Alt+↑` and
`Alt+↓` to switch between them. Each flock has its own message list and
end-to-end encryption key.

```
 flocks              ╔════════════════════════════════════╗
╔════════╗           ║ BIRD-00CCFF-... . 3 birds          ║
║> BIRD-…║           ║ Alice: hello!    ╔════════════════╗║
║  BIRD-…║           ║ Bob: hi there   ║ birds           ║║
╚════════╝           ╚═════════════════╩══════════════════╝
```

You start in your home flock automatically. Joining a new flock does not
leave the current one — you remain subscribed to all of them simultaneously.

### Set your name

When you start Starling for the first time, a popup asks for your display
name — the name other birds see next to your messages in the flock. Type it
and press Enter to join the murmuration. You can change it later with
`starling setup`.

### Logs

Errors are written to `logs/latest.log`. On each launch, the previous log is
gzipped to `logs/<timestamp>.log.gz`. Check this file if something isn't
working.

---

## Keybindings

| Key | Action |
|-----|--------|
| `Enter` | Send typed message (or `/join <code>` to join a flock) |
| `Alt+↑` | Switch to previous flock |
| `Alt+↓` | Switch to next flock |
| `Ctrl+K` | Start call with selected peer / hang up |
| `Ctrl+M` | Toggle mute |
| `Ctrl+V` | Toggle video |
| `Tab` | Cycle selected peer |
| `i` | Show invite popup |
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
│   video_frame ← VideoFrame                     │                │
└────────────────────────┊────────────────────────┊───────────────┘
                         ▼                        ▼
┌──────────────────────────────────────────────────────────────────┐
│ net.rs (network task)                                            │
│   gossip for chat · QUIC datagrams for voice                     │
│   QUIC uni streams for video                                     │
│   mic capture (voice.rs) → place_call (call.rs)                  │
│   webcam capture (video.rs) → place_video (call.rs)              │
└──────────────────────────────────────────────────────────────────┘
```

### Source layout

| File | Responsibility |
|------|---------------|
| `main.rs` | Event loop, keyboard handling, subcommand dispatch |
| `event.rs` | `Command` (UI→net) and `AppEvent` (net→UI) types |
| `net.rs` | Owns the iroh endpoint, flock map, voice/video handlers |
| `call.rs` | Opens/accepts QUIC streams for voice datagrams and video |
| `voice.rs` | Mic capture: cpal input → Opus encoder → channel |
| `playback.rs` | Audio output: channel → Opus decoder → ring buffer → cpal output |
| `video.rs` | Webcam capture: nokhwa → JPEG frames → channel, terminal rendering |
| `opus_ffi.rs` | Safe Rust wrappers around the pre-built Opus C library |
| `ui.rs` | Terminal rendering and UI state (`App` struct) |
| `setup.rs` | Setup wizard TUI for profile and device configuration |
| `config.rs` | Profile struct, disk persistence, 32-digit code, persistent identity key |
| `crypto.rs` | E2E encryption (ChaCha20-Poly1305) for gossip messages |
| `logger.rs` | File logger with gzipped log rotation |
| `util.rs` | Platform utilities (stderr suppression on Unix) |
| `build.rs` | Downloads pre-built Opus static libraries from shiguredo/opus-rs |

### How the murmuration works

Birds connect to the murmuration through iroh's global relay network and
node discovery. No central server coordinates them:

1. A bird opens a flock — their persistent node ID becomes the room code
   (e.g. `BIRD-00CCFF-00CCFF-...`). The gossip topic and E2E encryption key
   are both derived from this code via SHA-256.
2. Other birds join by entering the same room code — they subscribe to the
   same gossip topic and derive the same encryption key.
3. iroh's relay connects both peers on the topic automatically. No node
   IDs or addresses need to be exchanged beyond the room code.
4. Text messages broadcast over gossip reach all birds in the mesh.
5. Voice calls are direct peer-to-peer QUIC datagram streams — no relay
   needed if direct connectivity is available, with relay fallback.
6. Video calls use QUIC unidirectional streams carrying JPEG frames.

Audio is encoded as 48 kHz stereo Opus, 20 ms frames (960 samples per
channel), sent as QUIC datagrams. Playback uses a 2-second ring buffer to
absorb network jitter.

All text messages are end-to-end encrypted with ChaCha20-Poly1305 using a
key derived from the room code. Each flock gets its own encryption key, so
messages from different flocks are isolated cryptographically. Voice and
video calls are E2E encrypted via iroh's QUIC TLS 1.3. Relays and
intermediaries cannot read message content.

### Persistent identity

Starling saves your node's secret key to `~/.config/starling/identity.key`
on first launch. This means your room code stays the same every time you
open a flock — other birds can bookmark your code and rejoin later without
you needing to share a new one.

---

## Troubleshooting

### No microphone / no audio output (WSL2)

Run the one-time audio setup:

```bash
starling setup
# or: just setup
```

This installs `libasound2-plugins` and writes `/etc/asound.conf` to route
ALSA through PulseAudio. See [WSL2 setup](#wsl2-windows-subsystem-for-linux)
for details.

If it still doesn't work, verify PulseAudio is running:

```bash
ls /mnt/wslg/PulseServer   # should exist
echo $PULSE_SERVER          # should show unix:/mnt/wslg/PulseServer
```

If you don't have WSLg (older Windows 10), audio won't work in WSL2 —
use a [native Windows build](#windows) instead.

### `link.exe not found` (Windows)

You need the Visual Studio C++ Build Tools. Reinstall them and make sure
"Desktop development with C++" is selected.

### `libclang` not found (Linux / WSL2)

```bash
sudo apt install libclang-dev   # Debian/Ubuntu
sudo dnf install clang-devel    # Fedora
sudo pacman -S clang            # Arch
```

This is required by nokhwa for webcam support.

### No webcam detected (WSL2)

WSL2 doesn't expose USB webcams by default. Set up USB passthrough:

In Windows PowerShell (as Admin):
```powershell
winget install usbipd
usbipd list                     # find your camera's BUSID
usbipd bind --busid <BUSID>
usbipd attach --wsl --busid <BUSID>
```

In WSL2:
```bash
sudo apt install linux-tools-generic usbip hwdata
sudo update-usbids
ls /dev/video*                   # should show your camera
```

Then re-run `starling setup` to detect the camera.

### Build is slow on first compile

The Opus codec is downloaded as a pre-built static library on the first
build. Subsequent builds are cached. Expect 2–5 minutes for the initial
build.

### Check the logs

Errors are written to `logs/latest.log`. On each launch, the previous log is
gzipped to `logs/<timestamp>.log.gz`.

---

## License

Apache 2.0
