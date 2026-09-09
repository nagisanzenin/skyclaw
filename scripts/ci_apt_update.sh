#!/usr/bin/env bash
# GitHub-hosted Ubuntu images include an unrelated Google Chrome apt source.
# A stale Google Packages index must not block installing Ubuntu build libraries.
# Keep the installed browser and all package signature/hash checks intact.
set -euo pipefail
if [[ "${GITHUB_ACTIONS:-}" != "true" ]]; then
  echo 'This helper is only for disposable GitHub Actions runners.' >&2
  exit 2
fi
sudo env GITHUB_ACTIONS=true python3 "$(dirname "$0")/ci_apt_sources.py"
sudo apt-get update
