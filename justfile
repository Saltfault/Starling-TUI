# Starling — build & run helpers
#
# Usage:
#   just install-deps       # one-time: install all system packages
#   just setup-wsl-audio    # one-time: WSL2 only — enables voice calls
#   just run                # check deps, then run the app
#   just build              # check deps, then build
#   just join BIRD123456    # join an existing flock

# Install all system dependencies needed to build and run starling.
# Detects the OS/distro and uses the appropriate package manager.
install-deps:
    @if command -v apt-get >/dev/null 2>&1; then \
        echo "Detected Debian/Ubuntu/WSL — installing..."; \
        sudo apt-get update && sudo apt-get install -y \
            build-essential cmake pkg-config libasound2-dev libpulse-dev; \
    elif command -v dnf >/dev/null 2>&1; then \
        echo "Detected Fedora — installing..."; \
        sudo dnf install -y \
            gcc cmake pkgconf-pkg-config alsa-lib-devel pulseaudio-libs-devel; \
    elif command -v pacman >/dev/null 2>&1; then \
        echo "Detected Arch — installing..."; \
        sudo pacman -S --noconfirm base-devel cmake pkgconf alsa-lib pulseaudio; \
    elif command -v brew >/dev/null 2>&1; then \
        echo "Detected macOS (Homebrew) — installing..."; \
        brew install cmake pkg-config; \
    else \
        echo "Could not detect a supported package manager."; \
        echo ""; \
        echo "Please install manually:"; \
        echo "  Linux:  gcc, cmake, pkg-config, alsa-lib-dev, pulseaudio-dev"; \
        echo "  macOS:  cmake, pkg-config (via Homebrew)"; \
        echo "  Windows: Visual Studio Build Tools + CMake (see README)"; \
        exit 1; \
    fi

# Configure ALSA to route through PulseAudio. WSL2 only.
# This is required for voice calls to work on WSL2 — the pure-Rust
# PulseAudio crate that cpal uses can't authenticate with WSLg's server,
# but the C library (libpulse) that ALSA's pulse plugin uses can.
setup-wsl-audio:
    #!/usr/bin/env bash
    if [ ! -d /mnt/wslg ]; then
        echo "This command is for WSL2 only (no /mnt/wslg found)."
        echo "On native Linux, PulseAudio works without this step."
        exit 1
    fi

    echo "Installing libasound2-plugins (ALSA -> PulseAudio bridge)..."
    sudo apt-get update && sudo apt-get install -y libasound2-plugins

    echo "Writing /etc/asound.conf..."
    sudo tee /etc/asound.conf > /dev/null << 'ASOUNDCONF'
pcm.!default pulse
ctl.!default pulse
ASOUNDCONF

    echo ""
    echo "Done! Voice calls should now work."
    echo "Verify with: pactl info  (may need: sudo apt install pulseaudio-utils)"

# Check that all build prerequisites are present before running cargo.
# Prints clear messages for anything missing.
check-deps:
    #!/usr/bin/env bash
    missing=0
    for tool in cmake cc; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            echo "✗ '$tool' not found"
            missing=1
        fi
    done

    # Platform-specific library checks (only on Linux)
    if [ "$(uname -s)" = "Linux" ]; then
        if command -v pkg-config >/dev/null 2>&1; then
            if ! pkg-config --exists alsa 2>/dev/null; then
                echo "✗ ALSA development headers not found (install libasound2-dev)"
                missing=1
            fi
            if ! pkg-config --exists libpulse 2>/dev/null; then
                echo "✗ PulseAudio development headers not found (install libpulse-dev)"
                missing=1
            fi
        else
            echo "✗ pkg-config not found"
            missing=1
        fi
    fi

    if [ "$missing" -ne 0 ]; then
        echo ""
        echo "Run 'just install-deps' to install everything."
        exit 1
    fi
    echo "✓ All build dependencies present"

# Build the project (checks deps first).
build: check-deps
    cargo build

# Run the app — starts a new flock with a random room code.
run: check-deps
    cargo run -- open

# Join an existing flock with a room code.
join code: check-deps
    cargo run -- join {{code}}

# Run cargo check (fast, no binary output).
check: check-deps
    cargo check