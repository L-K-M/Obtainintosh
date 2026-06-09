# Obtainintosh Code Review — Bugs, Issues, Missing Features & Ideas

A thorough review of the current codebase. Items are grouped by severity/kind,
and each has a confidence note about whether it should be fixed/implemented.

Legend: ✅ = implemented in a follow-up PR · 💡 = idea, not (yet) implemented

---

## 🐛 Bugs

### B1. Failed downloads write the error page to disk ✅
`download_file()` in `src-tauri/src/commands.rs` never checks the HTTP status of
the response. If GitHub returns a 404/403/5xx, the *error body* is written to
`Foo.dmg` and then revealed in Finder as if it were a successful download.
**Fix:** bail on non-success status before writing anything.

### B2. Whole download is buffered in memory ✅
`response.bytes().await` slurps the entire asset into RAM before writing. DMGs
are routinely 100–500 MB. The `reqwest` `stream` feature is already enabled but
unused. **Fix:** stream chunks to disk.

### B3. Download requests send no User-Agent ✅
The release-info request sets `User-Agent: Obtainintosh/0.1.0`, but
`reqwest::get(url)` for the actual download sends none. GitHub's API and some
CDNs reject UA-less requests. **Fix:** use a shared client with a UA.

### B4. Asset filename is used unsanitized as a path ✅
`cache_dir.join(filename)` uses the asset name straight from the GitHub API. A
name containing path separators (`../`) would escape the cache directory.
GitHub mostly sanitizes asset names, but defense-in-depth is cheap.
**Fix:** strip everything up to the last path separator.

### B5. `parse_github_url` mis-parses anything but a bare repo URL ✅
It takes the last two `/`-separated segments, so:
- `https://github.com/owner/repo/releases` → owner `repo`, repo `releases`
- `https://github.com/owner/repo.git` → repo `repo.git`
- `https://github.com/owner` → owner `github.com`, repo `owner` (!)

**Fix:** parse the path properly relative to the host, strip `.git`, validate.

### B6. `add_app` finds the new app by URL, not by ID ✅
After inserting, it reloads all apps and `find()`s by `source_url`. If two
entries share a URL it returns the wrong one; it's also two redundant storage
round-trips. Related: nothing prevents adding the same repo twice.
**Fix:** have `Storage::add_app` return the app with its generated ID, and
reject duplicate source URLs with a friendly error.

### B7. Stale "installed" state never clears ✅
`check_for_updates` re-detects the installed app but only *sets*
`current_version`/`install_path` when found. If the user deletes the app from
`/Applications`, Obtainintosh keeps claiming it's installed (and "Up to Date")
forever. **Fix:** clear both fields when detection fails.

### B8. Repos with only pre-releases always fail ✅
`GET /releases/latest` returns 404 when a repo has only pre-releases (a very
common pattern for nightly/continuous-build apps — exactly the audience of this
tool). **Fix:** on 404, fall back to `GET /releases` and take the newest
non-draft release.

### B9. Extension matching has no dot ✅
`find_macos_asset` uses `name.ends_with("dmg")`, so an asset named
`checksums-dmg` or `something.notdmg` would match the highest-priority bucket.
**Fix:** match `.{ext}` with the dot.

### B10. `if (success || true)` in remove confirmation ✅
`+page.svelte`'s `confirmRemove()` contains `if (success || true)` — the
condition is always true, so the variable is decorative. Harmless today but
clearly not what was meant. **Fix:** just close the dialog unconditionally (the
store already surfaces errors via notifications) and drop the dead condition.

### B11. Auto-detect of the program name fires on the first keystroke ✅
`AddAppDialog` fills `name` from the URL whenever `name` is empty. Since it
runs on every `input` event, *typing* a URL sets the name to `"h"` (the first
character) and then never updates it again, because `name` is no longer empty.
Pasting works by accident; typing is broken.
**Fix:** keep auto-filling while the name is still auto-derived (until the user
edits it manually) and only derive from URLs that actually look like
`github.com/owner/repo`.

### B12. Version "is newer" fallback uses lexicographic compare ✅
`hasUpdate()` falls back to `latest > current` — a plain string comparison
where `"9.0" > "10.0"`. The file already builds a numeric `Intl.Collator` for
sorting; use it here too.

### B13. Newly added apps show "Not Found" before any check ran ✅
The status column shows "No Mac version for this application was found" when
`latest_version` is null — including for apps that simply haven't been checked
yet (`last_checked` is null). **Fix:** show a neutral "Not Checked" state.

### B14. Broken CSS in AddAppDialog ✅
The dialog's `<style>` applies `display: flex; gap: 12px; justify-content:
flex-end;` to `input` (which makes no sense) — those rules clearly belong on
`.actions`, which currently has no styles, so Cancel/Add aren't right-aligned
like in the other dialogs.

### B15. Stock `<title>` in app.html ✅
The window's HTML title is still "Tauri + SvelteKit + Typescript App".

---

## ⚠️ General issues

### G1. GitLab is half-promised
`SourceType::GitLab` exists, `detect_source_type` accepts any URL containing
"gitlab", the Add dialog says "GitHub or GitLab URL" — but every check then
fails with "GitLab support not yet implemented", forever, on every Check All.
Either implement the GitLab adapter or stop accepting the URLs. (The settings
UI already comments out the GitLab token field, which is the right instinct.)

### G2. Tokens stored in plaintext JSON
The GitHub token lands in `~/Library/Application Support/Obtainintosh/apps.json`
in plaintext. The macOS Keychain (e.g. via the `keyring` crate) would be the
proper home. At minimum, the file should be `chmod 600`.

### G3. Over-broad capabilities
`capabilities/default.json` grants `shell:allow-execute` and
`shell:allow-spawn` to the webview, but no frontend code uses the shell plugin
(the `open -R` call happens in Rust). Drop both permissions and the
`tauri-plugin-shell` dependency. Also `"csp": null` disables the content
security policy entirely.

### G4. Unused dependencies
`zip = "0.6"` (an old version with known CVEs) and `semver` are declared in
`Cargo.toml` but never used in Rust code. Dead weight on every build.

### G5. Mutex poisoning panics
All `Storage` methods use `.lock().unwrap()`; one panic while holding the lock
poisons it and every subsequent storage call panics too. Low practical risk,
but `parking_lot` or `unwrap_or_else(|e| e.into_inner())` would be sturdier.

### G6. Sequential update checks
`check_for_updates` hits the network one app at a time. With 30 tracked apps on
a slow connection that's a long "Checking…". The checks are independent —
run them concurrently (`futures::future::join_all` or a small buffer).

### G7. One global `loading` flag
Any operation flips a single `loading` boolean that disables the whole toolbar
and dims the table. Per-row busy state (e.g. while downloading one app) would
feel much better.

### G8. `onFocusChanged` listener is never cleaned up
`+page.svelte` cleans up the four menu listeners but not the focus listener.
Harmless in a single-page app, but inconsistent.

### G9. Release notes are fetched, then dropped
The backend parses `release_notes` from GitHub and the model carries it across
the IPC boundary… where nothing displays it. See idea I3.

### G10. README install steps don't clone
The README says `cd obtainintosh` without a preceding `git clone` line.

---

## 🚧 Missing features

### M1. Check for updates on launch ✅
An update manager that only checks when you press a button is a doorbell that
only rings when you're looking at it. Auto-check on startup (it already shows
per-app errors gracefully).

### M2. Download progress ✅
Downloads are silent until they finish. The backend should emit progress events
(`downloaded`/`total` bytes) and the frontend should show a progress bar —
see idea I1 for the fun version.

### M3. Periodic background checks
Beyond M1: a configurable interval ("check every 6 hours") with a macOS
notification when updates are found. That's the core Obtainium feature.

### M4. Per-app check
`check_for_updates` already accepts an optional `app_id`, but no UI calls it.
A per-row "check now" action would be cheap to add.

### M5. Update all
When five apps have updates, there's no "download them all" button.

### M6. Import/export of the app list
A JSON export/import (or even a shareable `obtainintosh://add?repo=owner/repo`
URL) makes the tool portable across machines.

### M7. Architecture-aware asset choice
`find_macos_asset` has a `todo` for this: on Apple Silicon prefer
`universal` > `arm64` > `x86_64`, on Intel the reverse. The machine's arch is
known at compile time / via `uname`.

### M8. Actually installing
The current flow downloads and reveals in Finder. A natural next step: mount
DMGs (`hdiutil attach`), copy the `.app` to `/Applications`, unzip zips —
with the user's consent per app.

---

## ✨ Novel / delightful / quirky ideas

### I1. System 7 "Finder copy" progress dialog ✅ (UI side)
When downloading, show a faithful System 7 file-copy dialog: the striped
progress bar, "Items remaining: 1", and the file name in Chicago. It's the
single most evocative System 7 interaction and fits the app's whole bit.

### I2. Startup chime & sad Mac
Play a tiny startup chime on launch; when an update check fails hard, show a
"sad Mac" (or the bomb dialog: *"Sorry, a system error occurred."* with a
useless Restart button that just retries the check — which is funnier because
that's exactly what the real one did).

### I3. Release notes in a SimpleText window
Double-click the "Latest" version cell → a SimpleText-style window with the
release notes (Geneva 9, the little page-with-pencil icon). The data is already
on the model (G9), it just needs a window.

### I4. "About This Macintosh" memory bars
Make the About dialog mimic "About This Macintosh": each tracked app drawn as a
RAM bar whose fill level = how outdated it is. "System Software 7.5.5" replaced
by "Obtainintosh 0.1.0".

### I5. Menu-bar Dogcow
A `🐄` Easter egg: clicking the version number in About cycles through Clarus
the Dogcow saying "Moof!".

### I6. Update count in the Dock badge
Tauri can set the macOS badge — show the number of available updates, the way
the App Store does.

### I7. Drag-and-drop a GitHub URL onto the window
Drag a repo URL from the browser straight onto the table to add it,
with the classic marching-ants drop highlight.

### I8. Puzzle desk accessory
Apple menu → "Puzzle": the 15-puzzle from System 7, because no reason.

---

## Implementation status

Backend fixes (B1–B9, M2 events) and frontend fixes (B10–B15, M1, I1) are
implemented in follow-up PRs, split so they touch disjoint files
(`src-tauri/**` vs `src/**`) and don't conflict with each other or with this
document.
