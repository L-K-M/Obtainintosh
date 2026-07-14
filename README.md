# Obtainintosh

Obtainintosh is a macOS desktop app for tracking applications distributed through GitHub Releases. It compares releases with applications found in `/Applications` or `~/Applications`, downloads a suitable macOS release asset, and reveals the downloaded file in Finder.

Obtainintosh currently supports GitHub repositories only. GitLab and arbitrary download pages are not supported.

> [!IMPORTANT]
> Obtainintosh does not install or replace applications. It downloads and reveals release files; you must inspect and install them yourself.

**Latest release:** v<!-- version -->1.2.1<!-- /version --> | [Download from GitHub Releases](https://github.com/L-K-M/Obtainintosh/releases/latest)

![Main window screenshot showing a list of programs in different states](./screenshot.png)

> [!NOTE]
> This project was developed with assistance from large language models (AI coding tools).

## Platform support

The release workflow builds Obtainintosh for:

- Apple Silicon (`aarch64-apple-darwin`)
- Intel (`x86_64-apple-darwin`)

No minimum macOS version is currently documented. Obtainintosh is not released for Windows or Linux.

Release builds can be produced without Apple signing credentials. The workflow enables signing and notarization only when the required secrets are configured, so this repository does not guarantee that a given release asset is signed or notarized. Check the release details and verify that downloads came from this repository before bypassing any macOS security warning.

## Install a release

1. Open the [latest GitHub Release](https://github.com/L-K-M/Obtainintosh/releases/latest).
2. Download the `.dmg` for your Mac's architecture: Apple Silicon or Intel.
3. Open the disk image and copy Obtainintosh to your Applications folder.

## Use Obtainintosh

1. Choose **Add Program** and enter a GitHub repository URL such as `https://github.com/owner/project`.
2. Enter the application's bundle name as it appears in `/Applications` or `~/Applications` so Obtainintosh can detect its installed version.
3. Check for updates. Obtainintosh also checks tracked applications when it starts.
4. Download an available update. Obtainintosh saves the asset to a temporary directory and reveals it in Finder.
5. Open the downloaded `.dmg`, `.pkg`, or archive and complete installation manually.

Obtainintosh reads the latest published GitHub Release. If a repository has no normal latest release, it can fall back to the newest non-draft release, including a prerelease.

## Supported release assets

Repositories must publish a macOS asset on GitHub Releases. Obtainintosh looks for these formats in priority order:

1. `.dmg`
2. `.pkg`
3. `.app.tar.gz`
4. `.tar.gz`
5. `.zip`

For generic archives such as `.tar.gz` and `.zip`, the filename must identify macOS or a supported architecture with a term such as `mac`, `macos`, `darwin`, `osx`, `universal`, `arm64`, `aarch64`, or `x86_64`. Universal assets are preferred, followed by assets matching the Mac's native architecture. Asset selection depends on filenames; Obtainintosh does not inspect archive contents or verify checksums.

## GitHub token and rate limits

Public repositories work without a token, but unauthenticated GitHub API requests have a lower rate limit. If checks are being rate-limited, add an optional GitHub personal access token under **Settings**. A fine-grained token with read-only access to public repositories is sufficient for public release checks. GitHub determines the applicable limits and reset time.

> [!WARNING]
> The token is currently stored in plaintext in `~/Library/Application Support/Obtainintosh/apps.json` alongside tracked applications and settings. Use a narrowly scoped token and protect that file. Obtainintosh does not currently store tokens in macOS Keychain.

## Data locations

- Tracked applications, settings, and tokens: `~/Library/Application Support/Obtainintosh/apps.json`
- Downloads: the macOS temporary directory under `obtainintosh-downloads`

The download directory is temporary and may be cleared by macOS. Move files elsewhere if you need to keep them.

## Build from source

Install Node.js 20, npm, the stable Rust toolchain, and the prerequisites for building Tauri applications on macOS. Then run:

```bash
git clone https://github.com/L-K-M/Obtainintosh.git
cd Obtainintosh
npm ci
npm run tauri dev
```

To create a local application bundle and disk image:

```bash
npm run tauri build
```

Bundles for the host architecture are written below `src-tauri/target/release/bundle/`. The release workflow cross-builds the separate Apple Silicon and Intel targets.

## Troubleshooting

### No macOS-compatible asset found

- Confirm that the repository has a published GitHub Release with a `.dmg`, `.pkg`, `.app.tar.gz`, `.tar.gz`, or `.zip` asset.
- For `.tar.gz` and `.zip` files, confirm that the filename clearly identifies macOS or the architecture using one of the terms listed above.
- Confirm that the release provides a universal asset or one matching your Mac's architecture.
- If the project's naming does not match Obtainintosh's rules, download the correct asset directly from its GitHub Release.

### GitHub API rate limit or `403` error

- Wait until GitHub resets the rate limit, then check again.
- Avoid repeatedly checking many tracked repositories.
- Add a valid, narrowly scoped GitHub token in **Settings**, or replace an expired token.

## License

Obtainintosh is available under the [MIT License](./LICENSE).
