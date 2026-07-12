# CI/CD

Obtainintosh ships GitHub Actions workflows for continuous integration and for
cutting desktop releases. CI runs on every pull request and on pushes to `main`;
releases are produced by pushing a version tag. Everything works with **no secrets
configured** — code signing and notarization are optional and only kick in when
the relevant secrets are present.

Obtainintosh is a **Mac OS X** application (an "Obtainium for OS X") and only
bundles a `.dmg`, so its release matrix is macOS-only. Since the system accent
color support, its Rust backend links against AppKit (`objc2-app-kit`), so CI
lints and tests it on a macOS runner to exercise the real target.

## Workflows
| Workflow | Trigger | Purpose |
| --- | --- | --- |
| `.github/workflows/ci.yml` | PRs + pushes to `main` | Type-check and build the SvelteKit frontend, then run `cargo fmt`/`clippy`/`test`. |
| `.github/workflows/release.yml` | Pushing a `v*.*.*` tag, or manual dispatch with a `tag` input | Build the macOS Tauri `.dmg` bundles (Apple Silicon + Intel) and attach them to a GitHub Release. |

## Continuous integration (`ci.yml`)

The CI workflow has two parallel jobs:

- **Frontend (check & build)** — installs npm dependencies with `npm ci`, runs
  `npm run check` (`svelte-kit sync` + `svelte-check`), then `npm run build`
  (`vite build`).
- **Rust (fmt, clippy, test)** — builds the frontend into `build/` first
  (because `tauri::generate_context!` embeds it at compile time), then runs
  `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, and
  `cargo test`. `Swatinem/rust-cache` caches the cargo build between runs. This job
  runs on macOS because the backend's system accent color support
  (`#[cfg(target_os = "macos")]` + `objc2-app-kit`) only compiles there; the
  macOS `.dmg` is built by the release workflow.

Obtainintosh's `src-tauri` is a single crate (not a Cargo workspace), so the cargo
commands run without `--workspace`. There is no `Cargo.toml` at the repository
root — all cargo commands run from `src-tauri/` (the CI steps set
`working-directory: src-tauri`).

Since the Rust job runs on macOS, no Tauri Linux system packages are needed in
CI. If a Linux job is ever added, Tauri v2 needs the `libwebkit2gtk-4.1-dev`
package (the 4.1 series); Tauri v1 projects use `-4.0-dev` instead.

### Running CI checks locally

```bash
# Frontend
npm ci
npm run check
npm run build

# Rust (run from src-tauri/)
cd src-tauri
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

On Linux you also need the Tauri system dependencies (`libwebkit2gtk-4.1-dev`,
`build-essential`, `libxdo-dev`, `libssl-dev`, `libayatana-appindicator3-dev`,
`librsvg2-dev`, etc.). On macOS none of these are required.

## Releases (`release.yml`)

To cut a release:

```
git tag v1.2.3
git push origin v1.2.3
```

Or use the helper, which bumps the version everywhere it's declared (`package.json`, `src-tauri/tauri.conf.json`, `Cargo.toml`) and the README, then creates and pushes the matching tag:

```
scripts/release.sh 1.2.3 --push
```

If a release run fails after the tag is already pushed, there is no need to
delete and re-push the tag: run the **Release** workflow manually from the
Actions tab (workflow_dispatch) and pass the existing tag (e.g. `v1.2.3`) as
the `tag` input. The workflow builds that tagged commit and attaches the
bundles to a release for it. If the failed run already created a draft
release, the re-dispatch reuses it instead of creating a duplicate.

Note that a dispatched run executes the workflow *file* from the branch you
dispatch it on (the Actions tab defaults to `main`) while building the tagged
commit's *source* — so if the workflow changed since the tag, a re-dispatch
runs the new pipeline against the old code, which is usually exactly what you
want for fixing a broken release run.

Re-dispatching a tag whose release is already published temporarily flips the
release back to draft while the bundles rebuild, then re-publishes it — so
nobody downloads from a half-replaced asset list mid-run.

Dispatching a release requires write access to the repository — the same trust
boundary as pushing a `v*.*.*` tag, so the manual path does not widen who can
cut releases. If the repository ever gains multiple writers plus tag
protection rules, gate this workflow with a protected environment (required
reviewers) to keep the two paths equivalent.

The workflow:

1. **Creates a draft GitHub Release** named `Obtainintosh v1.2.3` with
   auto-generated release notes. Tags containing `-` (e.g. `v1.2.3-rc.1`) are
   marked as pre-releases.
2. **Builds the macOS desktop bundles** with `tauri-apps/tauri-action@v0` across a
   two-way matrix and uploads each artifact to the draft release:
   - macOS Apple Silicon (`aarch64-apple-darwin`) — `.dmg` / `.app`
   - macOS Intel (`x86_64-apple-darwin`) — `.dmg` / `.app`

   The `bundle.targets` in `src-tauri/tauri.conf.json` is `"all"`; on a macOS
   runner that resolves to the `.app` and `.dmg` formats.
3. **Publishes the release** (flips it from draft to published) once all build
   jobs succeed. If a build fails, the release stays a draft so nothing
   half-built is published.

> **Why macOS-only?** Obtainintosh is presented as a Mac OS X program and only
> ships a `.dmg`, so the release matrix is restricted to macOS. The Rust backend
> is portable, so if Linux/Windows support is desired later you can extend the
> matrix with `ubuntu-22.04` / `windows-latest` entries (adding the
> `libwebkit2gtk-4.1-dev` Linux dependency step) the same way the other Tauri
> repos do.

Builds are **unsigned** unless the optional signing secrets below are configured.
An unsigned macOS app still runs, but users will see Gatekeeper warnings;
add the Apple secrets later to enable notarization without editing the workflow.

## Secrets

All secrets are **optional** — the workflows build and release successfully
without any of them. They only enable code signing / notarization and Tauri
updater signing.

| Secret | Used for |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64 of the Apple Developer ID signing certificate (.p12). |
| `APPLE_CERTIFICATE_PASSWORD` | Password for the .p12 certificate. |
| `APPLE_SIGNING_IDENTITY` | Signing identity name (e.g. `Developer ID Application: …`). |
| `APPLE_ID` | Apple ID used for notarization. |
| `APPLE_PASSWORD` | App-specific password for that Apple ID. |
| `APPLE_TEAM_ID` | Apple Developer Team ID. |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater private signing key. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for the Tauri updater signing key. |

`GITHUB_TOKEN` is provided automatically by GitHub Actions; you do not need to
create it.
