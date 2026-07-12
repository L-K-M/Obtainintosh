#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="$ROOT/scripts/check-release-version.sh"
VERSION="$(node -e "process.stdout.write(require('$ROOT/package.json').version)")"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

make_fixture() {
  local fixture="$1"
  mkdir -p "$fixture/src-tauri"
  cp "$ROOT/package.json" "$ROOT/package-lock.json" "$fixture/"
  cp "$ROOT/src-tauri/Cargo.toml" "$ROOT/src-tauri/tauri.conf.json" "$fixture/src-tauri/"
}

set_fixture_version() {
  node - "$1" "$2" <<'NODE'
const fs = require('fs');
const [root, version] = process.argv.slice(2);

for (const relative of ['package.json', 'src-tauri/tauri.conf.json']) {
  const file = `${root}/${relative}`;
  const data = JSON.parse(fs.readFileSync(file, 'utf8'));
  data.version = version;
  fs.writeFileSync(file, JSON.stringify(data, null, 2));
}

const lockFile = `${root}/package-lock.json`;
const lock = JSON.parse(fs.readFileSync(lockFile, 'utf8'));
if (!lock.packages?.['']) throw new Error('lockfile fixture has no root package');
lock.packages[''].version = version;
fs.writeFileSync(lockFile, JSON.stringify(lock, null, 2));

const cargoFile = `${root}/src-tauri/Cargo.toml`;
const cargo = fs.readFileSync(cargoFile, 'utf8');
fs.writeFileSync(cargoFile, cargo.replace(/(\[package\][\s\S]*?\nversion = ")[^"]+/, `$1${version}`));
NODE
}

"$CHECK" "v$VERSION" "$ROOT" >/dev/null

semver_fixture="$TMP/semver"
make_fixture "$semver_fixture"
for version in 1.2.3-rc.1 1.2.3+build.7 1.2.3-rc.1+build.7; do
  set_fixture_version "$semver_fixture" "$version"
  "$CHECK" "v$version" "$semver_fixture" >/dev/null
done

for tag in v1.2.3- v1.2.3-rc..1 v1.2.3-01 v1.2.3+ v1.2.3-rc_1 v01.2.3; do
  if output=$("$CHECK" "$tag" "$ROOT" 2>&1); then
    echo "error: malformed tag '$tag' unexpectedly passed" >&2
    exit 1
  fi
  [[ "$output" == *"must be valid"* ]] || {
    echo "error: malformed tag '$tag' did not produce the expected diagnostic" >&2
    echo "$output" >&2
    exit 1
  }
done

for target in package lock cargo tauri; do
  fixture="$TMP/$target"
  make_fixture "$fixture"
  set_fixture_version "$fixture" "1.2.3-rc.1+build.7"

  node - "$fixture" "$target" <<'NODE'
const fs = require('fs');
const [root, target] = process.argv.slice(2);
const mismatch = '1.2.3-rc.2+build.7';

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

  if output=$("$CHECK" "v1.2.3-rc.1+build.7" "$fixture" 2>&1); then
    echo "error: $target mismatch unexpectedly passed" >&2
    exit 1
  fi
  [[ "$output" == *"found '1.2.3-rc.2+build.7'"* ]] || {
    echo "error: $target mismatch did not produce the expected diagnostic" >&2
    echo "$output" >&2
    exit 1
  }
done

echo "Release version preflight self-test passed."
