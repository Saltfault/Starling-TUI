#!/usr/bin/env bash
# Starling installer — Linux / macOS
# Usage:
#   curl -sSfL https://forgejo.hearthhome.lol/Saltfault/<REPO>/releases/download/v<VERSION>/install.sh | bash
#   ./install.sh -v v0.6.15 -b starling-tui -r Starling-TUI
#   ./install.sh --uninstall -b starling-tui

set -euo pipefail

BINARY="starling-tui"
REPO="Starling-TUI"
VERSION="latest"
UNINSTALL=false
UPGRADE=false
FORGEJO="https://forgejo.hearthhome.lol/Saltfault"
INSTALL_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        -b|--binary) BINARY="$2"; shift 2 ;;
        -r|--repo) REPO="$2"; shift 2 ;;
        -v|--version) VERSION="$2"; shift 2 ;;
        --uninstall) UNINSTALL=true; shift ;;
        --upgrade) UPGRADE=true; shift ;;
        *) echo "Unknown flag: $1"; exit 1 ;;
    esac
done

# ---- detect platform ----
OS=$(uname -s)
ARCH=$(uname -m)
case "$OS" in
    Linux)  OS="unknown-linux-gnu" ;;
    Darwin) OS="apple-darwin" ;;
    *)      echo "Unsupported OS: $OS"; exit 1 ;;
esac
case "$ARCH" in
    x86_64|amd64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac
TARGET="${ARCH}-${OS}"
EXT=""
[[ "$OS" == *windows* ]] && EXT=".exe"

# ---- uninstall ----
if $UNINSTALL; then
    rm -f "$INSTALL_DIR/$BINARY$EXT"
    echo "Uninstalled $BINARY"
    exit 0
fi

# ---- upgrade ----
if $UPGRADE; then
    echo "Upgrading $BINARY to $VERSION..."
fi

# ---- resolve version ----
if [[ "$VERSION" == "latest" ]]; then
    TAG=$(curl -sSf "$FORGEJO/api/v1/repos/Saltfault/$REPO/releases/latest" | grep -o '"tag_name":"[^"]*"' | cut -d'"' -f4)
else
    TAG="$VERSION"
fi

# ---- download ----
ASSET="${BINARY}-${TARGET}${EXT}"
URL="$FORGEJO/$REPO/releases/download/$TAG/$ASSET"
echo "Downloading $ASSET ($TAG)..."
mkdir -p "$INSTALL_DIR"
curl -sSfL "$URL" -o "$INSTALL_DIR/$BINARY$EXT"
chmod +x "$INSTALL_DIR/$BINARY$EXT"

# ---- checksum verification ----
SHA_URL="$FORGEJO/$REPO/releases/download/$TAG/$BINARY-${TARGET}.sha256"
if EXPECTED=$(curl -sSf "$SHA_URL" 2>/dev/null | cut -d' ' -f1); then
    ACTUAL=$(sha256sum "$INSTALL_DIR/$BINARY$EXT" | cut -d' ' -f1)
    if [[ "$EXPECTED" != "$ACTUAL" ]]; then
        rm -f "$INSTALL_DIR/$BINARY$EXT"
        echo "Checksum mismatch! Expected $EXPECTED, got $ACTUAL"
        exit 1
    fi
    echo "Checksum verified"
else
    echo "Skipping checksum verification (not found)"
fi

# ---- verify on PATH ----
if ! command -v "$BINARY" &>/dev/null; then
    echo "NOTE: $INSTALL_DIR is not on your PATH."
    echo "  Add this to your shell profile:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
fi

echo "Installed $BINARY $TAG to $INSTALL_DIR/$BINARY$EXT"
echo "Run '$BINARY --version' to verify"
