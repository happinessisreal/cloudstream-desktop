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

### [T07] Extension updates, bulk actions and source tester
- **Status:** Completed
- **Priority:** P1
- **Completed Substeps:**
  1. Added typed models (`ExtensionUpdate`, `BulkUpdateResult`, `ProviderTestStageResult`, `ProviderTestReport`) in Rust (`crates/core/src/models.rs`) and TypeScript (`src/lib/types.ts`). Implemented `check_for_updates` in core, matching installed plugins with stored repositories and checking version numbers and changelogs.
  2. Implemented `update_extension` and `update_all_extensions` in core. Sequential bulk updates ensure that individual package download or activation failures do not abort remaining updates, and previous working extension versions are strictly preserved upon failure. Added `clean_interrupted_updates` to safely purge leftover temporary files (`.tmp`) upon application startup.
  3. Implemented `test_provider` self-test suite modeled after Android's `TestViewModel2`: sequentially tests Home, Search, Details, and Links on bounded candidates without unbounded streaming or whole-library traversal. Respects provider capabilities (skips Home if `hasMainPage == false`, skips Links if no candidate episodes), records per-stage duration and status, and supports cancellation.
  4. Exposed `check_for_updates`, `update_extension`, `update_all_extensions`, and `test_provider` via Tauri RPC commands in `src-tauri/src/main.rs`.
  5. Built UI components in `src/App.svelte` and `src/style.css`: added "Check for updates" and "Update all" buttons in Extensions Installed tab with version badges, and added a "Test" button on registered provider rows opening a modal dialog displaying stage-by-stage results, pass/skip/fail badges, millisecond timings, and diagnostic details.
- **Validation:**
  - Rust tests: `cargo test --test updates_and_tester` (3 passed: cleanup, update rollback on failure, and provider self-tester diagnostics).
  - All core tests: `cargo test -p cloudstream-core` (43 passed, 1 ignored).
  - Tauri build: `cargo check -p cloudstream-desktop` (0 errors, 0 warnings).
  - Svelte check: `npm run check` (0 errors, 0 warnings).
  - Playwright UI tests: `npm run test:ui` (16 passed, including tests for update check/bulk updates and provider self-tester modal).
  - Worker smoke: `bash scripts/smoke-worker.sh` (passed).

### [T08] Complete search parity, filters and suggestions
- **Status:** Completed
- **Priority:** P1
- **Completed Substeps:**
  1. Extended protocol and worker with `quickSearch` method support in Kotlin worker (`Main.kt`, `protocol/README.md`) and rebuilt `worker.jar`. Added typed models in Rust (`SearchHistoryEntry`, `SearchResult`, `search_suggestions` field on `AppearanceSettings`) and TypeScript (`SearchHistoryEntry`, `SourceSearchState`, `SearchFilters`).
  2. Implemented SQLite search history CRUD in `Store` (`crates/core/src/store.rs`) scoped by profile ID with exact prefix isolation (`substr(id, 1, length(?1)) = ?1`), tie-breaking order (`ORDER BY updated DESC, rowid DESC`), and ACID durability surviving database re-open.
  3. Implemented `search_provider`, `quick_search_provider`, and `search_suggestions` in `crates/core/src/lib.rs`. Suggestions combine profile search history and provider quick search queries with deduplication, order preservation, and a 10-item cap.
  4. Exposed `quick_search`, `get_search_history`, `add_search_history`, `delete_search_history_entry`, `clear_search_history`, and `search_suggestions` Tauri commands in `src-tauri/src/main.rs`.
  5. Built comprehensive frontend search in `src/App.svelte` and `src/style.css`:
     - Multi-source federated search orchestrating concurrent cancellable queries with request session IDs and stale response discarding.
     - Fault isolation: failing sources display source-scoped error banners with retry buttons without halting healthy source results.
     - Grouped by source vs Merged grid view modes toggleable with responsive grids, preserving items without collapsing titles.
     - Client-side loaded items filters for media type, quality, and dub/audio status.
     - Per-source pagination ("Load more from {providerName}") affecting only the requested source.
     - Search history and debounced (250ms) suggestions dropdown with entry deletion, click-outside dismissal, and Escape shortcut.
     - Search suggestions opt-out toggle in Settings under Appearance, immediately halting network suggestion requests.
- **Validation:**
  - Rust tests: `cargo test --test search_parity` (3 passed: history CRUD/isolation across restart, suggestions combining history and quick search, and pagination/cancellation).
  - All core tests: `cargo test -p cloudstream-core` (46 passed, 1 ignored).
  - Tauri check: `cargo check -p cloudstream-desktop` (0 errors, 0 warnings).
  - Svelte check: `npm run check` (0 errors, 0 warnings).
  - Playwright UI tests: `npm run test:ui` (18 passed, including federated search error isolation, view modes, per-source pagination, search history, and settings opt-out).
  - Worker smoke: `bash scripts/smoke-worker.sh` (passed).

### [T10] Local library statuses, favorites and watched episodes
- **Status:** Completed
- **Priority:** P1
- **Completed Substeps:**
  1. Enriched `LibraryItem` and added `WatchStatus` enum (`watching`, `completed`, `on_hold`, `dropped`, `plan_to_watch`, `none`) with serde snake_case mapping, retaining `is_favorite` and `is_subscribed` as independent attributes. Preserved legacy bookmarks as saved entries with default `none` status.
  2. Transactional SQLite Migration 4 added `watched_override` column to `watch_progress` table. Updated `save_watch_progress` to preserve manual watched overrides on playback resume updates.
  3. Added core store methods and Tauri commands:
     - `set_library_status`, `set_library_favorite`, `set_library_subscribed`, `remove_library_item`, `clear_library` (with optional status-scoped clear that preserves favorited/subscribed items).
     - `remove_history_item`, `clear_history`.
     - `get_media_watch_progress`, `mark_episode_watched`, `mark_episodes_up_to` (filtering by dub group and sorting deterministically by season, episode, index).
  4. Built rich UI in `src/App.svelte` and `src/style.css`:
     - Library tabs: All, Watching, Plan to watch, Completed, On hold, Dropped, Favorites, Subscribed.
     - Local title search filter and persistent sort controls (Recently updated, A-Z, Z-A, Release date, Score).
     - Per-card status badges, favorite/subscribed icon indicators, and single-item remove button on hover.
     - History section with per-item remove button and "Clear all" confirmation modal.
     - Confirmation modals for clearing library (with tab-specific vs all options) and clearing watch history.
     - Media details controls: Watch status dropdown, favorite toggle button, subscribe toggle button.
     - Episode list controls: Mark watched/unwatched toggle button and "Mark up to here" button in deterministic order.
- **Validation:**
  - Rust tests: `cargo test --test library_and_statuses` (5 passed: status/fav/sub coexistence, legacy bookmarks preservation, watched overrides surviving resume updates, deterministic mark up to dub group ordering, and single removal & bulk clear).
  - All core tests: `cargo test -p cloudstream-core` (51 passed, 1 ignored).
  - Tauri build: `cargo check -p cloudstream-desktop` (0 errors, 0 warnings).
  - Svelte check: `npm run check` (0 errors, 0 warnings).
  - Production frontend build: `npm run build` (0 errors, 0 warnings).
  - Playwright UI tests: `npm run test:ui` (19 passed, including new test for library statuses, favorites, tab filtering, episode watched toggles, and bulk clear).
  - Worker smoke: `bash scripts/smoke-worker.sh` (passed).

### [T09] Home parity, category fidelity and caching
- **Status:** Completed
- **Priority:** P1
- **Completed Substeps:**
  1. Updated worker `executeGetMainPage` in `Main.kt` to preserve `sectionIndex`, `listIndex`, `rowId`, `page`, and each list's `orientation` (`horizontal` / `vertical`) and `horizontalImages` without flattening multiple `HomePageList`s into a single requested category. Preserved section cursors and row page cursors independently. Enhanced `BlenderFilms` sample provider with multi-row fixtures (Section 0 with horizontal open movies and Section 1 with two distinct vertical lists).
  2. Implemented bounded TTL cache (10 minutes, 50-entry cap with LRU eviction) in `App` (`crates/core/src/lib.rs`) keyed by provider ID, package hash, section, page, and language. Added `force_refresh` support, source cache isolation, and cache invalidation on provider enable/disable, clone changes, and extension installations/updates/removals. Unresolved errors are never cached as empty success.
  3. Added `selected_provider` persistence to `ProfileSettings` in `crates/core/src/models.rs`, surviving application restarts.
  4. Exposed updated `provider_home` with `force_refresh` parameter in Tauri commands (`src-tauri/src/main.rs`).
  5. Built rich UI features in `src/App.svelte`, `src/lib/MediaRail.svelte`, `src/lib/Spotlight.svelte`, and `src/style.css`:
     - Supported list orientation in `MediaRail` and `MediaCard` (horizontal 16:9 banner cards vs 2:3 vertical poster cards).
     - Spotlight background enrichment: asynchronously fetches `load_details` in background to populate synopsis plot, genres/tags, rating score, and high-res backdrop art without blocking rail rendering.
     - Search isolation: opening search or submitting queries immediately aborts background spotlight detail resolution.
     - Source selector parity: added "No source selected" option, "Random source" action, "Random title" action, and force-refresh button.
     - Personalized library rails on Home: Continue Watching rail (with per-item removal and clear all confirmation), Plan to Watch rail, and Favorites rail that reactively update on library/status changes.
- **Validation:**
  - Rust tests: `cargo test --test home_and_caching` (5 passed: multi-row provider retaining named lists without flattening, caching and source isolation, cache invalidation on source/extension mutations, unresolved errors never cached, and selected provider surviving restart).
  - All core tests: `cargo test -p cloudstream-core` (56 passed, 1 ignored).
  - Tauri build: `cargo check -p cloudstream-desktop` (0 errors, 0 warnings).
  - Svelte check: `npm run check` (0 errors, 0 warnings).
  - Production frontend build: `npm run build` (0 errors, 0 warnings).
  - Playwright UI tests: `npm run test:ui` (20 passed, including test for multi-row provider, category expansion, random actions, and library rails).
  - Worker smoke: `bash scripts/smoke-worker.sh` (passed).

## Next Ready Tasks
- **[T11]** Local profiles, startup selection and access locks (Depends on: T01, T02, T10)
- **[T12]** Rich details and episode browser (Depends on: T03, T10)
- **[T14]** Authoritative mpv session state and events (Depends on: T02, T03)
