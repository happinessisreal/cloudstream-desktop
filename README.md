# Cloudstream Desktop for Linux

A working early implementation using **Tauri 2, Svelte 5 and Rust**, with a Kotlin/JVM compatibility worker built around Cloudstream's existing extension library.

**Current compatibility:** cross-platform Cloudstream **JAR extensions** and **data-only original `.cs3` packages** can execute. The desktop bridge translates DEX bytecode, restores external Kotlin JVM method names, and supports the legacy `load(Context)` entry point when Android services are not used. Extensions requiring Android UI, resources, WebView or app-only services still need further compatibility work. This is an independent development project, not an official upstream Linux release.

## Run locally

Requirements: Node.js 22+, Rust, a Java 21 JDK with `jlink`, mpv, and the [Tauri Linux development libraries](https://v2.tauri.app/start/prerequisites/#linux). On Debian/Ubuntu, install the development dependencies listed there and `mpv openjdk-21-jdk`. Choosing a local subtitle uses the desktop file chooser portal, with Zenity as a fallback.

```sh
npm ci
npm run worker:build
npm run desktop
```

Set `JAVA_HOME` to a Java 21 JDK if it is not already configured. The build script recognizes `/usr/lib/jvm/java-21-openjdk` on this development machine. The application includes the generated Java runtime when packaged; users of the package do not need Java separately.

In the app, choose **Try an open movie** to install the bundled Blender Open Movies compatibility sample. Browse a film, choose **Find a stream**, then select the returned stream. Playback opens in mpv. Bookmarks and watch progress are stored locally.

The accompanying **Linux development tarball** can be extracted and launched with `./start-cloudstream`. It contains the app, worker and Java runtime; the host still needs mpv, GTK 3 and WebKitGTK 4.1. The accompanying Debian package is also a development build, not a tested production release.

Choose **Add a repository** to use an existing Cloudstream repository manifest URL or a `cloudstreamrepo://` link. JAR entries use the existing `jarUrl`/`jarHash` fields. CS3 entries are checked by the desktop bridge; Installed shows whether each package actually loaded and explains failures. Repository short codes are not implemented.

`npm run dev` opens a browser-only UI preview. Native features require `npm run desktop`; the browser preview reports that clearly and does not fabricate media or extension results.

## Implemented

- Native Linux Tauri shell with a desktop adaptation of Cloudstream’s black/gray and blue Material theme, Roboto typography, persistent navigation, featured titles, horizontal poster shelves, content filters, shelf expansion, and a persistent poster-density preference.
- Existing repository manifests, relative plugin-list URLs, unknown-field tolerance, repository-scoped extension identities and upstream status values.
- Download size limits, available SHA-256 verification, package format inspection and atomic content-addressed installation.
- Limited original CS3 support through dex2jar, with explicit rejection of Android service dependencies and host API-name resolution for sanitized Kotlin value-class methods.
- Per-extension load diagnostics, visible search submission, and extension filtering even when no media sources are active.
- JVM package loading through `@CloudstreamPlugin` discovery or `manifest.json`, using upstream `BasePlugin`, `MainAPI`, shared provider and extractor registries.
- Provider homepage, paginated search, title details, episodes, stream resolution and subtitle metadata.
- Replacement load failure restores the previous installed package; old package files remain cached.
- Worker health handshake, hard deadlines, restart and extension restoration after worker exit.
- mpv playback, HTTP headers and referer, pause, seek, stop, local bookmarks and periodically saved watch progress. The player can add local SRT, VTT, ASS, SSA, TTML, DFXP and SAMI subtitles, including a selected subtitle inside a ZIP archive, without changing the original file.
- A bundled sample extension using official Blender open-movie trailers. The sample is explicit and opt-in.
- SQLite persistence and a CLI for testing the runtime without the desktop interface.

## Not implemented yet

- **Full Android runtime support**, Waydroid integration and Android settings/resources compatibility. Data-only `.cs3` execution is supported by the limited desktop bridge.
- A working JVM WebView resolver. Upstream marks that implementation as unfinished; WebView-dependent providers are not supported by this build.
- Broad compatibility certification against third-party repositories and older API versions. BdixCircleftp version 27 was loaded from its original installed `.cs3` and returned 40 live search results during development. That does not establish universal extension compatibility.
- Downloads, tracking-service integrations, Chromecast, torrents, DRM and embedded video rendering.
- Universal compatibility with session-aware providers. A loopback media relay carries provider cookies, interceptors and resource-specific headers for tested playback paths; live third-party providers and all manifest formats have not been certified.
- Per-extension process sandboxing. The Java worker is a separate process with a memory limit, and runs downloaded code with the user's permissions. Install extensions from trusted sources.
- Automatic application updates and deep-link OS registration.

## Validation

```sh
npm run check
npm run build
cargo test -p cloudstream-core
bash scripts/smoke-worker.sh
(cd runtimes/jvm && ./gradlew test)
# Optional: test an original package, including its live source search
python3 scripts/smoke-legacy.py /path/to/extension.cs3 --query test
cargo test -p cloudstream-core --test player_linux -- --ignored
npx playwright install chromium
npm run test:ui
```

If Chromium is already installed, use `PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npm run test:ui` instead of downloading it.

The Rust tests cover package classification, format/hash rejection, repository-scoped identities, unchanged installed records after corrupt package preparation, opaque-data persistence and existing repository/JAR metadata. The JVM smoke test performs actual package installation, provider registration, search, details and link resolution. Browser tests use an explicit development-only bridge to verify interface interactions, opaque episode data, playback commands, error recovery, keyboard dialogs, narrow layouts and WCAG A/AA accessibility rules. The test bridge is removed from production builds.

## CLI

```sh
cargo run -p cloudstream-core -- status
cargo run -p cloudstream-core -- sample
cargo run -p cloudstream-core -- providers
cargo run -p cloudstream-core -- smoke
cargo run -p cloudstream-core -- add-repo 'https://example.com/repo.json'
cargo run -p cloudstream-core -- install 'https://example.com/repo.json' 'InternalName'
```

Additional commands: `states` (per-extension runtime diagnostics), `home PROVIDER`, `search PROVIDER QUERY`, `load PROVIDER URL`, `links PROVIDER DATA`, `repositories`. Quote provider IDs and opaque episode data. Set `CLOUDSTREAM_DATA_DIR` and `CLOUDSTREAM_RESOURCE_DIR` to override the CLI's development paths. The desktop uses its platform application-data directory (`dev.cloudstream.desktop`), independently of CLI test data.

## Build a Linux package

```sh
npm run desktop:build
```

This produces a Debian package under `target/release/bundle/deb/`, with the generated JVM worker/runtime and an mpv package dependency. AppImage configuration is included; its distribution and clean-machine behavior still need validation. Release engineering and distribution support are unfinished.

## Layout

```text
src/                 Svelte interface and typed desktop bridge
src-tauri/           Tauri commands, capabilities and bundle configuration
crates/core/         Rust repository manager, storage, worker supervision, player and CLI
runtimes/jvm/        Kotlin compatibility worker and sample extension
protocol/            Versioned worker protocol
scripts/             Runtime build and smoke checks
tests/               Browser interaction and accessibility tests
```

## Upstream and licensing

GPL-3.0; see [LICENSE](LICENSE) and [NOTICE.md](NOTICE.md). The JVM worker uses the library from upstream commit [`5024ace2922a148d38df2a816b483b529789cf58`](https://github.com/recloudstream/cloudstream/tree/5024ace2922a148d38df2a816b483b529789cf58). Its immutable JitPack artifact is selected explicitly because the upstream POM incorrectly identifies that build as `master-SNAPSHOT`; the worker build declares the published runtime dependencies explicitly.

Before upstream contributions, follow [Cloudstream's AI policy](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/AI-POLICY.md), including disclosure of AI assistance and testing before submitting a pull request. No upstream changes or pull requests have been submitted by this project.
