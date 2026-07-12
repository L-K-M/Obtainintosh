# Obtainintosh Future Work

Updated from the complete 2026-07-12 review in [`sol.md`](./sol.md). This is the maintained backlog, not a record of completed fixes. Items implemented in open pull requests were removed from this document; their rationale, details, and PR links remain in `sol.md`.

## Product Principle

Make Obtainintosh a transparent, trustworthy release resolver and downloader before turning it into an automatic installer. A user should be able to answer:

- Which repository, release, and asset did it select?
- Why is that asset believed to be compatible with this Mac and this app?
- What identity, size, digest, bundle, and signer information was verified?
- Which facts are stale, ambiguous, unavailable, or based on heuristics?
- What will happen when the user clicks the action?

## P0: Trust And Identity

### Move tokens to macOS Keychain

Interim owner-only storage permissions reduce local exposure, but GitHub/GitLab tokens remain plaintext in `apps.json` and its recovery backups and are returned to the webview. Move secrets to Keychain, migrate and erase existing plaintext values, and expose only configured/not-configured state to the frontend. A GitHub device authorization flow would be friendlier than asking users to paste PATs.

Current references: `src-tauri/src/models.rs`, `src-tauri/src/storage.rs`, `src-tauri/src/commands.rs`, `src/lib/components/dialogs/SettingsPanel.svelte`.

### Establish stable application identity

Stop treating the editable display name as the installed application identity. Persist and validate:

- GitHub repository ID and canonical `full_name`.
- macOS `CFBundleIdentifier`.
- Known install path.
- `CFBundleShortVersionString` and `CFBundleVersion`.
- Code-signing Team Identifier.
- Selected channel and asset rule.

Detection order should revalidate the known path, match bundle ID in standard locations, offer Locate App, and use filename matching only as a legacy fallback. This is prerequisite work for dependable icons, signer continuity, installation, and renamed/transferred repositories.

Current references: `src-tauri/src/installer.rs`, `src-tauri/src/commands.rs`, `src-tauri/src/models.rs`.

### Pin and verify release artifacts

Checks should persist the exact GitHub release ID, asset ID, filename, expected size, API digest, and resolved URL. Downloads should consume that pinned asset rather than re-resolving latest. Add:

- GitHub SHA-256 digest verification where available.
- Optional publisher checksum-file support with clear trust wording.
- macOS quarantine metadata before revealing a file.
- A compact download receipt containing source, release, asset, bytes, digest, and verification status.
- Authenticated asset API downloads for private repositories; public `browser_download_url` behavior is insufficient for all private-release cases.

Size validation and unique partial publication are implemented separately, but cryptographic/provenance verification is not.

Current references: `src-tauri/src/models.rs`, `src-tauri/src/sources.rs`, `src-tauri/src/commands.rs`.

### Define version comparison policy

Move authoritative comparison out of the table UI and return `newer`, `equal`, `older`, or `incomparable` from the backend. Preserve prerelease semantics, calendar versions, build versions, product-prefixed tags, and per-source extraction patterns. Never render an incomparable version as Up to Date.

Also replace the self-update comparator that ignores prerelease/build suffixes; a stable `1.0.0` must be newer than `1.0.0-rc.1`.

Current references: `src/lib/components/AppTable.svelte`, `src-tauri/src/sources.rs`, `src-tauri/src/updates.rs`.

## P1: Resolver And Download Experience

### Add explicit asset selection and explanation

Heuristic ranking can never disambiguate every multi-product release. Add a release preview and per-app include/exclude rule or selected asset pattern. Show:

- Asset filename, size, package format, and architecture.
- Positive and negative scoring signals.
- Rejected candidates and reasons.
- A confidence threshold that asks instead of guessing.

A System 7 Resolver Inspector or Get Info window is a fitting presentation for this data.

### Add release channels and history

Support Stable, Prerelease/Beta, and custom tag patterns per app. A channel policy should govern recent-release fallback rather than silently mixing stable and prerelease artifacts. Show compatible release history and allow deliberate rollback only after artifact/signer verification exists.

### Build a real download queue

Global serialization keeps the current single progress dialog truthful, but the intended experience should support:

- A keyed queue with pending/running/completed/failed states.
- Cancel, retry, and optional resume.
- Disk-space checks and idle/overall diagnostics.
- One or two concurrent transfers at most.
- Cleanup/retention policy for completed temporary downloads.
- A final outcome sheet for batch operations.

Until installation exists, call batch actions Download All rather than Update All.

### Expose useful per-app actions

Add Check Now, Ignore This Release, mute notifications, view release notes, choose channel, and inspect selected asset. The backend command already accepts a single app ID; the UI should make it available once structured operation state is settled.

### Search, filter, and summarize

For larger collections, add search and filters for Updates, Failed, Not Installed, Unsupported, and Ignored. Persist sort/filter preference. A small footer such as `18 programs, 3 updates, 1 failed` would use empty space better without turning the app into a dashboard.

## P1: Better App Icons

Repository-owner avatars are not application identity, repeat for multi-product owners, leak an eager third-party request per row, and mix personal photographs with app artwork.

Use this fallback hierarchy:

1. User-selected override.
2. Installed `.app` icon resolved through `NSWorkspace.shared.icon(forFile:)`.
3. Icon from a downloaded and verified app bundle.
4. Optional source-declared Obtainintosh metadata icon.
5. Bounded repository icon candidates such as Tauri/Electron build icons.
6. Homepage `apple-touch-icon`/favicon with SSRF, MIME, redirect, size, and dimension controls.
7. A deterministic local monogram/document icon.
8. Owner avatar only as an opt-in legacy fallback.

The first implementation should batch native icon extraction, rasterize small PNG representations in Rust, and cache by bundle path/identifier/modification time. Do not store large base64 images in `apps.json`, insert remote SVG into the DOM, or expose arbitrary file URLs to the webview.

Current references: `src/lib/components/AppRow.svelte`, `src-tauri/Cargo.toml`.

## P2: Installation And Onboarding

### Add a validation preview before tracking

Pasting a repository should fetch and show canonical identity, description, selected channel, latest compatible release, selected asset, architecture, size, and installed-app match before saving. Ambiguous assets should require a choice and remember it.

Add clipboard detection and drag-and-drop for GitHub URLs. Keep a clear statement that current actions download and reveal an installer rather than installing it.

### Add existing-app onboarding

Allow dropping or choosing an installed `.app`. Extract bundle ID, versions, install path, icon, and signing identity first, then ask for or infer a source using explicit metadata. Do not silently guess a GitHub repository from an app name.

### Design verified installation

Support `.dmg`, `.zip`, and `.app.tar.gz` first. Mount/extract in a controlled temporary location, inspect bundle ID/version/signature/notarization, compare signer continuity, show the proposed change, and copy with explicit consent. Treat `.pkg` separately because it can execute scripts and require privileges.

Useful trust modes:

- Strict: require valid signing/notarization and signer continuity.
- Ask: warn and confirm unsigned or changed signers.
- Permissive: manual download only, visibly unverified.

Rollback or Time Machine history should preserve a prior verified bundle only after this trust model exists.

### Add portable collections

Provide validated JSON import/export and, later, a constrained `obtainintosh://add?repo=owner/repo` deep link. Imported entries must still show source/asset/trust confirmation.

### Decide provider expansion deliberately

GitLab can be reintroduced only with a complete adapter, authentication, tests, and honest UI. Generic vendor pages need an adapter contract that expresses identity, channel, ambiguity, selected artifact, version confidence, and verification evidence. Arbitrary fetching must block private-network SSRF, constrain redirects and response sizes, validate MIME, and handle dynamic pages as unsupported rather than guessing.

Current design notes: `GENERIC_SOURCES.md`.

## P2: Notifications And Self-Update

### Add scheduling only with clear lifetime semantics

Offer launch/daily/weekly/manual checks, last successful check, next check, quiet hours, and per-app mute. Notify only on a changed release or recovery from failure, summarize multiple updates, focus/filter the app when clicked, and set the Dock badge to actionable updates.

True background checks require a product decision: menu-bar lifetime/login item/helper. Do not add an interval setting that silently works only while the window remains open.

### Add signed Obtainintosh self-update

Use Tauri's updater plugin for Obtainintosh itself with signed architecture-specific artifacts, embedded public key, progress, confirmation, install/relaunch, key rotation, and private-key backup policy. Keep third-party update trust separate; Tauri's updater signature proves only Obtainintosh releases.

Current references: `src-tauri/src/updates.rs`, `src/lib/updateChecker.ts`, `src/lib/components/UpdateNotice.svelte`.

## UX And Accessibility

### Replace the remaining global operation model

The app store still has one `loading` flag for initial load, checks, adds, removes, and downloads. Introduce operation IDs, a pending counter, and keyed per-app states. Disable only conflicting actions, prevent stale frontend responses from replacing a newer list, and use operation-specific labels rather than always saying Checking.

Backend narrow writes prevent persisted edit loss, and global download serialization keeps progress truthful, but frontend concurrency semantics still need explicit modeling.

Current references: `src/lib/util/appStore.ts`, `src/routes/+page.svelte`, `src/lib/components/Toolbar.svelte`.

### Finish component-level accessibility upstream

`@lkmc/system7-ui` title-bar controls and dialogs need accessible names and title association. Add label props and `aria-labelledby` in the shared package, verify icon-only button names in rendered output, and make status help keyboard/screen-reader discoverable through real text or `aria-describedby`.

Current references: `src/routes/+page.svelte`, dialog components, and the locked UI package.

### Improve empty and selected states

Replace the one-line empty table with concise onboarding actions. Consider conventional row selection, keyboard navigation, menu commands, and a subtler selected/hover treatment; inverting every action button on row hover is visually noisy. Keep the System 7 language rather than adding modern cards and dashboard chrome.

### Surface release notes

Release notes are already fetched but unused. Open the Latest value into a SimpleText-style notes window and distinguish notes from verification information.

## Engineering Backlog

### Add frontend and integration tests

There is still no frontend test framework. Highest-value coverage:

- Deferred-promise tests for request ordering and pending state.
- Settings/Add/Edit/modal lifecycle tests.
- Keyboard and axe checks for links, dialogs, title-bar controls, tooltips, and icon actions.
- Responsive snapshots around the minimum window size.
- Local HTTP download tests for redirects, stalls, short bodies, mismatches, cleanup, and cancellation.
- Check/edit/remove race tests at command level.
- GitHub response fixtures for rate limits, private repositories, malformed payloads, drafts, channels, and ambiguous assets.
- Release-script and workflow smoke tests.

The implemented Rust branches add substantial helper/storage regression coverage, but they do not replace end-to-end network/UI tests.

### Align Tauri package versions

The locked Rust graph currently resolves Tauri 2.11.x while the npm API/CLI tree is 2.9.x. A native CLI diagnostic reports this mismatch even though direct Cargo and frontend builds pass. Align compatible Rust and JavaScript Tauri releases, regenerate both lockfiles, and perform a real macOS bundle smoke test.

### Harden supply chain and CI further

- Pin GitHub Actions to full commit SHAs.
- Declare `contents: read` for CI and scope release write permission per job.
- Pin/document a supported Rust toolchain or MSRV rather than moving `stable`.
- Enable Dependabot for npm, Cargo, and GitHub Actions.
- Add a deliberate audit/advisory policy rather than relying on occasional manual checks.

### Define the macOS support matrix

Choose, configure, and document a minimum macOS version. Exercise architecture-dependent tests for both Intel and Apple Silicon, run clean-install smoke tests on both, and verify Intel behavior without accidental host-architecture assumptions.

### Improve recovery communication and logging

Corrupt storage is backed up and recovered by an implementation PR, but the diagnostic currently targets process output. Surface a startup recovery notice with the backup path and a safe Open Folder action. Add useful release-build logging without exposing tokens.

### Decide crash durability requirements

Atomic rename protects against partial JSON reads, but full power-loss durability may require syncing the replacement file and parent directory. Implement only if the product requires that guarantee; do not add complexity without a stated durability target.

## Delightful, Coherent Ideas

- Resolver Inspector: Get Info for release, asset, score, digest, bundle, and signer.
- SimpleText Release Notes.
- Download receipts as small provenance documents.
- Marching-ants drag-to-add for URLs and `.app` bundles.
- Smart digest: `3 updates, 1 choice needed, 1 authentication failure`.
- Finder copy queue with remaining items and cancellation.
- Time Machine release history after verified rollback is safe.
- A small Dogcow interaction in About; avoid startup sounds or joke error dialogs that impede normal use.

## Suggested Next Sequence

1. Establish bundle/repository/signer identity.
2. Pin release and asset metadata; verify API digests and quarantine downloads.
3. Move tokens to Keychain and support authenticated private assets.
4. Add Resolver Inspector, ambiguity handling, and per-app asset/channel rules.
5. Replace global frontend operation state and add per-app actions/search/filters.
6. Implement native app icons and existing-app onboarding.
7. Add frontend/network integration tests and align Tauri versions/support matrix.
8. Add signed self-update and well-defined scheduling.
9. Design verified installation and rollback only after the trust foundation is complete.
