#!/usr/bin/env bash
# GitHub-hosted Ubuntu images include an unrelated Google Chrome apt source.
# A stale Google Packages index must not block installing Ubuntu build libraries.
# Keep the installed browser and all package signature/hash checks intact.
set -euo pipefail
if [[ "${GITHUB_ACTIONS:-}" != "true" ]]; then
  echo 'This helper is only for disposable GitHub Actions runners.' >&2
  exit 2
fi
for source in /etc/apt/sources.list.d/*.list; do
  [[ -f "$source" ]] || continue
  sudo sed -E -i '/https?:\/\/dl[.]google[.]com\/linux\/chrome[^[:space:]]*\/deb/d' "$source"
done
sudo apt-get update
