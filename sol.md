# Obtainintosh Review

Review baseline: `main` at `ac9f737` (`v1.0.0`), 2026-07-12.

This is a static review of the Svelte/Tauri application, its Rust backend, persistence and download flows, build/release configuration, documentation, and the current screenshot. Frontend type-checking could not initially run in this checkout because dependencies were not installed. The repository's hosted CI history and open pull requests were also inspected. Findings marked **Confirmed** follow directly from a reachable code path; roadmap items are intentionally separated from defects.

## Executive Summary

Obtainintosh has a charming and unusually coherent System 7 identity, a small understandable codebase, streamed downloads, atomic file replacement for its JSON store, architecture-aware intent, and a sensible manual-install safety boundary. It is already a good prototype.

It is not yet a dependable updater. The largest issue is false confidence: failed checks are reported as successful, stale versions can remain labeled "Up to Date," generic Linux or Windows archives can be selected as Mac builds, and a slow check can overwrite a user's newer edit. Concurrent downloads can also write to the same file. These are correctness issues in the app's core promise and should precede richer installation features.

The best product direction is to make Obtainintosh an excellent, transparent release resolver and verified downloader before making it an automatic installer. The interface should explain what release and asset it selected, why it selected them, what was verified, and what still requires the user's judgment.

## Existing Work In Flight

Open PR [#9](https://github.com/L-K-M/Obtainintosh/pull/9), `claude/ci-cd-not-running-rig7kc`, already fixes the Cargo working directory in CI, makes release creation retryable, formats/lints the Rust code, and adds default self-tracking. Its current CI checks pass. It touches most backend files, so backend fixes from this review should be based on its head or rebased after it lands rather than duplicating its work.

The repository has no `ANALYSIS.md` at this baseline. `awesome.md` is an older review whose completed items are useful history, but several remaining claims are now stale.

## Priority And Disposition

The disposition indicates what should happen after this review.

| ID | Finding | Severity | Confidence | Effort | Disposition |
|---|---|---:|---:|---:|---|
| C1 | Checks/downloads can overwrite newer edits | Critical | High | M | Implement |
| C2 | Same-named concurrent downloads corrupt each other | Critical | High | M | Implement |
| C3 | Asset selection can choose another OS/CPU | Critical | High | M | Implement |
| C4 | Failed checks are presented as success | High | High | M | Implement |
| C5 | Direct download persists contradictory status | High | High | S | Implement with C2 |
| C6 | Storage mutations survive a failed save | High | High | M | Implement |
| C7 | Plaintext tokens and unrestricted file mode | High | High | M | Roadmap; interim hardening now |
| C8 | Corrupt/old JSON prevents startup | High | High | M | Implement |
| C9 | Latest release without a Mac asset blocks older valid releases | High | High | M | Implement |
| C10 | Checks are sequential and repeatedly scan applications | Medium | High | M | Implement after C1/C4 |
| C11 | GitLab entries are accepted but never work | Medium | High | S | Implement |
| C12 | URL identity/duplicate validation is inconsistent | Medium | High | S-M | Implement with C11 |
| C13 | Download completion is not size-verified and can hang | High | High | M | Implement with C2 |
| C14 | Multiple processes can lose persisted data | Medium | High | S-M | Implement single-instance guard |
| C15 | Rename/install identity is filename-based and stale | Medium | High | M-L | Roadmap foundation |
| F1 | Frontend discards Tauri error strings | Medium | High | S | Implement |
| F2 | Settings load/save races and stale close timer | Medium | High | S | Implement |
| F3 | Add/Edit dialog can retain the wrong values | Medium | High | S | Implement |
| F4 | Overlapping operations break global loading/progress state | High | High | M | Implement with C1/C2/C4 |
| F5 | Repository opening is mouse-only and nonstandard | Medium | High | S | Implement |
| F6 | Unkeyed rows leak avatar failure state after sorting | Low | High | S | Implement with F5 |
| F7 | Modals can overlap and confuse focus | Medium | High | S-M | Implement |
| F8 | Narrow windows clip controls and dialogs | Medium | High | S | Implement |
| F9 | Notifications overlap and theme scope is inconsistent | Low | High | S | Implement |
| F10 | Custom frame exceeds its viewport | Low | High | XS | Implement with F8 |
| E1 | Rust CI/release path is broken on baseline | Critical | High | S | Existing PR #9 |
| E2 | Frontend dependencies have known advisories | High | High | S | Implement |
| E3 | Rust build has no committed lockfile | High | High | S | Implement after PR #9 |
| E4 | Release tag/version sources can disagree | High | High | S | Implement after PR #9 |
| E5 | CSP is disabled and opener permission is broad | Medium | High | S-M | Implement |
| E6 | Test coverage is narrow | Medium | High | M | Implement incrementally |
| E7 | Documentation/package metadata are incomplete | Low | High | S | Implement |

## Confirmed Backend Defects

### C1. Slow operations can overwrite newer user edits

**References:** `src-tauri/src/commands.rs:74-91`, `src-tauri/src/commands.rs:101-149`, `src-tauri/src/commands.rs:179-195`, `src-tauri/src/commands.rs:259-262`, `src-tauri/src/storage.rs:86-96`.

Checks and downloads clone a complete `App`, wait on network or disk work, then replace the complete stored record. If a user edits an app while a check is pending, the old check can restore the old name and URL. A remove can turn the eventual write into an error. The frontend permits these operations to overlap.

**Recommendation:** add narrow storage updates for check-owned and download-owned fields. Before committing network results, re-read the record and verify that its source identity/revision still matches. Do not replace a whole stale record to update `latest_version` or `last_checked`.

### C2. Concurrent downloads can corrupt or substitute installers

**References:** `src-tauri/src/commands.rs:204-220`, `src-tauri/src/commands.rs:275-333`, `src/routes/+page.svelte:39-66`, `src/lib/components/AppRow.svelte:88-99`.

Every asset is written to `$TMPDIR/obtainintosh-downloads/<asset name>`. Two repositories commonly publish names such as `latest.dmg`; a double-click can also start the same operation twice. Both tasks truncate and write the same path, while the UI has only one progress object. The first `done` event hides progress even if another download continues.

**Recommendation:** use a private per-operation directory, create an exclusive `.part` file, verify it, and atomically rename it. Clean up partial files. Track in-flight state by app ID and prevent a duplicate download for the same app; use a queue if multiple apps may download concurrently.

### C3. Asset selection confuses architecture with operating system

**References:** `src-tauri/src/sources.rs:146-200`.

`arm64`, `aarch64`, and `x86_64` are treated as macOS keywords. Consequently, `tool-linux-aarch64.zip` and `tool-windows-x86_64.zip` can be accepted as Mac artifacts. Extension is considered before compatibility, so an incompatible `.dmg` can beat a native `.zip`. Explicitly incompatible architecture assets can also win as a fallback.

**Recommendation:** score all candidates. First hard-reject explicit non-Mac operating systems and incompatible architectures. Require a Mac marker for generic archives. Then rank universal/native/Rosetta-compatible architecture, product-name similarity, GUI/app hints, and package usability. Add architecture-parameterized fixtures so Intel and Apple Silicon cases run on every CI host.

### C4. Per-app check failures are swallowed

**References:** `src-tauri/src/commands.rs:127-152`, `src/lib/util/appStore.ts:65-76`, `src/lib/components/AppRow.svelte:67-84`.

Network, authentication, rate-limit, unsupported-source, parsing, and missing-asset failures are logged and converted into an unchanged app. The frontend announces "Update check completed." Old release data can still display "Up to Date," while a first failure stays "Not Checked."

**Recommendation:** return structured per-app outcomes with a checked timestamp, success/failure code, and useful message. Preserve the last successful release but visibly mark stale/failed results. Show partial and total failure summaries; only show a success notification when every requested check succeeded.

### C5. Downloading an unchecked app creates a false "Not Found" state

**References:** `src-tauri/src/commands.rs:187-200`, `src-tauri/src/commands.rs:259-262`, `src/lib/components/AppRow.svelte:69-79`.

The download command resolves a release but stores only `last_checked`. After reload, `last_checked` is present and `latest_version` is absent, so the row says no Mac version was found immediately after downloading one.

**Recommendation:** persist the exact resolved release/version and asset metadata after a successful verified download. Better, pin and download the release selected during the last check rather than resolving "latest" again.

### C6. A failed persistence operation remains live in memory

**References:** `src-tauri/src/storage.rs:67-117`.

Storage mutates the mutex-protected state, unlocks, then saves. If the write or rename fails, the command reports failure but the rejected mutation remains in memory and can be persisted by a later unrelated save.

**Recommendation:** clone/propose, persist, then swap the in-memory snapshot only after successful replacement, or hold and roll back the old value. Add failure-injection tests.

### C7. API tokens are ordinary JSON secrets

**References:** `src-tauri/src/models.rs:32-36`, `src-tauri/src/storage.rs:14-52`, `src-tauri/src/commands.rs:155-165`.

GitHub/GitLab tokens are serialized in `apps.json`, with permissions inherited from the process umask, and returned to the webview. Every save replaces the file, so manually hardening an old file is not sufficient.

**Recommendation:** move tokens to macOS Keychain and expose only configured/not-configured state to the webview. As an immediate defense, create the directory and replacement file as user-only (`0700`/`0600`) and document the current storage risk.

### C8. A malformed or older data file prevents launch

**References:** `src-tauri/src/storage.rs:27-34`, `src-tauri/src/lib.rs:18-22`, `src-tauri/src/models.rs:32-50`.

Any JSON syntax or required-field mismatch makes `Storage::new` fail, then `run()` panics. There are no schema version/default migrations or recovery UI.

**Recommendation:** introduce a versioned schema and `serde` defaults, preserve an unreadable file under a timestamped backup name, and recover with a clear message rather than making the app unlaunchable.

### C9. Only the nominal latest release is searched for a Mac artifact

**References:** `src-tauri/src/sources.rs:60-120`.

If GitHub's latest stable release is source-only or platform-specific, Obtainintosh fails even when the immediately preceding release has a valid Mac artifact. The release list is consulted only when `/releases/latest` returns 404.

**Recommendation:** inspect a bounded recent release list within the selected channel and choose the newest release containing a sufficiently confident compatible asset. Distinguish "latest has no Mac build" from "repository has no Mac build."

### C10. Check All scales as the sum of every timeout

**References:** `src-tauri/src/commands.rs:114-150`, `src-tauri/src/installer.rs:8-41`.

Checks are fully sequential. Each app independently rescans `/Applications` and `~/Applications` and can launch `PlistBuddy` twice. Thirty failing repositories can approach fifteen minutes at the current timeout, and synchronous filesystem/process work runs inside the async command.

**Recommendation:** first fix C1 and C4, then index installed apps once, move blocking discovery to `spawn_blocking`, perform API requests with bounded concurrency (roughly four), and batch persistence. Respect GitHub rate-limit headers and avoid automatic checks when a recent successful result is fresh.

### C11. GitLab records are accepted but permanently nonfunctional

**References:** `src-tauri/src/sources.rs:219-226`, `src-tauri/src/commands.rs:127-149`, `src-tauri/src/commands.rs:189-199`, `src/lib/components/dialogs/AddAppDialog.svelte:23-27`.

The UI and source detector accept GitLab, but checks and downloads always fail. Because failures are swallowed, Check All can still claim success.

**Recommendation:** reject new GitLab URLs with explicit GitHub-only copy until a complete adapter exists. Preserve the enum only as needed to deserialize existing records and show those records as unsupported rather than silently broken.

### C12. URL validation and repository identity are inconsistent

**References:** `src-tauri/src/sources.rs:122-143`, `src-tauri/src/sources.rs:219-226`, `src-tauri/src/storage.rs:73-96`.

Source detection is substring-based, so a non-GitHub host containing `github.com` can pass add-time validation and fail later. Add duplicate checks handle only case/trailing slash; edit has no duplicate check. `.git`, `www`, deeper release URLs, queries, and fragments can represent the same repository multiple ways.

**Recommendation:** parse once at the command boundary and store/compare a canonical provider/owner/repository key. Enforce the same uniqueness rule atomically for add and edit. Longer term, retain GitHub's immutable repository ID and canonical `full_name` to survive transfers.

### C13. Downloads lack complete transfer validation and idle timeout

**References:** `src-tauri/src/commands.rs:289-333`, `src-tauri/src/sources.rs:28-33`.

Only connection timeout is configured. A server that stops sending after headers can leave the operation open indefinitely. Successful EOF is not compared with GitHub's advertised asset size. There is no digest/signature verification.

**Recommendation:** add an idle/read timeout without imposing a short total timeout on large downloads; reject byte-count mismatches before rename. Model GitHub asset IDs and digests where available, and optionally verify publisher checksum assets without overstating their trust value.

### C14. Multiple app processes can overwrite each other's data

**References:** `src-tauri/src/storage.rs:13-52`.

Each process loads one independent snapshot and uses the same temporary path. Last writer wins, silently discarding changes from another instance.

**Recommendation:** make Obtainintosh single-instance and focus the existing window when relaunched. A multi-process storage protocol is unnecessary complexity for this product.

### C15. Installed app identity is a user-editable filename

**References:** `src-tauri/src/commands.rs:38-43`, `src-tauri/src/installer.rs:4-45`.

The display name doubles as the expected `.app` filename. Repository and bundle names often differ, renaming breaks detection, duplicate bundle names exist, and only two top-level application folders are searched. Stored `install_path` is not revalidated first.

**Recommendation:** separate display and identity. Persist bundle identifier, known install path, short/build versions, and later signer Team ID. Detection should revalidate the known path, match bundle ID in standard locations, offer "Locate App," and use filename only as a legacy fallback.

### Other confirmed backend issues

- Finder reveal errors are logged but the success message still claims the file was revealed (`src-tauri/src/commands.rs:242-265`). Return a structured path/reveal result or propagate the failure.
- The self-update comparator strips prerelease/build suffixes, so stable `1.0.0` is not newer than running `1.0.0-rc.1` (`src-tauri/src/updates.rs:92-117`). Use standards-compliant semver.
- `check_self_update` on this baseline has no explicit request timeout (`src-tauri/src/updates.rs:41-55`); PR #9 changes it to the shared client.
- Mutex poisoning uses `.unwrap()` throughout storage (`src-tauri/src/storage.rs:42-117`). Avoid panicking on poisoned locks while redesigning transactions.

## Frontend, Accessibility, And Interaction

### F1. Tauri's useful error messages are discarded

**References:** `src/lib/util/appStore.ts:24-27`, `src/lib/util/appStore.ts:39-42`, `src/lib/util/appStore.ts:73-76`, `src/lib/components/dialogs/AddAppDialog.svelte:77-79`, `src/lib/components/dialogs/SettingsPanel.svelte:39-40`.

Tauri commonly rejects a Rust `Err(String)` as a JavaScript string. Catch blocks preserve only `Error` instances, replacing duplicate, validation, and storage details with generic messages.

**Recommendation:** centralize unknown-error normalization: preserve strings, then `Error.message`, then a contextual fallback.

### F2. Settings can erase input or close a later dialog

**References:** `src/lib/components/dialogs/SettingsPanel.svelte:10-43`.

The form is editable before `getSettings()` resolves, so a late response can overwrite typed input and an immediate Save can persist null defaults. The post-save timeout is not cleared; closing and reopening within a second lets the old instance close the new dialog.

**Recommendation:** model initial loading separately, disable the form until loaded, ignore late responses after destruction, and clear the timer on destroy (or close immediately).

### F3. Add/Edit state can retain the wrong app values

**References:** `src/lib/components/dialogs/AddAppDialog.svelte:7-21`, `src/routes/+page.svelte:68-71`, `src/routes/+page.svelte:249-255`.

Opening Edit and invoking Cmd+N changes the prop while the component remains mounted. The title changes to Add, but `url` and `name` were initialized once and retain the edited app.

**Recommendation:** use one dialog-state union and key the dialog by mode/app ID, or explicitly reset fields whenever the app identity changes.

### F4. One global busy flag cannot represent concurrent operations

**References:** `src/lib/util/appStore.ts:6-93`, `src/routes/+page.svelte:39-66`, `src/lib/components/Toolbar.svelte:16-21`.

Every operation independently sets `loading` true then false. An older operation can replace a newer app list and clear loading while work remains. Downloads, adds, removes, initial load, and checks all show the toolbar label "Checking...". Progress has no operation identity beyond the latest event.

**Recommendation:** use operation IDs/revisions, a pending counter, and keyed per-app states. Disable only conflicting actions. Model check summaries and a download queue explicitly.

### F5. Opening a repository is mouse-only

**References:** `src/lib/components/AppRow.svelte:34-59`.

A non-focusable table cell handles double-click, with the accessibility warning suppressed. Keyboard and assistive-technology users cannot activate it, and double-click is not discoverable.

**Recommendation:** render the name as a real link/button with an accessible name and normal single activation. Preserve a compact table appearance through styling rather than semantics suppression.

### F6. Avatar fallback state can move to another app after sorting

**References:** `src/lib/components/AppRow.svelte:39-54`, `src/lib/components/AppTable.svelte:158-167`.

Image failure imperatively mutates DOM styles. The row loop is unkeyed, so sorting reuses component/DOM positions and a hidden failed image can be inherited by a different app.

**Recommendation:** key rows by `app.id` and model image failure as component state reset when the URL changes.

### F7. Independent booleans permit stacked modal dialogs

**References:** `src/routes/+page.svelte:29-36`, `src/routes/+page.svelte:68-83`, `src/routes/+page.svelte:249-273`.

Menu shortcuts can open Settings/About/Add while another modal is open. Same-z-index backdrops and focus traps then overlap, with ambiguous Escape and focus restoration.

**Recommendation:** replace booleans with a single modal-state union, plus a separate nonmodal download queue if desired.

### F8. Narrow windows clip essential UI

**References:** `src-tauri/tauri.conf.json:13-21`, `src/lib/components/Toolbar.svelte:31-44`, `src/lib/components/dialogs/DownloadProgressDialog.svelte:52-61`, `src/lib/components/AppTable.svelte:18-24`.

No minimum window size is configured. Toolbar groups do not wrap, dialogs have fixed minimum widths, and five percentage columns compress beyond readability.

**Recommendation:** set a practical minimum size, use viewport-relative dialog widths, and provide horizontal table overflow or compact column behavior. At moderate widths, hide lower-value columns before actions or status.

### F9. Update and ordinary notifications occupy the same position

**References:** `src/routes/+page.svelte:205-221`, `src/lib/components/UpdateNotice.svelte:43-55`, `src/routes/+layout.svelte:21-22`.

Both are fixed at bottom-right with the same z-index. The self-update notice lives outside the runtime `.s7-root` theme scope.

**Recommendation:** use one notification stack inside the themed root, with consistent spacing and prioritization.

### F10. The custom frame is larger than the viewport

**References:** `src/routes/+page.svelte:285-303`, `src/routes/+layout.svelte:10-18`.

`100vw`/`100vh` are content-box dimensions and the border adds two pixels, while document overflow is hidden. Right/bottom chrome can be clipped.

**Recommendation:** apply `box-sizing: border-box`.

### Additional accessibility issues

- Several title-bar controls and dialogs from the locked `@lkmc/system7-ui` package have no accessible names or title association. Fix upstream by adding label props and `aria-labelledby`, then update the dependency.
- Status help wraps non-focusable spans (`src/lib/components/AppRow.svelte:69-79`), so keyboard users cannot discover it. Put essential explanation in ordinary accessible text or use a focusable help control.
- Icon-only actions need verified names from the actual rendered `Button`/icon combination, not only visual balloon help.
- Download progress has no cancellation or close path and uses a fixed 340px minimum (`src/lib/components/dialogs/DownloadProgressDialog.svelte:17-61`).

## Visual And Layout Review

### What works

- The System 7 visual language is confident and memorable rather than generic. The title bar, Geneva/Chicago-like typography, monochrome controls, hard shadows, table chrome, and Finder-style progress concept belong together.
- The screenshot has a clear hierarchy: global actions, sortable data, then per-row actions. Text labels accompany status color/icon cues.
- Runtime macOS accent colors are a thoughtful bridge between the historical theme and the user's current system.

### What needs refinement

- Owner avatars are visually inconsistent with app identity. Multiple apps from one owner receive the same icon; personal photos sit awkwardly beside genuine application marks; eager third-party image requests add privacy/network cost.
- The 16px icon treatment is too small for detailed avatars, while the emoji fallback is platform-rendered and stylistically unrelated. Use a crisp local 24-32px app icon or a deterministic monochrome monogram/document fallback.
- Five equal-ish columns work at the default width but quickly create awkward status wrapping and tightly packed action icons. Status and Actions need minimum widths; Name should flex.
- Hover inversion changes every action button at once and can create visual noise. A selected-row treatment or subtler name highlight would preserve the period look while reducing flicker.
- A nearly empty collection produces a large blank sheet. The empty state should be a useful onboarding panel, and a small list could show a bottom summary such as "5 programs, 2 updates" without filling the window with modern dashboard furniture.
- Pencil/floppy/trash imagery is charming but not always self-evident. Keep balloon help, add keyboard focus visibility, and consider a row selection model with conventional menu commands.
- "Download and Install" naming is misleading because the app only reveals a file. Call it Download or Download and Reveal until installation exists.

## Better App Icons

The app should not treat a repository owner's identity as the application's identity. A practical fallback hierarchy is:

1. User-selected override, cached as normalized PNG.
2. Installed `.app` icon resolved through `NSWorkspace.shared.icon(forFile:)`.
3. Icon from a downloaded and verified app bundle.
4. Optional source-declared Obtainintosh metadata/manifest icon.
5. Carefully bounded repository icon candidates (`app-icon.png`, `icon.png`, Tauri/Electron build icons).
6. Homepage `apple-touch-icon`/favicon fetched by the backend with SSRF, MIME, redirect, byte-size, and decoded-dimension protections.
7. Stable generated monogram/document icon.
8. GitHub owner avatar only as an opt-in legacy fallback.

The best first implementation is installed-app extraction plus deterministic fallback. The project already depends on `objc2-app-kit`; extend the native bridge, rasterize to small PNG representations, and cache by bundle path/identifier/modification time. Do not store large base64 icons in `apps.json`, inject remote SVG into the DOM, or read arbitrary `file://` URLs from the webview.

## Security And Trust Model

### Current boundary

The current manual reveal flow avoids privileged installation, which is good, but HTTPS from GitHub proves transport only. It does not prove the asset belongs to the installed app's publisher.

### Safe download baseline

Persist release ID, tag, asset ID/name/size/digest and the selection explanation. Download that exact asset to a unique partial path, verify size and available digest, apply quarantine metadata, then reveal it with a compact receipt. Never silently re-resolve "latest" between check and download.

### Before automatic installation

Inspect code-signing validity, Developer ID Team Identifier, notarization/Gatekeeper assessment, bundle identifier, and embedded version. Compare signer/bundle continuity with the installed app and require explicit confirmation for changes. Treat `.pkg` separately because it may execute installer scripts and require privileges.

### Tauri surface

`src-tauri/tauri.conf.json:23-25` disables CSP, and `src-tauri/capabilities/default.json:8-17` grants `opener:default`. No current injection was found, but a future webview injection could invoke backend commands and read the token.

**Recommendation:** enable a restrictive local CSP with only the image origins actually needed, and replace default opener scope with explicit HTTPS hosts. Backend-cached local icons would make this easier.

## Engineering And Delivery

### E1. Baseline Rust CI and release delivery are broken

**References:** `.github/workflows/ci.yml:69-79`, `.github/workflows/release.yml:26-41`, `CICD.md:50-53`.

Cargo runs from the repository root although the manifest is under `src-tauri`; hosted checks fail before Clippy/tests. The `v1.0.0` release was not publicly available during review. PR #9 already addresses the principal workflow defects and has green checks; do not duplicate it. Verify both architecture assets and `/releases/latest` after publication.

### E2. The frontend lockfile contains current advisories

`npm audit` reports seven vulnerabilities at this baseline, including high-severity advisories through Vite, SvelteKit, `devalue`, and `picomatch`. SSR-specific issues have limited runtime applicability because this is a static Tauri frontend, but the Vite development-server issues matter when development is exposed through `TAURI_DEV_HOST`.

**Recommendation:** refresh Svelte/SvelteKit/Vite and transitives in an isolated dependency PR, run check/build, add a deliberate audit policy, and enable Dependabot for npm and GitHub Actions.

### E3. Rust builds are not reproducible

**References:** `.gitignore:17-20`, `src-tauri/Cargo.toml:17-36`.

`Cargo.lock` is deliberately ignored for a distributed application, while dependency ranges and the `stable` toolchain move over time.

**Recommendation:** commit `src-tauri/Cargo.lock`, use `--locked` in CI/releases, and document/pin the supported Rust toolchain or MSRV.

### E4. Tags are not checked against shipped versions

Versions are duplicated across `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, and README text. Any matching tag starts a release.

**Recommendation:** add a release preflight comparing the tag to all machine-readable versions and fail before creating/uploading a draft. Prefer one canonical version source where tooling permits.

### E5. Workflow dependencies are movable

GitHub Actions and the Rust `stable` toolchain are tag-based. CI lacks an explicit read-only token permission while release jobs expose signing secrets to build actions.

**Recommendation:** pin actions to commit SHAs, set `permissions: contents: read` in CI, scope write permission to release jobs, and use Dependabot for Actions.

### E6. Test coverage is concentrated in helper functions

There are no frontend tests and only Rust tests for release parsing/selection and self-version comparison. Highest-value additions are:

- Storage transaction, permission, migration, corruption-recovery, and failure-injection tests.
- Check-versus-edit/remove race tests.
- Local HTTP integration tests for stalls, short 200 responses, collision, cleanup, redirects, errors, and size mismatch.
- Asset fixtures for both CPU architectures and explicit Linux/Windows generic archives.
- Structured check outcome and stale-state tests.
- Settings lifecycle, Add/Edit prop-change, keyed sorting/avatar failure, keyboard, and narrow-window component tests.
- Release tag/version and workflow smoke tests.

### E7. Documentation and metadata lag the product

The package description is empty, Cargo metadata is placeholder-like, MIT is declared without a `LICENSE`, and README does not explain supported macOS versions/architectures, signing/notarization, normal binary installation, GitHub-only scope, private repository download limitations, token storage, asset requirements, or the manual installation boundary.

`GENERIC_SOURCES.md` is thoughtful but assumes Apple Silicon in several places and needs an SSRF/trust contract before implementation. `awesome.md` should be archived as historical review or reconciled into maintained analysis.

## Missing Features, Ordered By Foundation

### High-value near-term features

1. Per-app Check Now with visible last successful/attempted times.
2. Search and filters for Updates, Failed, Not Installed, and Ignored.
3. Ignore this release / channel selection (stable, prerelease, custom tag pattern).
4. Release/asset detail with file size, architecture, release notes, and "Why this asset?".
5. Download queue with pause/cancel, one or two concurrent transfers, and a final outcome sheet.
6. Import/export and a constrained, confirm-before-adding `obtainintosh://` deep link.
7. Add preview that validates the repository and selected asset before saving.
8. Drag an existing `.app` into Obtainintosh to capture bundle identity/icon, then attach a GitHub source.
9. Signed self-update through Tauri's updater plugin once release signing is correctly configured.
10. Native summary notifications and a Dock badge while the app is running.

### Larger roadmap

- Background schedules require a product decision: menu-bar lifetime/login item/helper, because the current app exits when closed. Do not present an interval setting that only works while a hidden assumption is true.
- Verified installation of `.dmg`, `.zip`, and `.app.tar.gz` should follow bundle/signer validation. `.pkg` needs a separate, stricter path.
- Installed-app discovery should use bundle metadata and explicit source hints, not silently guess repositories from names.
- Generic web/vendor sources should wait until source adapters can express identity, channel, ambiguity, selected artifact, version confidence, and verification evidence, with SSRF protections.
- Release history and deliberate rollback become safe only after artifacts and signer continuity are pinned and verified.

## Delightful Ideas That Fit The Product

- **Resolver Inspector:** a System 7 Get Info window showing source identity, release, selected asset, confidence signals, digest, bundle ID, and signer.
- **SimpleText Release Notes:** open release notes from the Latest cell in a small period-appropriate document window.
- **Update Receipt:** after a download, save/show source, release ID, asset ID, expected/actual bytes, digest, and verification status.
- **Marching-Ants Drag To Add:** accept a GitHub URL or `.app` with classic drag feedback and an explicit preview before commitment.
- **Smart Digest:** one useful sentence such as "3 updates available, 1 needs a choice, 1 failed authentication" instead of notification spam.
- **Finder Copy Queue:** evolve the current progress dialog into a compact queue with remaining items and cancellation.
- **Time Machine History:** once verification exists, browse prior releases and rollback points with a deliberately theatrical historical UI.
- **Doggcow Easter Egg:** a small, optional About-dialog interaction is on-brand; avoid startup sounds or blocking joke error dialogs that reduce usability.

## Recommended Implementation Sequence

1. Let PR #9 land or base backend work on it.
2. Preserve frontend error details and fix settings/Add dialog lifecycle races.
3. Fix asset platform/architecture selection with fixtures.
4. Fix unique partial downloads, completion validation, direct-download state, and progress identity.
5. Add structured check outcomes and narrow revision-safe persistence updates.
6. Make persistence transactional/recoverable and enforce single instance.
7. Canonicalize GitHub identity and stop accepting new GitLab records.
8. Add bounded check concurrency and one installed-app index.
9. Refresh frontend dependencies, enable CSP, and tighten opener scope.
10. Establish bundle identity and local icon extraction.
11. Add resolver details, per-app actions, filters, release notes, and download receipts.
12. Move tokens to Keychain and build verified-install capabilities before any automatic replacement.

This ordering deliberately favors truthful state, deterministic selection, safe downloads, and recovery over adding more automation to an unreliable base.

## Implementation Pull Requests

The high-confidence implementation work from this review was split into small branches. Stacked PRs intentionally target the preceding branch so each diff contains one concern.

### Frontend and presentation

| PR | Change | Base |
|---|---|---|
| [#14](https://github.com/L-K-M/Obtainintosh/pull/14) | Preserve Tauri error details | `main` |
| [#10](https://github.com/L-K-M/Obtainintosh/pull/10) | Fix settings lifecycle races | `sol/frontend-errors` |
| [#12](https://github.com/L-K-M/Obtainintosh/pull/12) | Reset Add/Edit state when mode changes | `sol/settings-lifecycle` |
| [#16](https://github.com/L-K-M/Obtainintosh/pull/16) | Prevent overlapping modal dialogs | `sol/add-dialog-state` |
| [#11](https://github.com/L-K-M/Obtainintosh/pull/11) | Make repository links keyboard accessible | `main` |
| [#13](https://github.com/L-K-M/Obtainintosh/pull/13) | Keep avatar failure state with keyed app rows | `sol/repository-link-accessibility` |
| [#17](https://github.com/L-K-M/Obtainintosh/pull/17) | Stack app and self-update notifications | `main` |
| [#15](https://github.com/L-K-M/Obtainintosh/pull/15) | Fix constrained window layout | `main` |
| [#18](https://github.com/L-K-M/Obtainintosh/pull/18) | Refresh vulnerable frontend dependencies | `main` |
| [#19](https://github.com/L-K-M/Obtainintosh/pull/19) | Harden CSP and URL opener scope | `main` |
| [#20](https://github.com/L-K-M/Obtainintosh/pull/20) | Document usage and security boundaries | `main` |

### Delivery and process safety

| PR | Change | Base |
|---|---|---|
| [#9](https://github.com/L-K-M/Obtainintosh/pull/9) | Existing CI/release repair and self-tracking work | `main` |
| [#22](https://github.com/L-K-M/Obtainintosh/pull/22) | Commit and enforce the Rust lockfile | PR #9 branch |
| [#21](https://github.com/L-K-M/Obtainintosh/pull/21) | Enforce single-instance startup before storage | `sol/cargo-lock` |
| [#23](https://github.com/L-K-M/Obtainintosh/pull/23) | Guard release tags against shipped versions | PR #9 branch |

### Resolver, download, and storage stack

| PR | Change | Base |
|---|---|---|
| [#24](https://github.com/L-K-M/Obtainintosh/pull/24) | Fix macOS/CPU asset compatibility ranking | PR #9 branch |
| [#25](https://github.com/L-K-M/Obtainintosh/pull/25) | Harden release asset downloads | `sol/backend-assets` |
| [#26](https://github.com/L-K-M/Obtainintosh/pull/26) | Enforce canonical GitHub source identity | `sol/download-safety` |
| [#27](https://github.com/L-K-M/Obtainintosh/pull/27) | Keep failed storage mutations out of memory | `sol/source-validation` |
| [#28](https://github.com/L-K-M/Obtainintosh/pull/28) | Recover safely from corrupt/older storage | `sol/storage-transactions` |
| [#29](https://github.com/L-K-M/Obtainintosh/pull/29) | Restrict storage permissions | `sol/storage-recovery` |
| [#31](https://github.com/L-K-M/Obtainintosh/pull/31) | Report and persist update-check outcomes safely | `sol/storage-permissions` |
| [#30](https://github.com/L-K-M/Obtainintosh/pull/30) | Serialize downloads for truthful progress | `sol/check-outcomes` |
| [#32](https://github.com/L-K-M/Obtainintosh/pull/32) | Bound and accelerate batch checks | `sol/download-serialization` |
| [#33](https://github.com/L-K-M/Obtainintosh/pull/33) | Select a recent compatible release | `sol/check-outcomes` |

PR #9 should land before its dependent backend roots. The standalone PRs based on `main` currently inherit the baseline's broken Rust CI job; that failure is fixed by PR #9 rather than by duplicating workflow edits in every branch. Their frontend checks pass, and the pull-request checks should be rerun after PR #9 updates `main`.
