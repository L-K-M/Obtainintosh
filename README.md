# Obtainintosh

Obtainintosh is a desktop app for macOS and Linux that tracks applications distributed through GitHub or Forgejo releases. It compares releases with what is installed — `.app` bundles in `/Applications` or `~/Applications` on macOS; dpkg packages and AppImages on Linux — downloads a release asset suitable for the platform it runs on, and reveals the downloaded file in the file manager.

Obtainintosh currently supports GitHub repositories and Forgejo instances, including private instances that require a username and an application key. GitLab and arbitrary download pages are not supported.

> [!IMPORTANT]
> Obtainintosh does not install or replace applications. It downloads and reveals release files; you must inspect and install them yourself.

**Latest release:** v<!-- version -->1.6.0<!-- /version --> | [Download from GitHub Releases](https://github.com/L-K-M/Obtainintosh/releases/latest)

![Main window screenshot showing a list of programs in different states](./screenshot.png)

> [!NOTE]
> This project was developed with assistance from large language models (AI coding tools).

## Platform support

The release workflow builds Obtainintosh for:

- macOS on Apple Silicon (`aarch64-apple-darwin`), as a `.dmg`
- macOS on Intel (`x86_64-apple-darwin`), as a `.dmg`
- Linux on x86_64 (`x86_64-unknown-linux-gnu`), as a `.deb` and an `.AppImage`

No minimum macOS version is currently documented. The Linux packages are built on Ubuntu 22.04, so they run on Ubuntu 22.04 and later; the `.deb` targets Debian-family distributions, and the `.AppImage` should work on any distribution with a comparable glibc. Obtainintosh is not released for Windows.

Release builds can be produced without Apple signing credentials. The workflow enables signing and notarization only when the required secrets are configured, so this repository does not guarantee that a given release asset is signed or notarized. Check the release details and verify that downloads came from this repository before bypassing any macOS security warning. The Linux packages are not signed.

## Install a release

On macOS:

1. Open the [latest GitHub Release](https://github.com/L-K-M/Obtainintosh/releases/latest).
2. Download the `.dmg` for your Mac's architecture: Apple Silicon or Intel.
3. Open the disk image and copy Obtainintosh to your Applications folder.

On Ubuntu (and other Debian-family distributions):

1. Open the [latest GitHub Release](https://github.com/L-K-M/Obtainintosh/releases/latest).
2. Download the `.deb` and install it (double-click it, or run `sudo apt install ./obtainintosh_<version>_amd64.deb`), **or** download the `.AppImage`, mark it executable (`chmod +x`), and run it directly. Keeping AppImages in `~/Applications` lets Obtainintosh detect their versions.

## Use Obtainintosh

1. Choose **Add Program** and enter a repository URL such as `https://github.com/owner/project` or `https://codeberg.org/owner/project`.
2. Enter the application's name so Obtainintosh can detect its installed version: on macOS, the bundle name as it appears in `/Applications` or `~/Applications`; on Linux, the name of its dpkg package (case and spaces don't matter — "My App" finds `my-app`) or of an AppImage in `~/Applications` or `~/.local/bin`. An installed package takes precedence over an AppImage of the same name, and when several versions of an AppImage sit side by side, the highest version counts as the installed one.
3. Leave **Source** on *Detect automatically* for a `github.com` repository, and choose *Forgejo* for an instance whose address does not identify the software. See [Forgejo instances](#forgejo-instances).
4. Check for updates. Obtainintosh also checks tracked applications when it starts.
5. Download an available update. Obtainintosh saves the asset to a temporary directory and reveals it in the file manager (Finder on macOS).
6. Open the downloaded file (`.dmg`, `.pkg`, `.deb`, archive, …) and complete installation manually. An `.AppImage` just needs to be marked executable and moved wherever you keep AppImages.

Obtainintosh reads the latest published release. If a repository has no normal latest release, it can fall back to the newest non-draft release, including a prerelease.

## Forgejo instances

Forgejo is self-hosted, so an instance can be at any address. Obtainintosh recognises `codeberg.org` and hosts that name the software (for example `forgejo.example.org` or `gitea.example.org`) on its own. For anything else — `https://git.example.com/owner/project` — choose **Forgejo** under **Source** in the Add Program dialog, otherwise the URL is rejected as an unsupported source.

The repository must be the first two path segments of the URL: `<instance>/<owner>/<repository>`. Anything after that (`/releases`, `/src/branch/main`) is ignored, as is a trailing `.git`. An instance served under a URL path prefix is not supported.

### Private instances and repositories

A private instance rejects anonymous API reads, so Obtainintosh needs credentials for it. In the Add Program dialog, with **Source** set to *Forgejo*, fill in:

- **Forgejo Username** — the account name on that instance.
- **Application Key** — an access token generated on the instance under **Settings → Applications → Generate New Token**. Read access to the repository is enough.

Credentials are stored per tracked program, because every self-hosted instance issues its own key. They are sent to that instance only: the release lookup and the asset download both use them, and Obtainintosh does not attach them to a URL on any other host. Leave both fields blank for a public repository.

A username and key are sent as HTTP Basic credentials, which Forgejo accepts both on its API and on the routes that serve release assets. If an instance rejects that pair, leave **Forgejo Username** blank and enter only the application key: Obtainintosh then authenticates with Forgejo's `Authorization: token` scheme instead.

> [!WARNING]
> An `http://` instance address is accepted, because a Forgejo box on a local network often has no certificate. Both authentication schemes carry the application key in a request header, so over plain HTTP anyone on the network path can read it. Obtainintosh logs a warning each time it sends credentials that way. Prefer `https://` for anything beyond a trusted local network.

## Supported release assets

Repositories must publish an asset for the platform Obtainintosh runs on. Asset selection depends on filenames; Obtainintosh does not inspect archive contents or verify checksums.

On macOS, it looks for these formats in priority order:

1. `.dmg`
2. `.pkg`
3. `.app.tar.gz`
4. `.tar.gz`
5. `.zip`

For generic archives such as `.tar.gz` and `.zip`, the filename must identify macOS or a supported architecture with a term such as `mac`, `macos`, `darwin`, `osx`, `universal`, `arm64`, `aarch64`, or `x86_64`. Universal assets are preferred, followed by assets matching the Mac's native architecture.

On Linux, the priority order is:

1. `.deb`
2. `.AppImage`
3. `.tar.gz`
4. `.zip`

For generic archives, the filename must identify Linux with a term such as `linux`, `ubuntu`, or `debian`. Assets naming the machine's architecture (`amd64`, `x86_64`, `arm64`, `aarch64`) are preferred over unmarked ones, and assets naming a different architecture are skipped — there is no Rosetta on Linux. `.rpm` assets are not used.

## GitHub token and rate limits

Public repositories work without a token, but unauthenticated GitHub API requests have a lower rate limit. If checks are being rate-limited, add an optional GitHub personal access token under **Settings**. A fine-grained token with read-only access to public repositories is sufficient for public release checks. GitHub determines the applicable limits and reset time.

> [!WARNING]
> The token is currently stored in plaintext in `apps.json` (see [Data locations](#data-locations)) alongside tracked applications and settings. Forgejo application keys are stored the same way, next to the program they belong to. Use narrowly scoped credentials and protect that file. Obtainintosh does not currently store tokens in the macOS Keychain or a Linux keyring.

## Data locations

- Tracked applications, settings, tokens, and Forgejo application keys: `~/Library/Application Support/Obtainintosh/apps.json` on macOS; `$XDG_DATA_HOME/Obtainintosh/apps.json` (usually `~/.local/share/Obtainintosh/apps.json`) on Linux
- Downloads: the system temporary directory under `obtainintosh-downloads`

The download directory is temporary and may be cleared by the operating system. Move files elsewhere if you need to keep them.

## Build from source

Install Node.js 20, npm, and the stable Rust toolchain. On Ubuntu, also install the Tauri v2 system prerequisites:

```bash
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Then run:

```bash
git clone https://github.com/L-K-M/Obtainintosh.git
cd Obtainintosh
npm ci
npm run tauri dev
```

To create local bundles (a `.dmg` on macOS; a `.deb` and an `.AppImage` on Linux):

```bash
npm run tauri build
```

Bundles for the host architecture are written below `src-tauri/target/release/bundle/`. The release workflow cross-builds the separate Apple Silicon and Intel targets and builds the Linux packages on Ubuntu 22.04.

## Troubleshooting

### `No published releases found`

The repository has no published releases at all. Draft releases are visible only to users with write access, so a release whose publishing step failed — for example, a release workflow that uploads assets and marks the release published only at the end — can show on the releases page for the repository's maintainers while Obtainintosh (and every other anonymous reader) sees an empty repository. Check the repository's Actions runs and publish the draft release.

### No macOS-compatible or Linux-compatible asset found

- Confirm that the repository has a published release with an asset in one of the formats listed under [Supported release assets](#supported-release-assets) for your platform.
- For `.tar.gz` and `.zip` files, confirm that the filename clearly identifies the platform or the architecture using one of the terms listed above.
- Confirm that the release provides an asset matching your machine's architecture (or, on macOS, a universal one).
- If the project's naming does not match Obtainintosh's rules, download the correct asset directly from its release page.

### GitHub API rate limit or `403` error

- Wait until GitHub resets the rate limit, then check again.
- Avoid repeatedly checking many tracked repositories.
- Add a valid, narrowly scoped GitHub token in **Settings**, or replace an expired token.

### Unsupported source URL

- The URL is not a `github.com` repository and does not identify a Forgejo instance by name. Choose **Forgejo** under **Source** in the Add Program dialog.

### `Repository not found on this Forgejo instance`

- Confirm the URL is `<instance>/<owner>/<repository>`, and that the repository has at least one published release.
- For a private repository, confirm the username and application key are filled in and that the key has read access to it. Forgejo answers the same way for a repository that does not exist and for one the credentials cannot see.

### `Forgejo rejected the credentials`

- Regenerate the application key under **Settings → Applications** on the instance and re-enter it, or check that the key has not expired.
- If the instance does not accept a username and key pair, clear **Forgejo Username** and enter only the application key.

## License

Obtainintosh is available under the [MIT License](./LICENSE).
