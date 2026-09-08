#!/usr/bin/env bash
# Release update-path smoke test.
#
# Usage:
#   scripts/release_update_smoke.sh <prev_tag> <new_tag>
#
# Downloads the previous release's binary for the current platform,
# executes its `temm1e update` subcommand, and asserts the resulting
# binary reports the new version. This is the empirical gate that
# checks `temm1e update` for the selected prior release and host platform
# before the release is declared done.
#
# Added in v5.5.2 after the v5.5.1 `update` failed on macOS ARM and
# Ubuntu due to an asset-naming drift that had existed undetected for
# several releases. See docs/RELEASE_PROTOCOL.md §10.5.

set -euo pipefail

PREV_TAG="${1:?usage: $0 <prev_tag> <new_tag>   e.g. v5.5.1 v5.5.2}"
NEW_TAG="${2:?usage: $0 <prev_tag> <new_tag>}"
REPO="temm1e-labs/temm1e"

# Detect platform using install.sh's convention
OS="$(uname -s)"
case "$OS" in
    Linux*)  PLATFORM="linux" ;;
    Darwin*) PLATFORM="macos" ;;
    *) echo "Unsupported OS: $OS — update smoke only runs on Linux/macOS" >&2; exit 2 ;;
esac
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64|amd64) ARCH_TAG="x86_64" ;;
    aarch64|arm64) ARCH_TAG="aarch64" ;;
    *) echo "Unsupported arch: $ARCH" >&2; exit 2 ;;
esac

# Pick the preferred asset per install.sh's convention: desktop on Linux,
# single build on macOS. If desktop isn't available in the old release,
# fall back to server.
if [ "$PLATFORM" = "linux" ]; then
    CANDIDATES="temm1e-${ARCH_TAG}-linux-desktop temm1e-${ARCH_TAG}-linux"
else
    CANDIDATES="temm1e-${ARCH_TAG}-${PLATFORM}"
fi

UPDATE_SMOKE_DIR="$(mktemp -d)"
trap 'rm -rf "$UPDATE_SMOKE_DIR"' EXIT

# Download the previous-tag binary
DOWNLOADED=""
for asset in $CANDIDATES; do
    url="https://github.com/${REPO}/releases/download/${PREV_TAG}/${asset}"
    echo "→ trying $url"
    if curl --connect-timeout 15 --max-time 180 --max-filesize 134217728 -sSfL -o "$UPDATE_SMOKE_DIR/$asset" "$url"; then
        DOWNLOADED="$asset"
        break
    fi
done

if [ -z "$DOWNLOADED" ]; then
    echo "FAIL: no asset from [$CANDIDATES] was downloadable at $PREV_TAG" >&2
    exit 1
fi

# Never execute an unverified previous-release download. The release workflow
# publishes a single manifest containing exact asset names.
curl --connect-timeout 15 --max-time 60 --max-filesize 65536 -sSfL \
    "https://github.com/${REPO}/releases/download/${PREV_TAG}/checksums-sha256.txt" \
    -o "$UPDATE_SMOKE_DIR/checksums-sha256.txt"
python3 - "$UPDATE_SMOKE_DIR" "$DOWNLOADED" <<'VERIFY'
import hashlib
from pathlib import Path
import sys
root, asset = Path(sys.argv[1]), sys.argv[2]
entries = []
for line in (root / 'checksums-sha256.txt').read_text().splitlines():
    parts = line.split()
    if len(parts) == 2 and parts[1].removeprefix('*') == asset:
        entries.append(parts[0].lower())
hash_value = hashlib.sha256()
with (root / asset).open('rb') as binary:
    for chunk in iter(lambda: binary.read(1024 * 1024), b''):
        hash_value.update(chunk)
if entries != [hash_value.hexdigest()]:
    raise SystemExit('FAIL: prior release checksum missing, duplicate or mismatched')
VERIFY

chmod +x "$UPDATE_SMOKE_DIR/$DOWNLOADED"

# Sanity: previous binary reports the previous version
PREV_REPORTED="$("$UPDATE_SMOKE_DIR/$DOWNLOADED" --version 2>&1 || true)"
echo "Previous binary reports: $PREV_REPORTED"
case "$PREV_REPORTED" in
    *"${PREV_TAG#v}"*) ;;
    *) echo "FAIL: previous binary did not report $PREV_TAG — got: $PREV_REPORTED" >&2; exit 1 ;;
esac

# Run its update command. Must `cd` to a non-git-repo directory first
# because the updater auto-detects "in a git checkout" (via `git rev-parse`
# walking up from cwd) and switches to `git pull` mode — which would
# false-pass this smoke if the CI runner happens to be sitting in the
# repo checkout.
echo "→ running: $UPDATE_SMOKE_DIR/$DOWNLOADED update   (from $UPDATE_SMOKE_DIR to avoid git-mode)"
if ! (cd "$UPDATE_SMOKE_DIR" && printf 'y\n' | "./$DOWNLOADED" update); then
    echo "FAIL: \`temm1e update\` returned non-zero — the release is not updatable from $PREV_TAG on $PLATFORM/$ARCH_TAG" >&2
    exit 1
fi

# After update, the same path should now be the new version
NEW_REPORTED="$("$UPDATE_SMOKE_DIR/$DOWNLOADED" --version 2>&1 || true)"
echo "After update, binary reports: $NEW_REPORTED"
case "$NEW_REPORTED" in
    *"${NEW_TAG#v}"*)
        echo "PASS: $PREV_TAG → $NEW_TAG update-path works on $PLATFORM/$ARCH_TAG"
        ;;
    *)
        echo "FAIL: expected $NEW_TAG after update, got: $NEW_REPORTED" >&2
        exit 1
        ;;
esac
