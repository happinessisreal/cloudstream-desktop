# Cloudstream Desktop Parity Implementation Progress

Audit Baseline: 2026-11-01 (Upstream reference commit: 5024ace2922a148d38df2a816b483b529789cf58)

## Completed Tasks

### [T01] Versioned storage, settings service and view state
- **Status:** Completed
- **Priority:** P0
- **Completed Substeps:**
  1. Transactional SQLite migration runner using `PRAGMA user_version` (migrated from v1 to v2 with ordered steps, index creation, and automatic rollback on failure). Copyable fixture `crates/core/tests/fixtures/v1_database.db` embedded and verified.
  2. Typed Settings data structures (`Settings`, `GlobalSettings`, `AppearanceSettings`, `ProfileSettings`, `PosterDensity`) with `schemaVersion`, `revision`, range validation, absent-field defaults, and `extra` JSON preserving unknown future fields across restarts and updates. Native Tauri commands `get_settings` and `update_settings` added and wired to `snapshot`.
  3. UI poster density moved to Settings with one-time migration from `localStorage`, preserving user selection. Extracted typed UI error helper `toErrorMessage`.
- **Validation:**
  - `cargo test`: 13 passed, 1 ignored (compatibility suite: 8, migrations suite: 5).
  - `npm run check`: 0 errors, 0 warnings.
  - `npm run test:ui`: 10 passed (including settings persistence and UI density tests).

### [T02] Stable media/episode identity and progress migration
- **Status:** Completed
- **Priority:** P0
- **Completed Substeps:**
  1. Introduced stable `media_id` (`compute_media_id`) and `episode_id` (`compute_episode_id`) with alias mapping (`aliases` table) and fresh payload cache (`episode_payloads` table). Scoped media by `provider_id` and `unique_url` or URL, preserving installed package and provider identities.
  2. Episode identity derived from provider episode ID when present, else parent `media_id` + dub group (`DubStatus`) + `season` + `episode`. Handled movie synthetic episode stability and disambiguated unnumbered list episodes by index/name. Decoupled token-bearing payloads from primary identity and preserved legacy data-hash aliases.
  3. Implemented transactional Migration 3 in SQLite: upgraded existing `library` and `history` records, migrated playback seconds and converted legacy timestamps to epoch milliseconds, populated `aliases`, `episode_payloads`, and `watch_progress` without losing history or bookmarks.
- **Validation:**
  - `cargo test`: 19 passed, 1 ignored (identity migration suite: 6, migrations suite: 5, compatibility suite: 8).
  - `npm run check`: 0 errors, 0 warnings.
  - `npm run test:ui`: 10 passed.

### [T03] Complete typed provider/media/stream bridge
- **Status:** Completed
- **Priority:** P0
- **Completed Substeps:**
  1. Implemented protocol version 2 negotiation and contract additions in Kotlin mapping, Rust serde models, and TypeScript types. Preserved null vs absent semantics and unknown type strings across boundaries via open `extra` map containers. Added provider capabilities (`hasQuickSearch`, `hasDownloadSupport`, `hasChromecastSupport`, `sequentialMainPage`, `vpnStatus`, `providerType`, `supportedSyncNames`) and granular timeouts (`searchTimeoutMs`, `quickSearchTimeoutMs`, `getMainPageTimeoutMs`, `loadTimeoutMs`, `loadLinksTimeoutMs`).
  2. Enhanced search item mapping with `score`, `quality`, `year`, `otherName`, `episodesCount`, `dubEpisodes`, and `dubStatus` (`none`, `dubbed`, `subbed`, `both`). Upstream homepage response mapping preserves multiple shelves with individual `rowId`, `horizontalImages`, and `orientation` (`horizontal` / `vertical`).
  3. Added rich details: `plot`, `tags`, `duration`, `uniqueUrl`, `backgroundPosterUrl`, `logoUrl`, `contentRating`, `comingSoon`, `syncData` (e.g. TMDB, IMDb, MAL), `trailers`, `actors` (with actor/role/voiceActor metadata), `recommendations`, structured `dubGroups`, and `seasons`. Episode names avoid synthetic translated prefix mangling when dub status is modeled explicitly.
  4. Implemented discriminated streams by `kind` (`direct`, `hls`, `dash`, `torrent`, `magnet`, `playlist`, `drm`) retaining `extractorData`, `source`, separate `audioTracks` with headers, playlist items, DRM credentials (`uuid`, `kid`, `key`, `kty`, `licenseUrl`, `keyRequestParameters`), and subtitle metadata (`langTag`, `headers`).
  5. Enriched compatibility sample provider `BlenderFilms` to return rich fields across all methods. Added comprehensive Kotlin mapper unit tests and Rust contract deserialization fixtures for enriched payloads, backward compatibility with v1 JSON, and unknown field round-trips. Updated protocol documentation in `protocol/README.md`.
- **Validation:**
  - Kotlin worker tests: `cd runtimes/jvm && ./gradlew test` (passed).
  - Rust tests: `cargo test -p cloudstream-core` (25 passed, 1 ignored).
  - Worker smoke: `bash scripts/smoke-worker.sh` (passed, verified full JSON serialization).
  - Svelte check: `npm run check` (0 errors, 0 warnings).
  - Playwright UI tests: `npm run test:ui` (10 passed).

### [T04] Request scheduling, cancellation and incremental results
- **Status:** Completed
- **Priority:** P0
- **Completed Substeps:**
  1. Separated worker input reading from job execution in Kotlin worker (`Main.kt`). Reads lines continuously without blocking the stdin reader thread. Jobs execute concurrently inside coroutines launched on a dedicated `workerScope` and write framed responses using a synchronized stdout writer. Added `activeJobs` and `jobPluginMap` tracking with support for out-of-order execution and structured JSON-RPC errors (`-32800` Request cancelled, `-32700` Parse error, `-32600` Invalid request, `-32000` Provider execution/timeout error).
  2. Implemented `cancel` RPC method in Kotlin worker to abort active coroutines immediately upon request. Added mutex-protected extension lifecycle (`registryMutex`) for `loadPlugin` and `unloadPlugin`. Unloading an extension cancels active jobs running on its providers before closing the classloader. Enforced per-provider mutex serialization and `sequentialMainPageDelay` throttling for `sequentialMainPage = true` sources without blocking other providers. Added capped provider deadlines (`coerceIn(1_000L, 60_000L)`).
  3. Re-architected Rust worker (`worker.rs`) to multiplex requests asynchronously. Background task reads stdout, demultiplexes out-of-order responses by request ID, forwards `progress` events (such as real-time stream and subtitle discoveries), and safely tolerates malformed stdout lines. Worker crashes or unresponsiveness trigger immediate fail-fast of pending calls once with automatic session recreation and extension restoration on subsequent calls. Bounded in-flight concurrency with a semaphore.
  4. Exposed cancellation support through the entire stack: `App::call_cancellable` and `App::cancel_request` in Rust core, `cancel_request` command in Tauri 2, `CallOptions` with `AbortSignal` and client `requestId` in typed UI API (`src/lib/api.ts`). Integrated `AbortController` in Svelte UI (`src/App.svelte`) to abort abandoned search and link resolution jobs on new queries, navigation, and component unmount.
  5. Updated protocol documentation in `protocol/README.md`.
- **Validation:**
  - Kotlin tests: `cd runtimes/jvm && ./gradlew test` (10 passed, testing protocol handshake, cancellation, and malformed stdout tolerance).
  - Rust tests: `cargo test -p cloudstream-core` (31 passed, 1 ignored; including 5 tests in new suite `tests/worker_scheduling.rs` testing out-of-order responses, hung request cancellation, crash recovery, malformed stdout tolerance, and progress events).
  - Worker smoke: `bash scripts/smoke-worker.sh` (passed).
  - Tauri build: `cargo check -p cloudstream-desktop` (0 errors, 0 warnings).
  - Svelte check: `npm run check` (0 errors, 0 warnings).
  - Playwright UI tests: `npm run test:ui` (11 passed, including new test for in-flight search cancellation and navigation cleanup).

### [T05] Preserve and measure extension compatibility
- **Status:** Completed
- **Priority:** P0
- **Completed Substeps:**
  1. Implemented pre-flight package inspection (`inspectPackage`) in `LegacyDex.kt` and `Main.kt`. Extracts package format (`dex` vs `jar`), `apiVersion`, `entryClass`, `requiresResources`, `unavailableApis`, `actionableFailure`, `pluginType` (`provider`, `extractor`, `importer`, `empty`), and SHA-256 without loading. Short-circuits cleanly on fatal Android resource requirements before DEX-to-JAR conversion.
  2. Preserved content-addressed translation cache (`$hash.desktop-dex-v2.jar`) and `restoreJvmMethodNames` bytecode transformer. Added atomic registration rollback on extension load failure: worker tracks previously registered providers and extractors and reverts partial registrations if plugin initialization fails.
  3. Added `ExtensionState` and `PluginCapabilityReport` to Rust models (`models.rs`) and UI types (`src/lib/types.ts`). Enriched worker `getPluginStates` and exposed `"inspectPackage"` via worker API.
  4. Created lawful compatibility test fixtures in `crates/core/tests/fixtures/compatibility_manifest.json` recording 7 lawful fixtures and operations. Added comprehensive tests in `crates/core/tests/compatibility.rs` and `LegacyDexTest.kt` verifying capability extraction, resource rejection, package format handling, and atomic rollback.
  5. Updated `scripts/smoke-legacy.py` with `--inspect` flag and resilient stdout parsing. Updated `protocol/README.md`.
- **Validation:**
  - Kotlin worker tests: `cd runtimes/jvm && ./gradlew test` (passed, 12 tests in `LegacyDexTest`).
  - Rust tests: `cargo test -p cloudstream-core` (34 passed, 1 ignored; 11 passed in `compatibility.rs`).
  - Tauri build: `cargo check -p cloudstream-desktop` (0 errors, 0 warnings).
  - Svelte check: `npm run check` (0 errors, 0 warnings).
  - Playwright UI tests: `npm run test:ui` (11 passed).
  - Worker smoke: `bash scripts/smoke-worker.sh` (passed).
  - Legacy smoke: `python3 scripts/smoke-legacy.py runtimes/jvm/sample/build/libs/desktop-sample.jar --inspect` (passed).

### [T06] Repository and extension management parity
- **Status:** Completed
- **Priority:** P1
- **Completed Substeps:**
  1. Implemented repository short code resolution (`resolve_repository_url`, `resolve_shortcode_redirect` in `repository.rs`) supporting `!code` (resolving via `py.md`), `cutt.ly/{code}` short codes, and `cloudstreamrepo://` schemes with bounded redirects (max 5) and timeouts (10s), returning actionable diagnostics if external services are unreachable while keeping ordinary URLs completely offline-safe.
  2. Implemented repository refresh, duplicate handling (refreshes repository rather than failing), custom display name / rename metadata (`rename_repository`), and share link copying (`https://cs.repo/?{url}`). Removing a repository unloads its plugins but strictly preserves user library bookmarks, watch history, and progress in SQLite.
  3. Added local package import (`prepare_local`, `install_local` in `repository.rs` and `lib.rs`) for standalone `.jar` and `.cs3` files. Routes through the exact same bounded inspection (`inspect_package`), SHA-256 verification, and atomic storage under `local://packages`, ensuring independent identity even if local and remote extensions share an `internalName`.
  4. Added provider enable/disable preferences (`set_provider_disabled`, `list_disabled_providers`) and clone-site configuration (`ProviderCloneConfig`, `save_provider_clone`, `delete_provider_clone`). Clones create separate stable identities and stored URL handles without globally mutating the original source.
  5. Implemented catalog filtering in the UI (`src/App.svelte` and `src/style.css`): filter by name, author, language, extension type (all/providers/extractors), and status (all/installed/updates) that works seamlessly even with zero loaded providers. Added rich card details, provider capability badges, enable/disable toggles, and modal dialogues.
- **Validation:**
  - Rust tests: `cargo test -p cloudstream-core` (40 passed, 1 ignored; including 6 new tests in `tests/repository_management.rs`).
  - Tauri build: `cargo check -p cloudstream-desktop` (0 errors, 0 warnings).
  - Svelte check: `npm run check` (0 errors, 0 warnings).
  - Playwright UI tests: `npm run test:ui` (14 passed, including tests for catalog filtering with zero loaded providers, repository rename, share link, local package installation, and provider enable/disable toggle).
  - Worker smoke: `bash scripts/smoke-worker.sh` (passed).
  - Kotlin worker tests: `cd runtimes/jvm && ./gradlew test` (passed).

## Next Ready Tasks
- **[T07]** Extension updates, bulk actions and source tester (Depends on: T04, T05, T06)
