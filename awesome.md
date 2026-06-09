# Obtainintosh Code Review — Findings & Ideas

A thorough review of the codebase: bugs, general issues, missing features, and
some ideas for delightful improvements. Items marked **[implementing]** are
being addressed in follow-up PRs; the rest are documented for future work.

> Follow-up PRs: backend items are implemented in **#5**, frontend items
> (B7, B14, G9) in **#6**.

---

## Bugs

### B1. Downloads ignore HTTP status codes **[implementing]**
`download_file()` in `src-tauri/src/commands.rs` calls `reqwest::get(url)` and
writes whatever comes back straight to disk. A 404 or rate-limit response is an
`Ok` response in reqwest, so an HTML error page gets saved as `Foo.dmg`,
revealed in Finder, and presented to the user as a successful download. Needs
`error_for_status()`.

### B2. Entire download is buffered in memory **[implementing]**
`response.bytes().await` slurps the whole asset into RAM before writing. DMGs
are routinely hundreds of megabytes (a 1+ GB Xcode-adjacent tool would be
worse). Stream the body to disk chunk by chunk instead — this also unlocks
progress reporting (see F1).

### B3. Naive GitHub URL parsing picks the wrong owner/repo **[implementing]**
`parse_github_url()` takes the last two path segments of the URL:
- `https://github.com/owner/repo/releases` → owner=`repo`, repo=`releases`
- `https://github.com/owner` → owner=`github.com`, repo=`owner`
- Trailing `.git`, query strings, and fragments are not handled.

The fix is to parse the path *after* the `github.com` host and take the first
two segments, stripping `.git`.

### B4. `add_app` finds the new app by URL, not by ID **[implementing]**
After inserting, the command re-reads all apps and returns the first one whose
`source_url` matches. If two entries share a URL (which is currently allowed —
see B5), the wrong record (and wrong `id`) is returned to the frontend.
`Storage::add_app` should simply return the app with its generated UUID.

### B5. Duplicate apps can be added freely **[implementing]**
Nothing prevents adding the same repo twice. Combined with B4, the second add
returns the *first* app's ID, so editing/removing operates on the wrong row.

### B6. Editing an app's URL keeps stale version data **[implementing]**
`update_app` changes `name`/`source_url` but keeps `latest_version`,
`current_version` and `last_checked` from the *old* repo. Until the next
check, the row can claim "Update Available" by comparing versions from two
different projects.

### B7. Add dialog gets stuck disabled when adding fails
In `AddAppDialog.svelte`, the add path sets `loading = true` and then
fire-and-forgets `onadd(...)`. If the backend rejects the app (e.g.
"Unsupported source URL"), the dialog stays open with the Add button disabled
forever — `loading` is never reset because the dialog has no idea the call
failed. The callback should be awaited (or loading reset after dispatch).
**[implementing]**

### B8. Failed update checks render as "Not Found — no Mac version"
`check_for_updates` logs per-app errors and silently returns the stale app. A
rate-limited, offline, or 404 check shows the "Not Found" badge whose balloon
help says *"No Mac version for this application was found"* — a confidently
wrong diagnosis. The check also always toasts "Update check completed" as a
success. Distinguishing "no asset" from "check failed" needs an error field on
the app model; documented here as a follow-up.

### B9. Repos with only prereleases can never be checked **[implementing]**
The GitHub adapter uses `/releases/latest`, which excludes prereleases and
drafts and 404s for nightly-only projects. Fall back to listing `/releases`
and picking the newest non-draft release.

### B10. `~/Applications` is never searched **[implementing]**
`detect_installed_app` only scans `/Applications`. Apps installed per-user
(common for unsandboxed downloads, and what some apps self-install as) live in
`~/Applications` and are reported as "Not Installed".

### B11. No network timeouts **[implementing]**
Neither the API client nor the downloader sets a timeout. A hung connection
leaves the UI in "Checking..." with the toolbar buttons disabled indefinitely.

### B12. Version prefix stripping is too eager **[implementing]**
`tag_name.trim_start_matches('v')` strips *all* leading v's and mangles tags
that merely start with the letter: `vv1.0` → `1.0` (fine), but `version-2` →
`ersion-2`. Only strip a single leading `v` when it's followed by a digit.

### B13. Extension matching doesn't require a dot **[implementing]**
`find_macos_asset` uses `name.ends_with("dmg")`, so an asset named
`tool-amdmg` would match the "dmg" rule. Match `.dmg` (with the dot).

### B14. Dead `if (success || true)` in `+page.svelte` **[implementing]**
`confirmRemove()` contains `if (success || true)` — always true. The remove
dialog also closes when removal fails, which is arguably the desired behavior,
so the condition should just go away.

---

## General issues

### G1. Unused dependencies: `zip`, `semver`, `thiserror` **[implementing]**
None of these crates are referenced anywhere in the Rust code. They add build
time and audit surface for nothing.

### G2. Shell plugin granted but never used **[implementing]**
`tauri_plugin_shell` is initialized in `lib.rs` and `capabilities/default.json`
grants `shell:allow-execute` and `shell:allow-spawn` to the webview — but the
frontend doesn't even depend on `@tauri-apps/plugin-shell`. That's unnecessary
attack surface: any XSS in the webview could spawn arbitrary processes. Remove
the plugin, the dependency, and the permissions. (The Rust side uses
`std::process::Command` directly, which doesn't need any of this.)

### G3. No Content Security Policy
`tauri.conf.json` has `"csp": null`. Combined with G2 this weakens
defense-in-depth. A minimal CSP allowing `self` plus
`avatars.githubusercontent.com` for the icons would be a good baseline.

### G4. GitHub token stored in plaintext JSON
`apps.json` in Application Support holds the PAT in cleartext. Fine-grained
read-only tokens limit the blast radius, but the macOS keychain (e.g. the
`keyring` crate or `tauri-plugin-stronghold`) is the right home for it. The
Settings UI hint should also mention the file is not encrypted.

### G5. Hardcoded User-Agent version **[implementing]**
`"Obtainintosh/0.1.0"` will silently drift from the real version. Use
`env!("CARGO_PKG_VERSION")`. Also send `X-GitHub-Api-Version: 2022-11-28` and
`Accept: application/vnd.github+json` like the API docs ask.

### B/G6. Asset filename used unsanitized in path join **[implementing]**
`cache_dir.join(filename)` with the filename taken from the GitHub API
response. GitHub normalizes asset names today, but defensively rejecting path
separators costs three lines.

### G7. `check_for_updates` is sequential
Each app is checked one at a time. Ten tracked apps on a slow connection make
"Check All" feel glacial; `futures::join_all` (with a small concurrency cap)
would parallelize it. Not implementing now to keep error-handling changes
small, but it pairs well with B8.

### G8. `current_version` only refreshes on update checks
`get_all_apps` returns stored data, so if the user updates an app outside
Obtainintosh, the table shows the stale version until the next "Check All".
Re-detecting installed versions on `loadApps` (it's just a couple of
PlistBuddy calls) would keep the table honest.

### G9. `getIconUrl()` is called twice per row
Once in the `{#if}` and once for `src`. Trivial, but a `$:` reactive
declaration reads better and halves the work.

### G10. Stray test scaffolding in `sources.rs`
A large commented-out block of tests for a function
(`extract_version_from_text`) that doesn't exist — with a comment explaining
the tests would not compile. Delete it; git remembers.

---

## Missing features

### F1. Download progress
`file_size` is already fetched from the API and then ignored. Streaming
downloads (B2) plus a Tauri event per chunk would drive a classic Mac OS
progress bar — the System 7 copy-progress dialog is *right there* asking to be
recreated.

### F2. Show release notes
`release_notes` is fetched from GitHub and never displayed anywhere. A
"What's new" view before downloading an update would be genuinely useful.

### F3. Per-app "Check now"
The backend already supports `check_for_updates(app_id)`; the UI only exposes
"Check All".

### F4. Check on launch / periodic checks
The app only checks when asked. An optional "check on launch" and a background
interval (with a notification when updates appear) is the core promise of an
Obtainium-alike.

### F5. Last-checked timestamp is invisible
`last_checked` is stored and updated but never shown. A subtle "Checked 5
minutes ago" in the status bar area would build trust.

### F6. Export/import of the app list
It's already JSON on disk — expose "Export List…"/"Import List…" so users can
share their app collections, Obtainium-style.

### F7. GitLab support
Stubbed everywhere (enum, settings, commented-out UI) but unimplemented. The
GitLab releases API is a close cousin; the adapter trait is implicit already.

### F8. Real install automation
Downloading and revealing in Finder is honest, but mounting a DMG
(`hdiutil attach`), copying the `.app`, and unmounting is doable without
special entitlements for the common case. `.pkg` could hand off to the
Installer app. Checksum verification (the `checksum` field exists, forever
`None`) belongs here too.

### F9. Architecture-aware asset selection **[implementing]**
The TODO in `find_macos_asset` says it: prefer universal > native arch >
other. On an Apple Silicon Mac, an `x86_64.dmg` listed before an `arm64.dmg`
currently wins. Implementing preference ordering: universal → aarch64/arm64 →
x86_64/intel (compile-time native arch first).

---

## Novel / delightful ideas

### D1. A real "Special" menu
System 7 had a **Special** menu; Obtainintosh should too. Put "Empty Trash…"
in it and have it clean the download cache (`obtainintosh-downloads` in tmp),
reporting how much space was freed. Useful *and* bit-perfect nostalgia.

### D2. System 7 copy-progress dialog for downloads
See F1 — the striped barber-pole progress bar with "Items remaining: 1" would
be the most charming download UI shipped this decade.

### D3. Balloon Help everywhere
There's already a `BalloonHelp` component used in a few places. Lean in: a
Help-menu toggle for "Show Balloons" mode, like the real thing.

### D4. "Get Info" window per app
Classic Finder-style Get Info: icon, version, repo link, release notes (F2),
last checked (F5), install path. One window, several missing features solved,
maximum period-correctness.

### D5. Welcome dialog on first run
"Welcome to Obtainintosh." in Chicago with the classic startup chime, shown
once. (Tasteful: only once.)

### D6. Update-count Dock badge
When background checks (F4) find updates, badge the Dock icon with the count —
the one anachronism worth committing.
