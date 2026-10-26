#!/usr/bin/env bash
# Generate the license notices that ship inside the app bundles. Tauri runs
# this from build.beforeBuildCommand, after `npm run build` wrote the
# frontend to build/ and its npm notices to build/Third-Party-Notices.txt:
#
#   build/Third-Party-Notices.txt          npm packages, fonts and Rust crates
#                                          (embedded in the binary for the
#                                          About dialog, and copied into each
#                                          bundle)
#   src-tauri/licenses/Rust-Licenses.html  the Rust standard library
#
# tauri.conf.json places them in the bundles. The
# target comes from TAURI_ENV_TARGET_TRIPLE, which the Tauri CLI sets for its
# hooks; outside Tauri, pass it: scripts/notices.sh --target <triple>.
# Every generator fails when a bundled component has no license text.
set -euo pipefail
cd "$(dirname "$0")/.."

readonly OUT=src-tauri/licenses
mkdir -p "$OUT"

python3 scripts/rust-notices.py --manifest-path src-tauri/Cargo.toml "$@" \
  --title Obtainintosh --overrides licenses/rust-overrides.json --own-license MIT \
  --out "$OUT/Rust-Crates.txt" --rust-std "$OUT/Rust-Licenses.html"

# One notices file per artifact: the npm packages of the frontend build that
# just ran, followed by the Rust crates.
npm run --silent notices:web -- --include "$OUT/Rust-Crates.txt"
