# Starling — build & run helpers
#
# Usage:
#   just install-deps       # one-time: install all system packages
#   just setup-wsl-audio    # one-time: WSL2 only — enables voice calls
#   just run                # check deps, then run the app
#   just build              # check deps, then build
#   just join BIRD00CCFF    # join an existing flock

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
    echo 'pcm.!default pulse' | sudo tee /etc/asound.conf > /dev/null
    echo 'ctl.!default pulse' | sudo tee -a /etc/asound.conf > /dev/null

    echo ""
    echo "Done! Voice calls should now work."
    echo "Verify with: pactl info  (may need: sudo apt install pulseaudio-utils)"

check-deps:
    #!/usr/bin/env bash
    missing=0
    for tool in cmake cc; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            echo "✗ '$tool' not found"
            missing=1
        fi
    done

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

build: check-deps
    cargo build

run: check-deps
    cargo run -- open

join code: check-deps
    cargo run -- join {{code}}

check: check-deps
    cargo check