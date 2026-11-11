#!/usr/bin/env bash
# Regenerates cargo-sources.json (offline crate list Flatpak builds need) from
# Cargo.lock. Rerun after every dependency change, before a Flathub update.
set -euo pipefail
cd "$(dirname "$0")"
GEN=https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
curl -fsSL "$GEN" -o "${TMPDIR:-/tmp}/flatpak-cargo-generator.py"
# Needs python3 with aiohttp and tomlkit (pip install --user aiohttp tomlkit).
python3 "${TMPDIR:-/tmp}/flatpak-cargo-generator.py" ../../Cargo.lock -o cargo-sources.json
echo "wrote installer/flatpak/cargo-sources.json"
