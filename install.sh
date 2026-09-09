#!/bin/sh
# TEMM1E Installer — one-line install for macOS and Linux
#
# Usage:
#   curl -sSfL https://raw.githubusercontent.com/temm1e-labs/temm1e/main/install.sh | sh
#
# This script:
#   1. Detects your OS and architecture
#   2. Downloads the latest pre-built binary from GitHub Releases
#   3. Installs it to ~/.local/bin/temm1e (or /usr/local/bin with --global)
#   4. Verifies the checksum
#
# No Rust toolchain required.

set -e

REPO="temm1e-labs/temm1e"
# Default to ~/bin if it exists and is in PATH, otherwise ~/.local/bin
if [ -d "${HOME}/bin" ] && echo "$PATH" | grep -q "${HOME}/bin"; then
    INSTALL_DIR="${HOME}/bin"
else
    INSTALL_DIR="${HOME}/.local/bin"
fi
BINARY_NAME="temm1e"
GLOBAL=false

# Parse args
for arg in "$@"; do
    case "$arg" in
        --global) GLOBAL=true; INSTALL_DIR="/usr/local/bin" ;;
        --help|-h)
            echo "Usage: install.sh [--global]"
            echo ""
            echo "  --global    Install to /usr/local/bin (requires sudo)"
            echo "  (default)   Install to ~/.local/bin"
            exit 0
            ;;
    esac
done

# Colors (only if terminal supports them)
# No colors — keeps output clean when piped through sh
info()  { printf "> %s\n" "$1"; }
warn()  { printf "! %s\n" "$1"; }
error() { printf "x %s\n" "$1"; exit 1; }

# Detect OS
OS="$(uname -s)"
case "$OS" in
    Linux*)  PLATFORM="linux" ;;
    Darwin*) PLATFORM="macos" ;;
    *)       error "Unsupported OS: $OS. TEMM1E supports Linux and macOS." ;;
esac

# Detect architecture
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64|amd64)  ARCH_TAG="x86_64" ;;
    aarch64|arm64) ARCH_TAG="aarch64" ;;
    *)             error "Unsupported architecture: $ARCH. TEMM1E supports x86_64 and aarch64." ;;
esac

# On Linux, prefer the desktop binary (includes computer use / desktop control).
# Fall back to the server binary if desktop variant isn't available.
if [ "$PLATFORM" = "linux" ]; then
    ARTIFACT="${BINARY_NAME}-${ARCH_TAG}-${PLATFORM}-desktop"
    FALLBACK_ARTIFACT="${BINARY_NAME}-${ARCH_TAG}-${PLATFORM}"
else
    ARTIFACT="${BINARY_NAME}-${ARCH_TAG}-${PLATFORM}"
    FALLBACK_ARTIFACT=""
fi

info "Detected: ${PLATFORM} ${ARCH_TAG}"

# Get latest release tag
info "Finding latest release..."
LATEST_TAG=$(curl -sSfL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null \
    | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')

if [ -z "$LATEST_TAG" ]; then
    error "Could not find latest release. Check https://github.com/${REPO}/releases"
fi

info "Latest version: ${LATEST_TAG}"

# Download binary
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${LATEST_TAG}/${ARTIFACT}"
CHECKSUM_URL="https://github.com/${REPO}/releases/download/${LATEST_TAG}/checksums-sha256.txt"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

info "Downloading ${ARTIFACT}..."
if ! curl -sSfL -o "${TMPDIR}/${ARTIFACT}" "$DOWNLOAD_URL"; then
    if [ -n "$FALLBACK_ARTIFACT" ]; then
        info "Desktop binary not available, trying server binary..."
        ARTIFACT="$FALLBACK_ARTIFACT"
        DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${LATEST_TAG}/${ARTIFACT}"
        if ! curl -sSfL -o "${TMPDIR}/${ARTIFACT}" "$DOWNLOAD_URL"; then
            error "Download failed. Binary may not exist for your platform (${ARTIFACT})."
        fi
        info "Note: Server binary installed (no desktop control). For desktop control, build from source:"
        info "  cargo install --git https://github.com/${REPO}"
    else
        error "Download failed. Binary may not exist for your platform (${ARTIFACT})."
    fi
fi

# BEGIN CHECKSUM VERIFIER (also exercised by scripts/test_installer_checksum.py)
verify_checksum() {
    VERIFY_FILE="$1"
    VERIFY_MANIFEST="$2"
    VERIFY_NAME="$3"
    if command -v sha256sum >/dev/null 2>&1; then
        VERIFY_ACTUAL=$(sha256sum "$VERIFY_FILE" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
        VERIFY_ACTUAL=$(shasum -a 256 "$VERIFY_FILE" | awk '{print $1}')
    else
        error "Install sha256sum or shasum before installing Temm1e; checksum verification is required"
    fi
    # Match the complete filename, not a substring shared by fallback artifacts.
    VERIFY_EXPECTED=$(awk -v name="$VERIFY_NAME" '$2 == name || $2 == "*" name { print $1 }' "$VERIFY_MANIFEST")
    case "$VERIFY_EXPECTED" in
        *[!0-9a-fA-F]*|'') error "Missing, malformed or duplicate checksum for ${VERIFY_NAME}" ;;
    esac
    [ "${#VERIFY_EXPECTED}" -eq 64 ] || error "Invalid SHA-256 checksum for ${VERIFY_NAME}"
    VERIFY_EXPECTED=$(printf '%s' "$VERIFY_EXPECTED" | tr 'A-F' 'a-f')
    [ "$VERIFY_EXPECTED" = "$VERIFY_ACTUAL" ] || error "Checksum mismatch for ${VERIFY_NAME}"
    info "Checksum verified: ${VERIFY_NAME}"
}
# END CHECKSUM VERIFIER

info "Verifying checksum..."
curl -sSfL -o "${TMPDIR}/checksums.txt" "$CHECKSUM_URL" || error "Cannot download checksums; installation stopped"
verify_checksum "${TMPDIR}/${ARTIFACT}" "${TMPDIR}/checksums.txt" "$ARTIFACT"

# ────────────────────────────────────────────────────────────────────────
# Linux desktop binary — runtime library check
# ────────────────────────────────────────────────────────────────────────
# The Linux desktop binary is dynamically linked against Wayland, X11,
# PipeWire, XCB and friends via enigo + xcap. On minimal Ubuntu, WSL,
# Docker, or server installs these runtime libs are often missing.
# Detect that before copying the binary into place so the user is never
# left with a broken executable — either help them install the deps or
# fall back to the static musl server binary.
# ────────────────────────────────────────────────────────────────────────
if [ "$PLATFORM" = "linux" ]; then
    case "$ARTIFACT" in
        *-desktop)
            if command -v ldd >/dev/null 2>&1; then
                MISSING_LIBS=$(ldd "${TMPDIR}/${ARTIFACT}" 2>/dev/null \
                    | awk '/not found/ {print $1}' \
                    | sort -u)
            else
                MISSING_LIBS=""
            fi

            if [ -n "$MISSING_LIBS" ]; then
                echo ""
                warn "Desktop binary needs system libraries that are not installed:"
                for lib in $MISSING_LIBS; do
                    printf "    - %s\n" "$lib"
                done
                echo ""

                DEPS_CMD=""
                if command -v apt-get >/dev/null 2>&1; then
                    DEPS_CMD="sudo apt-get update && sudo apt-get install -y libwayland-client0 libwayland-cursor0 libwayland-egl1 libxcb1 libxcb-randr0 libxcb-shm0 libxkbcommon0 libpipewire-0.3-0 libspa-0.2-modules libegl1 libgbm1 libdrm2 libxdo3 libdbus-1-3 libxrandr2"
                elif command -v dnf >/dev/null 2>&1; then
                    DEPS_CMD="sudo dnf install -y wayland-libs-client libxkbcommon libxcb pipewire-libs mesa-libEGL mesa-libgbm libdrm dbus-libs libXrandr libxdo"
                elif command -v pacman >/dev/null 2>&1; then
                    DEPS_CMD="sudo pacman -S --needed wayland libxkbcommon libxcb pipewire mesa libdrm dbus libxrandr xdotool"
                fi

                RESOLVED=false
                if [ -n "$DEPS_CMD" ]; then
                    echo "  Install command:"
                    echo ""
                    printf "    %s\n" "$DEPS_CMD"
                    echo ""
                    if [ -r /dev/tty ]; then
                        printf "  Install them now via sudo? [Y/n] "
                        read -r ANSWER </dev/tty || ANSWER=""
                        case "${ANSWER:-y}" in
                            y|Y|yes|YES|Yes|"")
                                info "Installing system libraries..."
                                if sh -c "$DEPS_CMD"; then
                                    REMAINING=$(ldd "${TMPDIR}/${ARTIFACT}" 2>/dev/null \
                                        | awk '/not found/ {print $1}')
                                    if [ -z "$REMAINING" ]; then
                                        info "All system libraries resolved"
                                        RESOLVED=true
                                    else
                                        warn "Some libraries still missing after install:"
                                        for lib in $REMAINING; do
                                            printf "    - %s\n" "$lib"
                                        done
                                    fi
                                else
                                    warn "Dependency install failed"
                                fi
                                ;;
                        esac
                    else
                        warn "Running non-interactively — cannot prompt for sudo."
                        warn "Run the command above manually, then re-run the installer."
                    fi
                else
                    warn "No supported package manager (apt/dnf/pacman) detected."
                fi

                if [ "$RESOLVED" != "true" ]; then
                    echo ""
                    info "Falling back to the server (musl) binary — desktop control disabled."
                    info "Install the libraries above and re-run this script to upgrade."
                    FALLBACK_NAME="${BINARY_NAME}-${ARCH_TAG}-${PLATFORM}"
                    FALLBACK_URL="https://github.com/${REPO}/releases/download/${LATEST_TAG}/${FALLBACK_NAME}"
                    if ! curl -sSfL -o "${TMPDIR}/${FALLBACK_NAME}" "$FALLBACK_URL"; then
                        error "Failed to download fallback binary from ${FALLBACK_URL}"
                    fi
                    verify_checksum "${TMPDIR}/${FALLBACK_NAME}" "${TMPDIR}/checksums.txt" "$FALLBACK_NAME"
                    ARTIFACT="$FALLBACK_NAME"
                fi
            fi
            ;;
    esac
fi

# Install
mkdir -p "$INSTALL_DIR"
if [ "$GLOBAL" = true ]; then
    info "Installing to ${INSTALL_DIR} (may require sudo)..."
    sudo cp "${TMPDIR}/${ARTIFACT}" "${INSTALL_DIR}/${BINARY_NAME}"
    sudo chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
else
    cp "${TMPDIR}/${ARTIFACT}" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"
fi

# Verify installation
if "${INSTALL_DIR}/${BINARY_NAME}" --version >/dev/null 2>&1; then
    VERSION=$("${INSTALL_DIR}/${BINARY_NAME}" --version 2>&1 || echo "unknown")
    info "Installed: ${VERSION}"
else
    warn "Installed to ${INSTALL_DIR}/${BINARY_NAME} but --version failed."
    warn "Run it manually to see the error:  ${INSTALL_DIR}/${BINARY_NAME} --version"
fi

# Check PATH
case ":$PATH:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        warn "${INSTALL_DIR} is not in your PATH."
        echo ""
        echo "  Add this to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
        echo ""
        echo "    export PATH=\"${INSTALL_DIR}:\$PATH\""
        echo ""
        ;;
esac

echo ""
printf "TEMM1E installed!\n"
echo ""
echo "  Quick start:"
echo "    temm1e setup          # Interactive setup wizard"
echo "    temm1e auth login     # Authenticate with ChatGPT (optional)"
echo "    temm1e start          # Start the bot"
echo ""
echo "  Full guide: https://github.com/${REPO}#quick-start"
echo ""
