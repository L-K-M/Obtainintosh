#!/usr/bin/env bash
# Build Obtainintosh. Targets:
#   linux    .deb + .AppImage via Tauri (tauri.linux.conf.json) -> src-tauri/target/release/bundle/
#   flatpak  .flatpak via scripts/build-flatpak.sh              -> dist/
#   macos    .app + .dmg via Tauri                              -> src-tauri/target/<triple>/release/bundle/
# With no target it builds everything this machine can build; naming a target
# explicitly means it must build (a missing tool is an error, not a skip).
#
#   scripts/build.sh            # build what this machine can build
#   scripts/build.sh linux      # .deb + .AppImage only
#   scripts/build.sh flatpak    # .flatpak only
#   scripts/build.sh macos      # .app + .dmg only
#   scripts/build.sh --check    # print the build plan, build nothing
#   scripts/build.sh --clean    # rm -rf dist/ and the Tauri bundle dir first
#   scripts/build.sh --install  # build and install Obtainintosh.app to /Applications
#   scripts/build.sh --run      # launch the app after building
#
# Needs: Node 20+ and npm ci (frontend), a Rust toolchain (everywhere),
# webkit2gtk + friends on Linux (see .github/workflows/release.yml), Xcode CLT
# on macOS. --install and --run are about the macOS app: with no explicit
# targets they build just that, and on a non-Mac they fail.
set -euo pipefail
cd "$(dirname "$0")/.."

CLEAN=0 CHECK=0 INSTALL=0 RUN=0
TARGETS=() EXPLICIT=""
for arg in "$@"; do
  case "$arg" in
    --clean) CLEAN=1 ;;
    --check) CHECK=1 ;;
    --install) INSTALL=1 ;;
    --run) RUN=1 ;;
    -h|--help) awk 'NR==1 && /^#!/ {next} /^set -euo pipefail/{next} /^#/ {sub(/^# ?/,""); print; next} {exit}' "$0"; exit 0 ;;
    linux|flatpak|macos) TARGETS+=("$arg"); EXPLICIT=1 ;;
    *) echo "!! unknown argument: $arg (see --help)" >&2; exit 1 ;;
  esac
done

# --install/--run are about the macOS app: imply the target, and fail rather
# than silently skip on a host that cannot produce it.
if [[ $INSTALL -eq 1 || $RUN -eq 1 ]]; then
  if [[ $EXPLICIT -ne 1 ]]; then
    TARGETS=(macos); EXPLICIT=1
  else
    found=0
    for t in "${TARGETS[@]}"; do [[ "$t" == macos ]] && found=1; done
    [[ $found -eq 0 ]] && TARGETS+=(macos)
  fi
fi

[[ ${#TARGETS[@]} -eq 0 ]] && TARGETS=(linux flatpak macos)

VERSION="$(sed -n 's/^  "version": "\([^"]*\)".*/\1/p' src-tauri/tauri.conf.json)"

declare -a OK=() SKIPPED=() FAILED=()

skip_or_fail() { # target reason
  if [[ $EXPLICIT -eq 1 ]]; then
    echo "!! $1: $2"; FAILED+=("$1")
  else
    echo ".. skipping $1: $2"; SKIPPED+=("$1")
  fi
}

have() { command -v "$1" >/dev/null 2>&1; }

frontend() { # node_modules + vite build once, then no-op
  [[ -n ${FRONTEND:-} ]] && return
  if ! have node || ! have npm; then
    echo "!! frontend: node/npm not found"; return 1
  fi
  if [[ ! -d node_modules ]]; then
    npm ci --no-audit --no-fund || return 1
  fi
  npm run --silent build || return 1
  FRONTEND=ok
}

if [[ $CHECK -eq 1 ]]; then
  echo "==> plan"
  echo "-- targets:  ${TARGETS[*]}${EXPLICIT:+ (explicit)}"
  echo "-- version:  $VERSION"
  for t in node npm cargo rustc flatpak-builder; do
    printf -- "-- %-18s %s\n" "$t:" "$(command -v "$t" 2>/dev/null || echo missing)"
  done
  exit 0
fi

if [[ $CLEAN -eq 1 ]]; then
  echo "==> cleaning"
  rm -rf dist src-tauri/target/release/bundle src-tauri/target/*/release/bundle
fi

# The bundled .app path differs by --target: with a cross triple it lands under
# target/<triple>/release/bundle/, a native build under target/release/bundle/.
macos_app() {
  # Stale triple-specific bundles can sit beside the fresh native one;
  # newest mtime wins.
  ls -td src-tauri/target/release/bundle/macos/Obtainintosh.app \
         src-tauri/target/*/release/bundle/macos/Obtainintosh.app 2>/dev/null | head -1
}

for target in "${TARGETS[@]}"; do
  case "$target" in
    linux)
      echo "==> linux .deb + .AppImage"
      if [[ "$(uname -s)" != "Linux" ]]; then
        skip_or_fail linux "Linux bundles build on Linux"; continue
      fi
      if ! have cargo; then
        skip_or_fail linux "Rust toolchain not installed"; continue
      fi
      frontend || { FAILED+=("linux"); continue; }
      if npm run --silent tauri build; then
        OK+=("linux → src-tauri/target/release/bundle/")
      else
        FAILED+=("linux")
      fi
      ;;
    flatpak)
      echo "==> flatpak"
      if [[ "$(uname -s)" != "Linux" ]]; then
        skip_or_fail flatpak "Flatpak builds run on Linux"; continue
      fi
      if ! have flatpak || ! have flatpak-builder; then
        skip_or_fail flatpak "flatpak or flatpak-builder not installed"; continue
      fi
      if scripts/build-flatpak.sh; then
        OK+=("flatpak → dist/")
      else
        FAILED+=("flatpak")
      fi
      ;;
    macos)
      echo "==> macOS app"
      if [[ "$(uname -s)" != "Darwin" ]]; then
        skip_or_fail macos "the macOS app builds on macOS"; continue
      fi
      if ! have cargo; then
        skip_or_fail macos "Rust toolchain not installed"; continue
      fi
      frontend || { FAILED+=("macos"); continue; }
      if npm run --silent tauri build; then
        APP="$(macos_app)"
        if [[ -z "$APP" ]]; then
          echo "!! macos: no Obtainintosh.app under src-tauri/target/*/release/bundle/" >&2
          FAILED+=("macos"); continue
        fi
        OK+=("macos → $APP")
        if [[ $INSTALL -eq 1 ]]; then
          exe="$(basename "$APP" .app)"
          # macOS truncates process names to 15 chars (MAXCOMLEN).
          if pgrep -x "${exe:0:15}" >/dev/null; then
            echo "-- quitting running $exe"
            # The process can exit between pgrep and pkill; don't let the
            # race abort the install under set -e.
            pkill -x "${exe:0:15}" || true; sleep 1
          fi
          echo "-- installing /Applications/$exe.app"
          if ! rm -rf "/Applications/$exe.app"; then
            echo "!! cannot replace /Applications/$exe.app (permissions?)" >&2
            FAILED+=("macos (install)")
          # ditto preserves the signature, resource forks and permissions.
          elif ditto "$APP" "/Applications/$exe.app"; then
            OK+=("installed → /Applications/$exe.app")
            if [[ $RUN -eq 1 ]]; then
              open "/Applications/$exe.app"
            else
              open -R "/Applications/$exe.app"
            fi
          else
            FAILED+=("macos (install)")
          fi
        elif [[ $RUN -eq 1 ]]; then
          open "$APP"
        fi
      else
        FAILED+=("macos")
      fi
      ;;
    *)
      echo "!! unknown target: $target"
      FAILED+=("$target")
      ;;
  esac
done

echo
echo "Summary"
for x in ${OK[@]+"${OK[@]}"}; do echo "   ok      $x"; done
for x in ${SKIPPED[@]+"${SKIPPED[@]}"}; do echo "   skipped $x"; done
for x in ${FAILED[@]+"${FAILED[@]}"}; do echo "   FAILED  $x"; done
[[ ${#FAILED[@]} -eq 0 ]]
