#!/usr/bin/env bash
set -euo pipefail

ROOT="${2:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

json_version() {
  node - "$1" "$2" <<'NODE'
const fs = require('fs');
const [file, kind] = process.argv.slice(2);
const data = JSON.parse(fs.readFileSync(file, 'utf8'));
const version = kind === 'lock' ? data.packages?.['']?.version : data.version;

if (typeof version !== 'string' || version.length === 0) {
  process.exit(1);
}
process.stdout.write(version);
NODE
}

package_version="$(json_version package.json package)" || {
  echo "error: could not read version from package.json" >&2
  exit 1
}
tag="${1:-v${package_version}}"
if [[ ! "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: release tag must match vX.Y.Z (got '$tag')" >&2
  exit 1
fi
expected="${tag#v}"

lock_version="$(json_version package-lock.json lock)" || lock_version="(unreadable)"
tauri_version="$(json_version src-tauri/tauri.conf.json tauri)" || tauri_version="(unreadable)"
cargo_version="$(awk -F'"' '
  /^\[package\][[:space:]]*$/ { in_package = 1; next }
  in_package && /^\[/ { in_package = 0 }
  in_package && /^[[:space:]]*version[[:space:]]*=/ { print $2; exit }
' src-tauri/Cargo.toml)"
cargo_version="${cargo_version:-(unreadable)}"

status=0
check_version() {
  local source="$1" actual="$2"
  if [[ "$actual" != "$expected" ]]; then
    echo "error: release tag $tag requires $source version '$expected', found '$actual'" >&2
    status=1
  fi
}

check_version "package.json" "$package_version"
check_version 'package-lock.json packages[""]' "$lock_version"
check_version "src-tauri/Cargo.toml [package]" "$cargo_version"
check_version "src-tauri/tauri.conf.json" "$tauri_version"

if [[ "$status" -ne 0 ]]; then
  exit "$status"
fi
echo "Release version preflight passed: $tag matches all shipped versions."
