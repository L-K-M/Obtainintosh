#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="$ROOT/scripts/check-release-version.sh"
VERSION="$(node -e "process.stdout.write(require('$ROOT/package.json').version)")"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

"$CHECK" "v$VERSION" "$ROOT" >/dev/null

for target in package lock cargo tauri; do
  fixture="$TMP/$target"
  mkdir -p "$fixture/src-tauri"
  cp "$ROOT/package.json" "$ROOT/package-lock.json" "$fixture/"
  cp "$ROOT/src-tauri/Cargo.toml" "$ROOT/src-tauri/tauri.conf.json" "$fixture/src-tauri/"

  node - "$fixture" "$target" <<'NODE'
const fs = require('fs');
const [root, target] = process.argv.slice(2);
const mismatch = '0.0.0-mismatch';

if (target === 'cargo') {
  const file = `${root}/src-tauri/Cargo.toml`;
  const data = fs.readFileSync(file, 'utf8');
  fs.writeFileSync(file, data.replace(/(\[package\][\s\S]*?\nversion = ")[^"]+/, `$1${mismatch}`));
} else {
  const file = target === 'lock'
    ? `${root}/package-lock.json`
    : target === 'tauri'
      ? `${root}/src-tauri/tauri.conf.json`
      : `${root}/package.json`;
  const data = JSON.parse(fs.readFileSync(file, 'utf8'));
  if (target === 'lock') data.packages[''].version = mismatch;
  else data.version = mismatch;
  fs.writeFileSync(file, JSON.stringify(data, null, 2));
}
NODE

  if output=$("$CHECK" "v$VERSION" "$fixture" 2>&1); then
    echo "error: $target mismatch unexpectedly passed" >&2
    exit 1
  fi
  [[ "$output" == *"found '0.0.0-mismatch'"* ]] || {
    echo "error: $target mismatch did not produce the expected diagnostic" >&2
    echo "$output" >&2
    exit 1
  }
done

echo "Release version preflight self-test passed."
