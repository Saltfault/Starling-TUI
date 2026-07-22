# Starling — build & run helpers
#
# Usage:
#   just install-deps       # one-time: install all system packages
#   just setup              # configure profile, audio devices, and deps
#   just run                # check deps, then run the app
#   just build              # check deps, then build
#   just join BIRD00CCFF    # join an existing flock

install-deps:
    @if command -v apt-get >/dev/null 2>&1; then \
        echo "Detected Debian/Ubuntu/WSL — installing..."; \
        sudo apt-get update && sudo apt-get install -y \
            build-essential pkg-config libasound2-dev libpulse-dev libclang-dev libv4l-dev; \
        if [ -d /mnt/wslg ] && [ ! -f /etc/asound.conf ]; then \
            echo "Setting up WSL2 audio bridge..."; \
            sudo apt-get install -y libasound2-plugins; \
            printf 'pcm.!default {\ntype pulse\n}\nctl.!default {\ntype pulse\n}\n' | sudo tee /etc/asound.conf > /dev/null; \
            echo "WSL2 audio bridge installed."; \
        fi; \
    elif command -v dnf >/dev/null 2>&1; then \
        echo "Detected Fedora — installing..."; \
        sudo dnf install -y \
            gcc pkgconf-pkg-config alsa-lib-devel pulseaudio-libs-devel clang-devel; \
    elif command -v pacman >/dev/null 2>&1; then \
        echo "Detected Arch — installing..."; \
        sudo pacman -S --noconfirm base-devel pkgconf alsa-lib pulseaudio clang; \
    elif command -v brew >/dev/null 2>&1; then \
        echo "Detected macOS (Homebrew) — installing..."; \
        brew install pkg-config; \
    else \
        echo "Could not detect a supported package manager."; \
        echo ""; \
        echo "Please install manually:"; \
        echo "  Linux:  gcc, pkg-config, alsa-lib-dev, pulseaudio-dev, libclang-dev"; \
        echo "  macOS:  pkg-config (via Homebrew)"; \
        echo "  Windows: Visual Studio Build Tools (see README)"; \
        exit 1; \
    fi

check-deps:
    #!/usr/bin/env bash
    missing=0
    for tool in cc; do
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
        if ! ls /usr/lib/x86_64-linux-gnu/libclang.so* >/dev/null 2>&1 && ! ls /usr/lib/llvm-*/lib/libclang.so* >/dev/null 2>&1 && ! ls /usr/lib/aarch64-linux-gnu/libclang.so* >/dev/null 2>&1; then
            echo "✗ libclang not found (install libclang-dev for nokhwa/V4L2)"
            missing=1
        fi
        if command -v pkg-config >/dev/null 2>&1; then
            if ! pkg-config --exists libv4l2 2>/dev/null && ! pkg-config --exists v4l-utils 2>/dev/null; then
                echo "✗ libv4l not found (install libv4l-dev for nokhwa/V4L2)"
                missing=1
            fi
        fi
        # WSL2 audio bridge check
        if [ -d /mnt/wslg ] && [ ! -f /etc/asound.conf ]; then
            echo "✗ WSL2 audio bridge not configured (run 'just setup' or 'starling setup')"
            missing=1
        fi
    fi

    if [ "$missing" -ne 0 ]; then
        echo ""
        echo "Run 'just install-deps' to install system packages,"
        echo "then 'just setup' to configure audio and profile."
        exit 1
    fi
    echo "✓ All build dependencies present"

build: check-deps
    cargo build

setup: install-deps
    cargo build
    cargo run -- setup

run: check-deps
    cargo run -- open

join code: check-deps
    cargo run -- join {{code}}

check: check-deps
    cargo check