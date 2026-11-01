# Cloudstream Android → Linux desktop implementation handoff

Prepared 2026-11-01. Target: the existing **Rust + Tauri 2 + Svelte 5 Linux application**, preserving usable existing Cloudstream extensions and the approved desktop redesign.

## How to use this handoff

Give the implementing agent this file, `CLOUDSTREAM_PARITY_TASKS.json`, `CLOUDSTREAM_IMPLEMENTER_PROMPT.md`, and the project source directory containing this `handoff/` folder. The JSON contains the same feature IDs and task dependencies for progress tracking. Implement **one ready work package at a time**, using its numbered substeps as separate patches when necessary. Start with **T01**, then **T02–T05**. Do not start with casting, DRM or an Android runtime.

This is a source-based feature audit and an implementation specification. It does not claim that every provider or external service currently works. “Android has” below means reachable implementation or an explicitly identified API capability in the pinned source. Android-only behavior is mapped to a Linux equivalent; dormant definitions and conditional capabilities are identified separately. App code was not changed by this audit.

## Scope, evidence and baseline

Android reference: [`recloudstream/cloudstream` at `5024ace2922a148d38df2a816b483b529789cf58`](https://github.com/recloudstream/cloudstream/tree/5024ace2922a148d38df2a816b483b529789cf58). Local checkout: `../reference/cloudstream-android` relative to this handoff folder. The desktop worker uses this same upstream library revision; do not silently advance it while adding features.

The audit enumerated **268 app Kotlin files**, **148 common library Kotlin files**, the active mobile navigation graph, manifest entry points, current Compose settings, shared preferences, and the older XML preferences. Old fragments and new `*2` Compose wrappers coexist: for example `SettingsFragment2` delegates to `SettingsFragmentScreen`. Treat the active graph and current screen handlers as the primary UI evidence, rather than counting duplicate implementations as additional features.

Paths in this document use these roots:

| Prefix | Meaning |
|---|---|
| `D/` | `../` relative to this document (the project root); commands run there |
| `A/` | upstream `app/src/main/java/com/lagradost/cloudstream3/` |
| `L/` | upstream `library/src/commonMain/kotlin/com/lagradost/cloudstream3/` |
| `S/` | upstream `shared/src/commonMain/kotlin/com/lagradost/cloudstream4/` |

The source index at the end links every Android evidence ID to immutable upstream files, so this handoff also works on another computer. Clone the upstream repo and check out the exact revision if the local reference checkout is unavailable. Do not replace the existing desktop checkout with upstream Android code.

### Current desktop architecture and verified capabilities

| Layer | Existing files | What is actually implemented |
|---|---|---|
| UI | `D/src/App.svelte`, `D/src/lib/{types.ts,api.ts,MediaCard.svelte,MediaRail.svelte,Spotlight.svelte}`, `D/src/style.css` | Home, Search, Library, Extensions, Settings; source selection; loaded-item media filters; spotlight and rails; expansion/pagination; dialogs; basic player bar; poster density persisted in localStorage. |
| Native bridge | `D/src-tauri/src/main.rs` | Snapshot, repository/package operations, providers/home/search/details/links, bookmark/remove bookmark, play and basic player actions. |
| Rust services | `D/crates/core/src/{lib.rs,models.rs,store.rs,repository.rs,worker.rs,player.rs}` | SQLite JSON records; repository parsing, bounded downloads, optional hash checking, content-addressed package install; worker supervision; external mpv and periodic watch progress. |
| Compatibility worker | `D/runtimes/jvm/src/main/kotlin/dev/cloudstream/worker/{Main.kt,LegacyDex.kt}` | JSON-lines RPC, JAR plugins and a limited DEX-to-JVM bridge for original data-only CS3, shared provider/extractor registries, search/home/load/links and load diagnostics. |
| Distribution | `D/scripts/build-worker.sh`, `D/src-tauri/tauri.conf.json` | Java 21 runtime bundled, development Debian/portable packages; host mpv and GTK/WebKitGTK remain required. |
| Validation | `D/crates/core/tests/`, `D/runtimes/jvm/src/test/`, `D/tests/desktop.spec.ts`, `D/scripts/smoke-worker.sh`, `D/scripts/smoke-legacy.py` | Package/storage compatibility fixtures; Kotlin ABI tests; browser behavior/accessibility fixtures; native mpv integration; live legacy-provider smoke tooling. |

Known desktop limitations that affect almost every task:

- `App.svelte` is a large shared-state component. Extract a service/store or view only when needed for the task; preserve the accepted blue/black desktop layout and existing interactions.
- Store schema is `records(kind,id,value,updated)` with `PRAGMA user_version=1`. There is no ordered migration system. Settings beyond poster density are not implemented.
- `WatchEntry.id` currently depends on **opaque episode data**. Expiring provider tokens can create duplicate history or lose resume state. Fix identity before implementing statuses, downloads or sync.
- `MediaDetails`, `Episode`, `Stream`, `Subtitle` and `Provider` lose upstream fields. Anime dub groups are flattened into episode names. Search mapping always sets `year` to null. Homepage mapping flattens upstream response lists under a requested category and loses individual row identities/orientation.
- Worker requests are sequential. Soft deadlines are not independent cancellation; hard timeout discards/restarts the worker. A search can wait behind another provider operation. Do not implement federated search by merely putting `Promise.all` around the current command.
- Runtime format and execution mode differ: stored `RuntimeKind::Android` identifies a DEX package, while a working limited bridge reports `dex_jvm`. It does not imply Android services are available.
- mpv runs in its own native window, with position/duration polling every five seconds. Stream headers and referer are passed, but session cookies, subtitle-specific headers, separate audio, extractor verifier jobs and video interceptors are not connected. UI pause state is optimistic rather than authoritative.
- No downloads, tracking accounts, profiles, casting, full Android backend, working JVM WebView resolver, torrent integration or DRM playback currently exist.

### Verification evidence

During this audit, `cargo test -p cloudstream-core` passed **8 compatibility tests**, with the display-dependent native mpv test explicitly **ignored by default**. `npm run check` also passed with **0 errors and 0 warnings**. Earlier development validation recorded 10 browser interaction/accessibility tests, 3 Kotlin tests, the native mpv integration and worker smoke tests passing. Those earlier results are baseline evidence, not a promise that future changes are tested.

One unchanged **BdixCircleftp v27 CS3** was previously loaded and returned **40 live search results**; its home and episode details also loaded. This is a network-dependent example, not universal extension certification. `MegaProvider` is a repository importer, not a searchable provider, and the current bridge rejects its Android repository-manager dependency. Keep these two cases distinct.

## Rules for the implementing agent

1. Preserve Tauri, Rust, Svelte and the compatibility worker. Do not rewrite plugins in Rust or require extension authors to port existing CS3 just to satisfy this project.
2. Preserve repository-scoped package IDs and provider IDs. Never identify a source by its display name alone. Do not treat same-named titles from different sources as identical without explicit metadata matching.
3. **Provider URL and episode `data` are opaque.** Round-trip them byte-for-byte as strings. Never decode/rebuild JSON, construct an episode URL from a title, strip query strings, lowercase URLs or replace data with a guessed URL.
4. Keep library/progress/settings data during upgrades. Ship ordered transactional migrations and fixture tests for every persistent shape change. Never “fix” a migration by clearing the database.
5. Add fields with defaults through all boundaries together: Kotlin → protocol → Rust → Tauri → TypeScript → UI → persisted data. Test old payloads. Version incompatible protocol changes and negotiate capabilities.
6. Preserve install verification, bounded downloads, atomic replacement, rollback on load failure, classloader cleanup, registry cleanup, worker restart restoration, and original CS3 hash/cache behavior.
7. A compatibility rejection must say what is unsupported. Do not catch an exception and pretend the source has no titles, advertise full Android support, or silently remove compatibility guards.
8. Shared extractors may be registered by one plugin and used by another. Avoid arbitrary per-plugin workers without an explicit dependency/registry strategy. Downloaded code currently has the user's permissions; the JVM process is not a security sandbox.
9. Use actual service results and deterministic local fixtures. Keep `window.__CLOUDSTREAM_TEST__` development-only; do not add production mock posters, fake account connection, fake download progress or empty-success error handling.
10. Implement real error/loading/empty states, keyboard navigation and cancel/retry behavior for each visible action. Keep secrets out of logs and exported diagnostics. Use OS credential storage for new service tokens; profile PIN is an application access gate, not data encryption.
11. Use the source as behavior evidence, not code to copy blindly: Android has obsolete XML keys, TODO platform implementations and capabilities unsupported on Linux. Source services/endpoints may have changed; verify the relevant official integration documentation when implementing an adapter.
12. Do not publish, submit PRs or update app packages as part of a small feature patch unless asked. GPL notices and dependency notices must stay intact. For an upstream PR, read and follow its [AI policy](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/AI-POLICY.md).

## Android feature inventory and parity matrix

**Status refers to the current desktop:** Present = baseline implementation exists within its stated limits; Partial = a subset exists; Missing = no implementation found; Adapt = use a Linux equivalent; Conditional = provider/runtime/device/service support is required; Platform-specific = do not mechanically port Android APIs. Each row identifies tasks that close the gap. Features inside a row are related user behavior, not a numerical parity score.

### Browse

| ID | Android capability / desktop gap | Desktop state | Work | Evidence |
|---|---|---|---|---|
| F001 | Provider home, featured poster, horizontal rails and expandable shelves | Partial | [T09](#t09) | [A02], [A05] |
| F002 | Homepage category pagination, row pagination, sequential delays and cache | Partial | [T04](#t04), [T09](#t09) | [A02], [A05] |
| F003 | Remember selected source; random source/title; no-source home mode | Partial | [T09](#t09) | [A02], [A28] |
| F004 | Continue watching with episode progress; remove individual/all entries | Partial | [T02](#t02), [T12](#t12), [T15](#t15) | [A02], [A17] |
| F005 | Home bookmark/watch-status selection and profile switching | Partial | [T10](#t10), [T11](#t11), [T09](#t09) | [A02], [A15], [A16] |
| F006 | Poster title/score/quality/dub/sub/episode badges and display toggles | Partial | [T03](#t03), [T30](#t30) | [A05], [A28] |
| F007 | Full media taxonomy, not just Movie/TV/Anime filters | Partial | [T03](#t03), [T08](#t08), [T30](#t30) | [A05], [A26] |

### Search

| ID | Android capability / desktop gap | Desktop state | Work | Evidence |
|---|---|---|---|---|
| F008 | Search across multiple selected sources with partial results and errors | Missing | [T04](#t04), [T08](#t08) | [A03] |
| F009 | Per-source next pages; grouped/merged search layouts | Partial | [T08](#t08) | [A03] |
| F010 | Search history, delete/clear history, profile isolation | Missing | [T08](#t08), [T11](#t11) | [A03], [A17] |
| F011 | Debounced remote suggestions, opt-out and cancellation | Missing | [T08](#t08) | [A03], [A28] |
| F012 | Quick search, metadata search and source fallback | Missing | [T03](#t03), [T08](#t08), [T24](#t24) | [A03], [A05], [A18] |
| F013 | Source language/type/sub-dub/quality filters and preferred sources | Partial | [T08](#t08), [T30](#t30) | [A03], [A26], [A28] |

### Details

| ID | Android capability / desktop gap | Desktop state | Work | Evidence |
|---|---|---|---|---|
| F014 | Plot, tags, poster, year and duration | Partial | [T03](#t03), [T12](#t12) | [A04], [A05] |
| F015 | Backdrop/logo, score, content rating, status, coming soon and next airing | Missing | [T03](#t03), [T12](#t12) | [A04], [A05] |
| F016 | Actors, roles, related/recommended titles and provider/metadata links | Missing | [T03](#t03), [T12](#t12), [T24](#t24) | [A04], [A37] |
| F017 | Trailers with preference to disable | Missing | [T12](#t12) | [A04], [A28] |
| F018 | Anime dub/sub groups, seasons, ranges, episode sorting and selection | Partial | [T03](#t03), [T12](#t12) | [A04], [A05] |
| F019 | Episode image, synopsis, score, air date, runtime and filler markers | Partial | [T03](#t03), [T12](#t12), [T24](#t24) | [A04], [A05] |
| F020 | Mark watched/unwatched and mark watched through selected episode | Missing | [T10](#t10), [T12](#t12) | [A04], [A17] |
| F021 | Episode action menu: play, mirror, download, subtitles, reload and cast | Partial | [T12](#t12), [T15](#t15), [T20](#t20), [T22](#t22), [T28](#t28) | [A04], [A32] |
| F022 | Bookmark status, separate favorite, subscription and tracker controls | Partial | [T10](#t10), [T13](#t13), [T23](#t23) | [A04], [A15], [A19] |
| F023 | Copy/share title, open original source and search another source | Missing | [T12](#t12), [T27](#t27) | [A04], [A03] |

### Library

| ID | Android capability / desktop gap | Desktop state | Work | Evidence |
|---|---|---|---|---|
| F024 | Watching, completed, on hold, dropped and plan to watch lists | Missing | [T10](#t10) | [A15] |
| F025 | Favorites and subscribed shows separate from watch status | Missing | [T10](#t10), [T13](#t13) | [A15], [A19] |
| F026 | Local library search, list tabs and supported sort choices | Partial | [T10](#t10) | [A15] |
| F027 | Remote tracking libraries; switch service, refresh and search | Missing | [T23](#t23) | [A15], [A18] |
| F028 | Local profiles, profile creation/edit/delete/avatar and startup selector | Missing | [T11](#t11) | [A16], [A17] |
| F029 | Profile PIN and biometric gate | Missing / adapt | [T11](#t11) | [A16], [A30], [A36] |
| F030 | New-episode subscription checks and notifications | Missing / adapt | [T13](#t13) | [A19] |

### Playback

| ID | Android capability / desktop gap | Desktop state | Work | Evidence |
|---|---|---|---|---|
| F031 | Resolve mirrors and select source, quality and language preferences | Partial | [T03](#t03), [T16](#t16) | [A07], [A10] |
| F032 | Play/pause, seek, fullscreen, position/duration and resume persistence | Partial | [T02](#t02), [T14](#t14), [T15](#t15) | [A07], [A08] |
| F033 | Volume/mute, speed, aspect/resize, video/audio delay and rotation | Missing / adapt | [T16](#t16) | [A08], [A29] |
| F034 | External and embedded audio/subtitle track selection | Partial | [T03](#t03), [T17](#t17) | [A06], [A07] |
| F035 | Next/previous episodes, automatic next, replay, start paused and failover | Missing | [T14](#t14), [T15](#t15), [T16](#t16) | [A07], [A29] |
| F036 | Subtitle local file, online search, preferred language and downloads | Partial | [T17](#t17), [T22](#t22) | [A07], [A09], [A18] |
| F037 | Subtitle styling/font/outline/background/alignment/delay and text cleanup | Missing | [T17](#t17) | [A09] |
| F038 | Skip intro/outro/chapter controls and skip-service timestamps | Missing | [T18](#t18) | [A07], [A11] |
| F039 | Seekbar thumbnail preview and player metadata overlay | Missing | [T18](#t18) | [A08], [A28], [A29] |
| F040 | Source/quality priority profiles with automatic selection | Missing | [T16](#t16) | [A10] |
| F041 | Audio compressor and decoding/cache/buffer settings | Missing | [T16](#t16), [T18](#t18) | [A08], [A29] |
| F042 | Control lock, gestures, double click/tap seek and long press speed | Missing / adapt | [T18](#t18), [T30](#t30) | [A29], [A36] |
| F043 | Picture in picture and background media controls | Missing / adapt | [T18](#t18), [T27](#t27) | [A07], [A36] |
| F044 | Custom request headers, referer and session cookies for media resources | Partial | [T05](#t05), [T19](#t19) | [A05], [A06], [A08] |
| F045 | Extractor verification job and per-video network interceptor | Missing | [T03](#t03), [T19](#t19) | [A05], [A07] |
| F046 | Playlists and separate audio resources in stream model | Missing | [T03](#t03), [T16](#t16), [T17](#t17) | [A06] |
| F047 | DRM-aware streams and supported license/key configuration | Missing / conditional | [T29](#t29) | [A06], [A08] |
| F048 | Live streams, live-edge/seek-window behavior | Partial | [T29](#t29) | [A05], [A34] |
| F049 | Torrent/magnet playback and download-progress integration | Missing / conditional | [T29](#t29) | [A05], [A34] |
| F050 | Direct URLs and external play/share/file entry points | Partial | [T27](#t27) | [A01], [A32] |
| F051 | Preferred external player, browser, clipboard, manifest and other actions | Partial / adapt | [T27](#t27) | [A32] |

### Downloads

| ID | Android capability / desktop gap | Desktop state | Work | Evidence |
|---|---|---|---|---|
| F052 | Single film/episode, selected mirror and episode range/all downloads | Missing | [T20](#t20), [T21](#t21) | [A04], [A12] |
| F053 | Persistent queue: pause/resume/cancel/retry/reorder/clear | Missing | [T20](#t20) | [A13] |
| F054 | HTTP range transfers, reconnect and bounded parallel connections | Missing | [T20](#t20) | [A12] |
| F055 | HLS playlist/segment downloads and subtitle companions | Missing | [T21](#t21) | [A12] |
| F056 | Download directory picker, filenames, disk use and deletion | Missing / adapt | [T20](#t20), [T21](#t21) | [A12], [A27] |
| F057 | Offline title/episode catalog, local playback and offline progress | Missing | [T21](#t21) | [A13], [A14] |
| F058 | Download notification/progress, restart recovery and queue service | Missing / adapt | [T20](#t20), [T27](#t27) | [A13], [A01] |

### Accounts

| ID | Android capability / desktop gap | Desktop state | Work | Evidence |
|---|---|---|---|---|
| F059 | AniList tracking/account/library integration | Missing | [T23](#t23) | [A18] |
| F060 | MyAnimeList tracking/account/library integration | Missing | [T23](#t23) | [A18] |
| F061 | Kitsu tracking/account/library integration | Missing | [T23](#t23) | [A18] |
| F062 | Simkl tracking/account/library integration | Missing | [T23](#t23) | [A18] |
| F063 | Progress/status/score sync, metadata matching and manual correction | Missing | [T23](#t23), [T24](#t24) | [A04], [A18] |
| F064 | OpenSubtitles, Addic7ed, Subdl and SubSource subtitle providers | Missing | [T22](#t22) | [A18] |
| F065 | Multiple service accounts per local profile and logout | Missing | [T11](#t11), [T22](#t22), [T23](#t23) | [A18] |
| F066 | AnimeSkip authentication and AniSkip/intro database lookups | Missing | [T18](#t18), [T23](#t23) | [A11], [A18] |
| F067 | Metadata providers: TMDB, cross-TMDB, Trakt, MyDramaList and ID mapping | Missing / conditional | [T24](#t24) | [A05], [A37] |

### Extensions

| ID | Android capability / desktop gap | Desktop state | Work | Evidence |
|---|---|---|---|---|
| F068 | Existing repositories, manifest lists and install/remove extensions | Present, limited runtime | [T06](#t06), [T07](#t07) | [A21] |
| F069 | URL/deep-link/share/short-code repository entry and repository metadata | Partial | [T06](#t06), [T27](#t27) | [A01], [A21], [A22] |
| F070 | Extension search, language/type/status/installed filters and bulk actions | Partial | [T06](#t06), [T07](#t07) | [A22] |
| F071 | Extension description, author, icon, size, version and website; votes exist in legacy detail code | Partial | [T06](#t06) | [A22] |
| F072 | Manual/automatic plugin updates, local packages and load diagnostics | Partial | [T06](#t06), [T07](#t07), [T32](#t32) | [A21], [A22], [A31] |
| F073 | Provider self tests with detailed results | Missing | [T07](#t07) | [A23] |
| F074 | Unmodified CS3 loading and provider/extractor registrations | Partial | [T05](#t05), [T33](#t33) | [A21], [A05], [A06] |
| F075 | Plugin resources, Context/UI settings, custom actions and Android services | Missing | [T33](#t33) | [A21], [A22], [A32] |
| F076 | Plugin credentials, clone/override sites and provider capabilities | Missing | [T03](#t03), [T05](#t05), [T06](#t06) | [A05], [A27] |

### Network

| ID | Android capability / desktop gap | Desktop state | Work | Evidence |
|---|---|---|---|---|
| F077 | WebView request interception, JS callbacks and browser challenge flows | Missing | [T19](#t19), [T33](#t33) | [A24], [A25] |
| F078 | DNS over HTTPS selection and optional GitHub/jsDelivr proxy | Missing | [T19](#t19), [T30](#t30) | [A24], [A27] |
| F079 | Provider timeouts, cancellation, caching and error recovery | Partial | [T04](#t04), [T05](#t05), [T09](#t09) | [A05] |

### Transfer

| ID | Android capability / desktop gap | Desktop state | Work | Evidence |
|---|---|---|---|---|
| F080 | Selective settings/data backup and restore | Missing | [T25](#t25) | [A20] |
| F081 | Scheduled backups, destination and retention behavior | Missing / adapt | [T26](#t26) | [A20], [A31] |
| F082 | Transfer Android library, watched state, profiles and settings to Linux | Missing; desktop requirement | [T25](#t25) | [A17], [A20] |

### Preferences

| ID | Android capability / desktop gap | Desktop state | Work | Evidence |
|---|---|---|---|---|
| F083 | Searchable settings grouped by general/UI/player/providers/accounts/updates | Missing | [T01](#t01), [T30](#t30) | [A26], [A27], [A28], [A29], [A30], [A31] |
| F084 | Theme, accent, poster size/detail toggles and alternate layouts | Partial | [T30](#t30) | [A26], [A28] |
| F085 | Language selection, translated resources and initial setup/redo setup | Missing | [T30](#t30), [T31](#t31) | [A27], [A35] |
| F086 | Player, subtitle, network, download, update and backup preferences | Missing | [T01](#t01), [T16](#t16), [T17](#t17), [T19](#t19), [T20](#t20), [T26](#t26), [T32](#t32) | [A26], [A29], [A31] |

### Platform

| ID | Android capability / desktop gap | Desktop state | Work | Evidence |
|---|---|---|---|---|
| F087 | Chromecast discovery/session/queue/remote controls and styled subtitles | Missing / conditional | [T28](#t28) | [A33], [A09] |
| F088 | FCast discovery/session action | Missing / conditional | [T28](#t28) | [A33], [A32] |
| F089 | Phone/tablet/TV/emulator layouts, focus navigation and TV continue watching | Partial / adapt | [T30](#t30), [T27](#t27) | [A01], [A36] |
| F090 | System media session, headset controls, audio focus and screen wake | Missing / adapt | [T27](#t27) | [A07], [A36] |
| F091 | Application update checks, prerelease and installer choice | Missing / adapt | [T32](#t32) | [A31] |
| F092 | Log viewer/error reporting and runtime diagnostics | Partial / adapt | [T32](#t32) | [A31] |
| F093 | Project links, disclaimer/legal notices and banana/Monke easter egg | Partial / optional | [T30](#t30), [T32](#t32) | [A27], [A01] |
| F094 | Android storage permissions, battery exemptions, APK installer and receiver services | Platform-specific | [T27](#t27), [T32](#t32) | [A01], [A27], [A31] |

## Implementation work packages

Existing file targets are listed first; create focused new modules beside them as needed. Each package is a bounded feature area, often requiring multiple small patches. “High / specialist” work should not be approximated with placeholders. Dependencies are mandatory foundations, while optional cross-links can stay disabled until implemented.

| Task | Priority | Dependencies | Complexity |
|---|---|---|---|
| [T01 — Versioned storage, settings service and view state](#t01) | P0 | None | Medium |
| [T02 — Stable media/episode identity and progress migration](#t02) | P0 | T01 | High |
| [T03 — Complete typed provider/media/stream bridge](#t03) | P0 | T01, T02 | High |
| [T04 — Request scheduling, cancellation and incremental results](#t04) | P0 | T03 | High |
| [T05 — Preserve and measure extension compatibility](#t05) | P0 | T03 | High |
| [T06 — Repository and extension management parity](#t06) | P1 | T01, T03, T05 | Medium |
| [T07 — Extension updates, bulk actions and source tester](#t07) | P1 | T04, T05, T06 | Medium |
| [T08 — Federated search, pagination, history and suggestions](#t08) | P1 | T03, T04 | Medium |
| [T09 — Home parity, category fidelity and caching](#t09) | P1 | T03, T04, T10 | Medium |
| [T10 — Local library statuses, favorites and watched episodes](#t10) | P1 | T01, T02, T03 | Medium |
| [T11 — Local profiles, startup selection and access locks](#t11) | P1 | T01, T02, T10 | High |
| [T12 — Rich details and episode browser](#t12) | P1 | T03, T10 | Medium |
| [T13 — New-episode subscriptions and notifications](#t13) | P2 | T04, T09, T10, T11 | Medium |
| [T14 — Authoritative mpv session state and events](#t14) | P0 | T02, T03 | High |
| [T15 — Episode queues, resume and automatic next](#t15) | P1 | T12, T14 | Medium |
| [T16 — Player controls, quality policy and decoding preferences](#t16) | P1 | T03, T14, T15 | Medium |
| [T17 — Subtitles and audio selection/styles](#t17) | P1 | T03, T14, T19 | High |
| [T18 — Skip markers, previews, gestures and mini player](#t18) | P2 | T03, T14, T16, T17, T24 | High |
| [T19 — Session-aware media transport and WebView integration boundary](#t19) | P0 | T03, T04, T05 | High / specialist |
| [T20 — Persistent download engine and direct-file queue](#t20) | P1 | T01, T02, T03, T04, T19 | High |
| [T21 — HLS downloads, offline catalog and subtitles](#t21) | P1 | T17, T20 | High |
| [T22 — Online subtitle provider adapters](#t22) | P2 | T11, T17 | Medium per adapter |
| [T23 — Tracking accounts, remote libraries and progress sync](#t23) | P2 | T10, T11, T14, T24 | High per adapter |
| [T24 — Metadata matching, rich posters and filler enrichment](#t24) | P1 | T03 | High |
| [T25 — Desktop backup/export and Android backup import](#t25) | P1 | T01, T02, T10, T11 | High |
| [T26 — Scheduled backups and retention](#t26) | P2 | T25 | Medium |
| [T27 — Linux entry points, media integration and external actions](#t27) | P2 | T14 | High |
| [T28 — Casting: FCast first, Chromecast separately](#t28) | P3 | T03, T14, T15, T19, T27 | High / specialist |
| [T29 — Live, torrents and DRM: separate optional backends](#t29) | P3 | T03, T14, T19 | High / specialist |
| [T30 — Settings UI, appearance and desktop accessibility parity](#t30) | P1 | T01, T03 | Medium |
| [T31 — Localization and first-run setup](#t31) | P2 | T01, T06, T30 | Medium |
| [T32 — Diagnostics, app updates and release readiness](#t32) | P2 | T05, T07, T30 | High |
| [T33 — Advanced original-CS3 Android compatibility backend](#t33) | P3 | T04, T05, T19 | Very high / specialist |

<a id="t01"></a>

### T01 — Versioned storage, settings service and view state

**P0 · Medium · Depends on: none.**

Read Android: [A17], [A26].

Existing desktop entry points: `D/crates/core/src/store.rs`, `D/crates/core/src/models.rs`, `D/src-tauri/src/main.rs`, `D/src/lib/api.ts`, `D/src/lib/types.ts`, `D/src/App.svelte`.

1. Add an ordered, transactional migration runner using user_version. Preserve records version 1 and keep a copyable fixture of that database; do not require a new installation.
2. Introduce typed Settings with schemaVersion, global/device values and profile-scoped values. Persist in Rust, expose get/update commands, validate ranges and apply defaults to absent fields. Use the settings inventory below for semantics; add only settings with working consumers.
3. Move poster density from localStorage into settings with one-time migration and preserve its current choice. Extract typed UI state/error helpers as touched, without a whole-app refactor.

**Done when:**

- Version-1 repository, package, bookmark and history records survive upgrade and reopening. Failed migration rolls back.
- Settings update persists across restart and does not erase unknown future fields on unrelated updates. Defaults and invalid values are tested.

**Check:** Rust old-database fixture and migration failure test; npm run check; existing UI density test.

<a id="t02"></a>

### T02 — Stable media/episode identity and progress migration

**P0 · High · Depends on: T01.**

Read Android: [A05], [A17].

Existing desktop entry points: `D/crates/core/src/models.rs`, `D/crates/core/src/lib.rs`, `D/crates/core/src/store.rs`, `D/crates/core/src/player.rs`, `D/src/lib/types.ts`.

1. Introduce stable mediaId and episodeId plus alias records. Scope media by providerId and provider uniqueUrl when available, keeping the original provider URL separately. Preserve existing providerId and installed package IDs.
2. Use provider episode identity when supplied; otherwise parent mediaId + dub group + season + episode. For unnumbered episodes persist a disambiguated original-list identity with aliases; do not hash token-bearing data as the primary ID. Store fresh opaque data separately and retain data-hash aliases for existing entries.
3. Migrate bookmark/history records transactionally. Map ambiguous entries conservatively, retaining unresolved records for review. Resolve newest episode payload on resume without overwriting known progress. Define seconds for playback, epoch milliseconds for new timestamps, explicit conversions for legacy seconds.

**Done when:**

- Changed episode token retains progress; same numbered episode in two dub groups remains distinct; movie synthetic episode stays stable.
- Existing bookmarks/history remain visible. Missing provider can be reported without deleting progress. Duplicate/unnumbered episodes never overwrite each other.

**Check:** Rust identity/migration fixtures with Unicode, colon-containing provider names, duplicate names and changed token payloads.

<a id="t03"></a>

### T03 — Complete typed provider/media/stream bridge

**P0 · High · Depends on: T01, T02.**

Read Android: [A05], [A06].

Existing desktop entry points: `D/runtimes/jvm/src/main/kotlin/dev/cloudstream/worker/Main.kt`, `D/protocol/README.md`, `D/crates/core/src/models.rs`, `D/src/lib/types.ts`, `D/src/lib/api.ts`.

1. Implement the contract additions below in Kotlin mapping, Rust serde and TS types together. Preserve null vs absent semantics and unknown type strings. Add provider capability/timeouts, row identity/orientation, search score/quality/year and dub counts.
2. Add rich detail fields, structured dub/season/episode data, sync IDs, trailers/recommendations/actors and uniqueUrl. Keep the legacy episodes list during migration if consumers require it. Do not infer dub status from a translated name prefix.
3. Add discriminated streams for direct/HLS/DASH/playlist/torrent/DRM, separate audio resources and subtitle metadata. Retain extractorData and opaque resource handles for verifier/interceptor jobs. Negotiate methods/capabilities, with a protocol version bump for incompatible shapes.
4. Use a sample/fake upstream provider to return every supported field. Add serialization snapshots through all layers; rendering empty optional metadata is valid.

**Done when:**

- No field in an enriched fixture disappears or changes units across worker→Rust→UI. Old saved JSON loads with defaults.
- Unsupported capabilities are explicitly represented and do not become play/download buttons. Current sample and legacy smoke continue to work.

**Check:** Kotlin mapper tests, Rust deserialization fixtures, npm run check, worker smoke; update protocol documentation.

<a id="t04"></a>

### T04 — Request scheduling, cancellation and incremental results

**P0 · High · Depends on: T03.**

Read Android: [A03], [A05].

Existing desktop entry points: `D/runtimes/jvm/src/main/kotlin/dev/cloudstream/worker/Main.kt`, `D/crates/core/src/worker.rs`, `D/src-tauri/src/main.rs`, `D/protocol/README.md`, `D/src/lib/api.ts`.

1. Separate worker input reading, job execution and a synchronized stdout writer. Track request IDs, cancellation tokens and structured errors. Support out-of-order responses with Rust ID demultiplexing and bounded in-flight jobs.
2. Protect plugin load/unload and shared registries from active jobs. Schedule fairly across sources with bounded concurrency and per-provider serialization/delays when needed. Keep shared extractor visibility; do not casually split into isolated plugin processes.
3. Emit search/link progress events and final completion exactly once. Use a client generation/session ID so canceled or stale events cannot replace newer results. Retain a capped provider deadline and worker watchdog; only kill/restart when the affected runtime is unresponsive.
4. Add cancel support to Tauri and typed UI API; stop abandoned jobs on navigation, new query and window close. Worker restart should fail pending calls once, restore packages and recover the UI.

**Done when:**

- A hung source cannot indefinitely block a healthy source; canceling one search leaves another active request intact.
- Crash, malformed stdout, late response and unload during a job produce bounded recovery with no duplicate/ghost results.

**Check:** Fake slow/crashing worker and two-source integration fixtures; cancellation/stale UI tests.

<a id="t05"></a>

### T05 — Preserve and measure extension compatibility

**P0 · High · Depends on: T03.**

Read Android: [A05], [A21], [A25].

Existing desktop entry points: `D/runtimes/jvm/src/main/kotlin/dev/cloudstream/worker/LegacyDex.kt`, `D/runtimes/jvm/src/main/kotlin/dev/cloudstream/worker/Main.kt`, `D/crates/core/src/worker.rs`, `D/scripts/smoke-legacy.py`.

1. Create a capability report per package: original format, API revision, actual runtime, providers/extractors registered, required unavailable APIs, resource use and actionable failure. Distinguish an importer/action-only plugin from an empty provider.
2. Create lawful local fixtures for pure JAR, unchanged DEX, shared extractor producer/consumer, sanitized Kotlin ABI, unload/reload, old ABI and rejected Android UI/resource dependencies. Record package SHA/version and tested operations in a compatibility manifest.
3. Preserve exact external method descriptor resolution and content-addressed translation caches. Add a repeatable known-package smoke job when its package is supplied locally; do not distribute third-party packages without checking redistribution rights.
4. Exercise atomic update rollback and worker restoration with partially registered failing plugins. Keep UI honest about supported calls; capability probing must not execute speculative dangerous provider operations.

**Done when:**

- Fixtures pass through install→registration→search/home→load→links where applicable. Failed load removes partial registrations and restores previous version.
- The existing data-only CS3 route still works; Android/resource/WebView rejection remains explicit until a tested backend exists.

**Check:** Kotlin LegacyDexTest, Rust compatibility tests, worker smoke, optional supplied original-CS3 live smoke.

<a id="t06"></a>

### T06 — Repository and extension management parity

**P1 · Medium · Depends on: T01, T03, T05.**

Read Android: [A21], [A22], [A27].

Existing desktop entry points: `D/crates/core/src/repository.rs`, `D/crates/core/src/lib.rs`, `D/crates/core/src/models.rs`, `D/src-tauri/src/main.rs`, `D/src/App.svelte`.

1. Add repository refresh, rename/display metadata, copy/share link, local package picker/import and duplicate handling. Route all installations through existing bounded verification/atomic replacement, including local files.
2. Expose catalog filtering by name/language/type/status/installed/updates and package detail fields (authors/icon/size/source link). Separate installed but failed, loaded, update available and unsupported runtime.
3. Add provider enable/disable preferences, capability-aware source selection and clone-site configuration. For canBeOverridden providers create separate stable clone identities and stored URL/credential handles; no global mutation of the original source.
4. Implement repository short codes from RepositoryManager.parseRepoUrl: !-prefixed codes resolve a Location from py.md; other allowed codes use cutt.ly. Validate destinations, bound redirects/timeouts and report resolver unavailability; do not assume those external services remain available. Keep ordinary URL entry working. VotingApi/PluginDetailsFragment implement legacy voting, but the current Compose detail route does not show a vote control: treat voting as optional legacy parity, not a current blocker.

**Done when:**

- Local JAR and CS3 install use the same checks as remote packages; repositories sharing an internalName remain independent.
- Catalog filters work with zero loaded providers. Clone settings persist and removing a repository does not silently erase library history.

**Check:** Local fixture manifests/packages, malformed short-code output, duplicate repo and failed import tests; targeted UI tests.

<a id="t07"></a>

### T07 — Extension updates, bulk actions and source tester

**P1 · Medium · Depends on: T04, T05, T06.**

Read Android: [A21], [A22], [A23], [A31].

Existing desktop entry points: `D/crates/core/src/repository.rs`, `D/crates/core/src/worker.rs`, `D/crates/core/src/lib.rs`, `D/src-tauri/src/main.rs`, `D/src/App.svelte`.

1. Implement check-for-updates and explicit per-package/bulk update jobs with progress/cancel, changelog metadata if available and a summary of failures. Add automatic updates for installed packages only with a persisted setting and bounded schedule.
2. Keep old package hashes/records until replacement loads. Recover an interrupted update transaction on restart. Never auto-install every repository extension: current shared preferences explicitly mark the old auto-download mode unused.
3. Add a provider tester inspired by TestViewModel2: capability-aware home/search/details/links stages, duration and diagnostic output. Use a bounded user-started sample, do not download videos or test every title. Track unavailable network separately from bridge incompatibility.

**Done when:**

- A bad update leaves the previous version playable; mixed batch shows each result. No unexpected new packages are installed.
- Tester can cancel, handle sources lacking home/search/download and report which stage failed without freezing normal navigation.

**Check:** Two-version local repository fixtures, interrupted update recovery and slow-source tester tests.

<a id="t08"></a>

### T08 — Federated search, pagination, history and suggestions

**P1 · Medium · Depends on: T03, T04.**

Read Android: [A03], [A26], [A28].

Existing desktop entry points: `D/crates/core/src/lib.rs`, `D/src-tauri/src/main.rs`, `D/src/lib/api.ts`, `D/src/lib/types.ts`, `D/src/App.svelte`.

1. Add source multi-select plus all-enabled mode. Orchestrate bounded per-source queries using T04, stream results as each finishes and display source-scoped errors/retry. Keep query/session IDs through every event.
2. Keep page/hasNext/loading independently per source; support grouped and merged views without collapsing items by title. Add explicit source/type/language/quality/dub filters using structured data. Preferences filter eligible sources; loaded-item filters must not imply server-side search.
3. Persist per-profile history with delete and clear; debounce suggestions, cancel stale requests and provide an opt-out since queries leave the device. Add quickSearch only for providers advertising it. Metadata-source search and fallback to provider search come through T24, not guessed URLs.

**Done when:**

- One failed source still leaves healthy results. Rapid query A→B never shows A after B. Load more affects only the requested source.
- History survives restart, profile changes isolate it; disabling suggestions stops network calls. Empty query and no-source states are useful.

**Check:** Three fixture sources (fast/slow/error), pagination/stale cancellation and history UI tests; npm run check.

<a id="t09"></a>

### T09 — Home parity, category fidelity and caching

**P1 · Medium · Depends on: T03, T04, T10.**

Read Android: [A02], [A05].

Existing desktop entry points: `D/runtimes/jvm/src/main/kotlin/dev/cloudstream/worker/Main.kt`, `D/crates/core/src/lib.rs`, `D/src/App.svelte`, `D/src/lib/MediaRail.svelte`, `D/src/lib/Spotlight.svelte`.

1. Preserve each HomePageResponse/HomePageList name, list orientation and identity instead of flattening lists into a requested category. Keep section cursor and row page cursors separate and honor sequentialMainPage delays/scroll delays.
2. Add a bounded TTL cache by provider identity/package hash/section/page/language preferences. Show cached results while refreshing, invalidate on source or extension changes and avoid caching unresolved errors as empty success.
3. Remember selected source per profile, support no-source and random source/title actions, enrich spotlight from details without blocking rails, and add selected watch-status/favorite/subscription rails. Resume supports remove one/clear and uses T02 IDs.

**Done when:**

- A provider returning two named lists retains both with correct independent more behavior. Cache from source A never appears as source B.
- Source selection survives restart; slow spotlight details do not block search; library rail updates after status/profile changes.

**Check:** Multi-row provider fixture, sequential timing check, cache invalidation and homepage UI tests.

<a id="t10"></a>

### T10 — Local library statuses, favorites and watched episodes

**P1 · Medium · Depends on: T01, T02, T03.**

Read Android: [A15], [A17], [A04].

Existing desktop entry points: `D/crates/core/src/models.rs`, `D/crates/core/src/store.rs`, `D/crates/core/src/lib.rs`, `D/src-tauri/src/main.rs`, `D/src/App.svelte`.

1. Add named local watch statuses watching/completed/on_hold/dropped/plan_to_watch/none; keep favorites and subscription as independent attributes. Preserve old bookmarks as saved entries without inventing a prior watch status.
2. Implement tabs, local query, alphabetical/updated/release-date sorting and persistent selection. Support score sorting only when actual scores exist; Android LocalList intentionally omits its commented score sort.
3. Persist episode watched overrides separately from position and implement mark watched/unwatched/up to this episode in deterministic selected dub/season order. Add remove-one and confirmed bulk clear, and publish changes to Home/Details/Library.

**Done when:**

- Status, favorite and subscription can coexist without overwriting each other. Old bookmarks remain accessible.
- Watched overrides survive resume updates; mark-through respects dub group and ordering, and never marks unrelated episodes.

**Check:** Store reopen/migration and status transition tests; library/details action UI tests.

<a id="t11"></a>

### T11 — Local profiles, startup selection and access locks

**P1 · High · Depends on: T01, T02, T10.**

Read Android: [A16], [A17], [A18], [A30].

Existing desktop entry points: `D/crates/core/src/store.rs`, `D/crates/core/src/models.rs`, `D/crates/core/src/lib.rs`, `D/src-tauri/src/main.rs`, `D/src/App.svelte`.

1. Add a default profile during migration and scope library/history/watched/search history/preferences/service-account references by profileId. Repositories/packages stay device-wide initially; source selection and credentials are profile-scoped.
2. Implement create/edit/avatar/delete and startup selection/skip setting. On switch, cancel profile jobs, flush playback progress under the original profile, clear caches and reload scoped data. Keep orphaned downloaded files from being unexpectedly deleted.
3. Add optional PIN gate using a salted password hash and a reset/recovery design that does not promise encryption. Linux biometrics is optional behind detected OS support; do not copy Android BiometricPrompt or pretend support. New secrets belong in the OS keyring; absence has an explicit fallback/error policy.

**Done when:**

- Two profiles never see or update each other's history/tokens, including while player events arrive during a switch.
- Deleting a profile requires explicit confirmation and leaves installed packages intact. Legacy data lands in exactly one default profile.

**Check:** Migration fixtures, late-event/profile switch test, locked-profile navigation test and keyring mock integration.

<a id="t12"></a>

### T12 — Rich details and episode browser

**P1 · Medium · Depends on: T03, T10.**

Read Android: [A04], [A05].

Existing desktop entry points: `D/src/App.svelte`, `D/src/lib/MediaCard.svelte`, `D/src/lib/types.ts`, `D/src/lib/api.ts`, `D/src-tauri/src/main.rs`.

1. Build a detail view using optional backdrop/logo, score/rating scale, year/runtime, content rating, status/next airing, tags, actors/roles, recommendations and provider links. Render provider text safely; do not inject raw plot HTML.
2. Add season selector, structured dub/sub selector, episode ranges, sort modes and virtualized episode rows with synopsis/date/runtime/score/progress/filler state. Preserve episodeId/data when sorting or filtering.
3. Implement play/resume, mirror selection, link reload/cancel, bookmark status, favorite, subscription and copy/open/share actions. Gate download/cast/tracker actions on their implemented capabilities; do not add dead controls.
4. Add trailer playback with disable preference and cancellation on close; handle extractor-based trailers through worker rather than assuming every trailer is a direct video URL. Recommendations retain original source identity.

**Done when:**

- Large mixed-dub series stays responsive; selection sends exact chosen opaque data; changing sort never changes watched state.
- Optional metadata absent or a failed trailer does not prevent episode play. Details restore keyboard focus when dismissed.

**Check:** Rich/empty/movie/anime/large-series fixtures; opaque data and accessible dialog tests.

<a id="t13"></a>

### T13 — New-episode subscriptions and notifications

**P2 · Medium · Depends on: T04, T09, T10, T11.**

Read Android: [A19], [A15], [A17].

Existing desktop entry points: `D/crates/core/src/lib.rs`, `D/crates/core/src/store.rs`, `D/src-tauri/src/main.rs`, `D/src/App.svelte`.

1. Persist subscription snapshot per profile/media/dub group with last observed episode identities/counts and last notification state. Port Android's periodic check intent (source schedules roughly every six hours), not WorkManager classes.
2. Add bounded checks on app start/while running, manual refresh and notification opt-in. Reuse cached details and provider deadlines. Notify only for actual new available episodes, not sorting changes or duplicate polls.
3. Define Linux lifetime explicitly: first version checks while app runs; closed-app checking requires a later opt-in user service/tray implementation. Notification opens correct media/episode through T27 when available.

**Done when:**

- New episode yields one notification; restart, transient provider failure and unchanged results do not repeat it.
- Unsubscribe/profile deletion cancels pending work; no claims that checks run while fully closed.

**Check:** Fake clock/provider snapshots and notification sink; subscription UI test.

<a id="t14"></a>

### T14 — Authoritative mpv session state and events

**P0 · High · Depends on: T02, T03.**

Read Android: [A07], [A08].

Existing desktop entry points: `D/crates/core/src/player.rs`, `D/crates/core/src/lib.rs`, `D/src-tauri/src/main.rs`, `D/src/lib/api.ts`, `D/src/lib/types.ts`, `D/src/App.svelte`.

1. Keep mpv as the working playback backend. Add a persistent IPC reader, request/response IDs and observe_property events for position/duration/pause/volume/speed/tracks/cache. Handle file-loaded, end-file(reason), shutdown and command errors.
2. Create a playback sessionId carrying profileId/mediaId/episodeId and source selection; publish typed player events from Rust to Tauri. Replace optimistic nowPlaying/paused UI with actual state; throttle progress events and flush at pause, end, stop and app close.
3. Keep the existing private short Unix-socket directory permissions and literal-header handling. Clean up only processes/sockets owned by the session. Do not kill unrelated mpv processes; handle missing mpv and window closure.

**Done when:**

- Closing/pausing native mpv immediately updates desktop state and saves progress to correct episode/profile.
- Stop is idempotent; failed open does not show playing; stale events cannot affect a new session.

**Check:** Extend native player_linux integration for property events/exit; fake IPC event ordering tests; player UI state test.

<a id="t15"></a>

### T15 — Episode queues, resume and automatic next

**P1 · Medium · Depends on: T12, T14.**

Read Android: [A07], [A04], [A29].

Existing desktop entry points: `D/crates/core/src/player.rs`, `D/crates/core/src/lib.rs`, `D/src/lib/types.ts`, `D/src/App.svelte`.

1. Add a playback queue of episode references (not expiring resolved URLs), previous/next selection and exact current index. Resolve links just in time and reuse the selected quality/language policy.
2. Define completion/resume thresholds as configurable documented desktop policy; retain explicitly watched state separately. Save final progress, autoplay only on normal EOF, respect auto-next/start-paused preferences and optionally prefetch next links with cancellation.
3. Handle last episode, disabled/unavailable source, network failure and manually stopped playback. Preserve position on mirror changes and ask/show recoverable error when no source remains. Resume reloads current provider data using T02 aliases.

**Done when:**

- EOF advances exactly once; user stop/error never unexpectedly advances. Same episode resumes after URL/token refresh.
- Next/previous stays in selected dub/season ordering, honors autoplay off and never plays another profile's queue.

**Check:** Fake EOF/manual stop/error fixtures; two short local video native integration; queue UI test.

<a id="t16"></a>

### T16 — Player controls, quality policy and decoding preferences

**P1 · Medium · Depends on: T03, T14, T15.**

Read Android: [A08], [A10], [A29].

Existing desktop entry points: `D/crates/core/src/player.rs`, `D/crates/core/src/models.rs`, `D/src-tauri/src/main.rs`, `D/src/App.svelte`.

1. Expose volume/mute, absolute seek/scrubber, speed, fullscreen, aspect/fit and audio/video delay through a typed command enum with validated numeric limits. Keep pause/relative seek/stop working. Add rotation where mpv supports it.
2. Implement quality/source priority profiles with actual upstream labels/quality values, preferred language and deterministic tie-breaks. Allow manual mirror switching at the same position and bounded failover; preserve request context per resource.
3. Wire start-paused, hardware/software decoding, buffer/cache limits and optional dynamic audio compressor to actual mpv properties/options. Validate options against the installed mpv version and show unsupported capabilities. Multi-segment ExtractorLinkPlayList needs a tested timeline adapter; do not play just its first segment.

**Done when:**

- Controls match actual mpv state and persist intended settings; rejected commands produce useful errors.
- Quality selection handles unknown quality without inventing resolution, and switching source retains position and chosen tracks when possible.

**Check:** IPC command fixtures, local alternate mirrors/playlist fixture and native control smoke; UI keyboard control tests.

<a id="t17"></a>

### T17 — Subtitles and audio selection/styles

**P1 · High · Depends on: T03, T14, T19.**

Read Android: [A06], [A07], [A09].

Existing desktop entry points: `D/crates/core/src/player.rs`, `D/crates/core/src/models.rs`, `D/src-tauri/src/main.rs`, `D/src/App.svelte`.

1. Expose embedded tracks from mpv plus resolved external audio/subtitles with language/format/label/forced/hearing-impaired metadata. Add track pickers, off option, local subtitle file picker, language preference and subtitle/audio delay.
2. Use T19 request context for header/cookie-protected subtitles and separate audio; do not pass one global subtitle header map to all resources. Support common subtitle files and archives through bounded safe extraction; keep temporary assets through session lifetime.
3. Add subtitle preview and persisted font/size/color/bold/italic/outline/shadow/background/alignment/elevation settings, reset/undo and custom font picker. Map to mpv/libass options accurately; explicitly report unsupported styling combinations.
4. Caption/music/bloat removal and uppercase are opt-in text transforms on a copied asset; never overwrite the downloaded original. Respect ASS styling unless the user requests override.

**Done when:**

- Protected subtitle and separate audio fixtures play with correct per-resource headers; language/off preference survives restart.
- Switching tracks/delay/style does not restart progress or modify originals; Unicode/RTL subtitle and path-with-spaces fixtures work.

**Check:** Local header-protected subtitle/audio server fixture, track-event tests, style preview/reset UI tests and native playback smoke.

<a id="t18"></a>

### T18 — Skip markers, previews, gestures and mini player

**P2 · High · Depends on: T03, T14, T16, T17, T24.**

Read Android: [A07], [A08], [A09], [A11], [A29], [A36].

Existing desktop entry points: `D/crates/core/src/player.rs`, `D/crates/core/src/lib.rs`, `D/src/App.svelte`, `D/src/style.css`.

1. Add episode chapters/skip intervals as optional timed metadata. Implement source-specific AniSkip/AnimeSkip/IntroDbSkip/TheIntroDBSkip adapters one at a time with metadata ID matching, provider enable switches and a manual skip fallback. Never guess timestamp matches from title text alone.
2. Add bounded thumbnail preview generation/caching for seekbar with timeout/cancel; avoid spawning unlimited ffmpeg/mpv processes. Add optional metadata overlay and control visibility preferences.
3. Implement configurable keyboard/double-click seek, temporary speed hold and control lock; touch gestures are optional. Use desktop volume/brightness controls without silently changing system brightness.
4. Implement mini-player/picture-in-picture as an always-on-top native mpv/window mode with compositor-aware capability fallback. Do not promise a browser PiP feature for an external mpv renderer.

**Done when:**

- Wrong/unmatched metadata never triggers automatic skip. Skip end stays within duration and preference off stops service calls.
- Preview requests cancel on close, disabled previews use no extraction resources, and fullscreen/mini-player can be exited by keyboard.

**Check:** Fixture skip-service responses, local video thumbnails with cancellation, gesture/keyboard UI test and native window-mode manual smoke.

<a id="t19"></a>

### T19 — Session-aware media transport and WebView integration boundary

**P0 · High / specialist · Depends on: T03, T04, T05.**

Read Android: [A05], [A06], [A24], [A25].

Existing desktop entry points: `D/runtimes/jvm/src/main/kotlin/dev/cloudstream/worker/Main.kt`, `D/crates/core/src/worker.rs`, `D/crates/core/src/player.rs`, `D/src-tauri/src/main.rs`, `D/protocol/README.md`.

1. Design an opaque resource/session handle so provider-owned OkHttp cookie jar, getVideoInterceptor and extractorVerifierJob stay in the worker that owns them. Rust/player asks that service for media rather than attempting to serialize an interceptor or session cookies into UI fields.
2. Add a loopback-only, random-port media transport with per-session unpredictable tokens (or equivalent private transport). Preserve GET/HEAD/Range/status/Content-Range/cancellation; rewrite nested HLS/DASH resource references to retain correct context; restrict upstream origins according to resource/session policy. Do not expose arbitrary fetch endpoints or attach credentials to unrelated redirects.
3. Route protected posters, subtitles and audio independently. Keep credentials out of mpv argv/diagnostics where possible; do not send raw session state to the UI. Maintain an explicit fallback for ordinary header-only links.
4. The JVM WebView resolver is a TODO. First add structured unsupported reports and bridge interfaces; actual WebView/browser request interception is a separate T33 milestone. Tauri navigation callbacks alone do not provide all upstream request interception/body/cookie semantics. Add DoH/proxy selection only with a real shared network consumer and correct cache/session scoping.

**Done when:**

- Local manifest→segment→key/subtitle/audio fixture receives exact per-origin headers/cookies and Range behavior. Verifier starts/stops with playback.
- Invalid token, redirect to unrelated origin, canceled session and worker crash fail closed with useful errors; no generic publicly reachable proxy exists.

**Check:** Deterministic multi-origin transport fixture suite, verifier lifecycle fixture, IPC reconnect tests; no live challenge service needed for CI.

<a id="t20"></a>

### T20 — Persistent download engine and direct-file queue

**P1 · High · Depends on: T01, T02, T03, T04, T19.**

Read Android: [A12], [A13], [A27].

Existing desktop entry points: `D/crates/core/src/lib.rs`, `D/crates/core/src/store.rs`, `D/crates/core/src/models.rs`, `D/src-tauri/src/main.rs`, `D/src/App.svelte`.

1. Create new download modules with persisted job state machine queued/resolving/downloading/paused/completed/failed/canceled and progress events. Store profile/media/episode identity, provider payload reference, destination, selected policy and validators; URLs may expire.
2. Implement direct-file GET with resumable Range, ETag/Last-Modified validation, retries/backoff, per-job cancellation and bounded global jobs/per-file connections. Detect servers ignoring Range and restart safely rather than append duplicate bytes.
3. Add pause/resume/cancel/retry/reorder/clear queue, mirror choice, single/range/all episode enqueue with duplicate protection. Choose folder via native picker, sanitize filenames and persist metadata independently from display names. Confirm disk file deletion separately from queue removal.
4. Use .part files, atomic completion, free-space errors, startup recovery and visible size/speed/remaining state. Preserve session transport from T19; no browser-only fake downloader.

**Done when:**

- Restart during transfer resumes or safely restarts without corrupt output; completed file matches fixture hash.
- Cancel releases connections, no writes escape the chosen directory, duplicate enqueue is idempotent and max jobs/connections is respected.

**Check:** Local HTTP servers with 206/200/416, changed ETag, disconnect and disk-error fixtures; persistent queue/UI tests.

<a id="t21"></a>

### T21 — HLS downloads, offline catalog and subtitles

**P1 · High · Depends on: T17, T20.**

Read Android: [A12], [A13], [A14].

Existing desktop entry points: `D/crates/core/src/lib.rs`, `D/crates/core/src/store.rs`, `D/crates/core/src/player.rs`, `D/src/App.svelte`.

1. Extend downloads to HLS master/media playlists, relative segments, byte ranges, init segments, AES-128 keys and discontinuities with deterministic ordered resume state. Select variant/audio tracks according to policy; validate supported encryption/types and report unsupported DRM rather than corrupting a file.
2. Decide and document output strategy (local playlist/assets or a validated remux tool). Do not concatenate arbitrary fragmented MP4 segments and call them a valid MP4. Use T19 per-resource session handling and protect key storage.
3. Add offline catalog grouped by title/season, episode progress, subtitle selection/download, file existence repair, storage size and confirmed deletion. Play entirely from local assets with network disabled and save progress against the same stable episode identity.
4. For a file downloaded by another profile, preserve original ownership and apply deliberate sharing policy; do not delete media just because a profile or plugin is removed. DASH offline downloading is optional until separately implemented/tested.

**Done when:**

- Fixture HLS with separate audio, relative URLs and restart produces playable complete media; missing/changed segment is diagnosed.
- Offline movie/episode and subtitles work with networking blocked. Delete cleans only owned assets and metadata, with no orphan partials.

**Check:** Deterministic HLS fixture suite, offline native smoke and catalog/delete UI tests.

<a id="t22"></a>

### T22 — Online subtitle provider adapters

**P2 · Medium per adapter · Depends on: T11, T17.**

Read Android: [A18], [A07], [A09], [A40].

Existing desktop entry points: `D/crates/core/src/lib.rs`, `D/crates/core/src/models.rs`, `D/src-tauri/src/main.rs`, `D/src/App.svelte`.

1. Define SubtitleProvider trait for search/resource/login/logout/capabilities and per-profile account selection. Implement one adapter per patch: OpenSubtitles, Addic7ed, Subdl, SubSource. Read its Android implementation and current official API requirements; source registration alone does not prove the service still works.
2. Search by structured title/IDs/year/season/episode/language/hearing-impaired preference; preserve release and language labels. Download selected resource with its headers/session, unpack archives safely and attach through T17; no automatic wrong-title fallback.
3. Use OS keyring references, token refresh/rate-limit handling and real disconnected/connected states. Supply service app IDs/keys through documented configuration where needed, never copy upstream private credentials. Support unauthenticated adapters only if their actual API permits it.

**Done when:**

- Each adapter passes the common fake-transport contract before a live smoke is declared. Wrong/no results and API limit errors remain distinct.
- Credentials stay profile-scoped and logout revokes local access; subtitle selection actually changes the player.

**Check:** Recorded/redacted response fixtures; mock auth/expiry/429 tests; optional user-authorized live smoke per service.

<a id="t23"></a>

### T23 — Tracking accounts, remote libraries and progress sync

**P2 · High per adapter · Depends on: T10, T11, T14, T24.**

Read Android: [A18], [A15], [A04], [A39].

Existing desktop entry points: `D/crates/core/src/lib.rs`, `D/crates/core/src/store.rs`, `D/crates/core/src/models.rs`, `D/src-tauri/src/main.rs`, `D/src/App.svelte`.

1. Define TrackingProvider trait and shared account/remote-library models. Implement AniList, MyAnimeList, Kitsu and Simkl as separate patches using source behavior plus verified current official auth/API requirements. Do not add a Trakt tracking account merely because a Trakt metadata provider exists.
2. Use external-browser OAuth with validated state and PKCE where supported, native deep-link/loopback callbacks, secret storage and token refresh/revoke. Other login methods follow each service's supported flow. Multiple accounts are scoped to a local profile.
3. Add remote-library service selection, refresh, paginated status tabs, supported sorts, manual metadata-ID match correction and editing score/status/episode progress. Keep local and remote status enum conversion explicit; remote rewatching is not local none.
4. Queue idempotent sync mutations with origin/timestamp, offline retry/rate limits and visible conflict resolution. Define watched threshold policy and only sync eligible completed episode progress; dragging a seekbar must not mark an entire series complete. Prevent feedback loops when applying remote changes.

**Done when:**

- Every adapter passes a common contract with real score scale/status mapping; account switching never cross-writes libraries.
- Offline completion queues once, reconnect syncs once; expired auth prompts re-login without erasing local progress. Conflicts are visible and recoverable.

**Check:** Fake OAuth server and service responses, status/score round trips, retry/idempotency/profile isolation tests; optional live smoke per configured service.

<a id="t24"></a>

### T24 — Metadata matching, rich posters and filler enrichment

**P1 · High · Depends on: T03.**

Read Android: [A05], [A37], [A04], [A18].

Existing desktop entry points: `D/runtimes/jvm/src/main/kotlin/dev/cloudstream/worker/Main.kt`, `D/crates/core/src/models.rs`, `D/crates/core/src/lib.rs`, `D/src/App.svelte`.

1. Preserve provider syncData and supportedSyncNames/getLoadUrl capability. Define MediaIdentity metadata-ID map and matching confidence/provenance; never equate titles by display string alone.
2. Expose metadata providers through the existing worker when library support permits, or add a clearly separated adapter. Start with supplied provider IDs, then explicit user-selected search matches; add TMDB/cross-TMDB/Trakt/MyDramaList only as each source is supported.
3. Merge optional posters/cast/recommendations/airing/filler data without replacing provider episode payloads or source identity. Respect Kitsu poster/filler/trailer toggles; cache by service ID/locale and preserve original provenance. Trackers can correct a match manually.
4. Provide metadata-to-source lookup via getLoadUrl or real provider search and user selection; never synthesize source URLs from metadata IDs.

**Done when:**

- Same title/remake/year mismatch does not auto-link; manual correction survives restart.
- Missing metadata/API outage leaves core browsing playable; source opaque data is unchanged by enrichment.

**Check:** Ambiguous/remake/anime/movie fixtures, metadata provenance merge tests and matched-source search UI test.

<a id="t25"></a>

### T25 — Desktop backup/export and Android backup import

**P1 · High · Depends on: T01, T02, T10, T11.**

Read Android: [A20], [A17], [A26].

Existing desktop entry points: `D/crates/core/src/store.rs`, `D/crates/core/src/models.rs`, `D/src-tauri/src/main.rs`, `D/src/App.svelte`.

1. Create a versioned desktop backup format for profiles/library/watched/progress/settings/repo references, with selective export/import, schema version, source revision and checksum. Export a consistent database snapshot; exclude runtime paths/caches/plain secrets by default.
2. Add an Android adapter for BackupFile {datastore,settings}, each with _Bool/_Int/_String/_Float/_Long/_StringSet maps. Parse nested JSON strings according to DataStoreHelper and account key namespaces. Android numeric hash IDs are not Linux IDs: reconcile provider names+URLs/repository evidence and retain unresolved items.
3. Map watch statuses/profiles/history/settings explicitly, handle old keys and units, and present an import preview with counts/unmatched entries/conflict policy. Select import settings vs user data; validate size/type before touching DB, then apply a transaction with rollback and a pre-import backup.
4. Do not import Android filesystem paths, APK installer/permissions or silently install plugins/tokens. Keep unresolved provider references for later matching. Export Android-compatible format only if a tested reverse mapping is implemented; desktop export alone does not imply Android round-trip support.

**Done when:**

- Fixture Android backup imports statuses/profiles/progress with no ID collisions or token-based duplicates; unresolved sources remain reviewable.
- Corrupt/newer backup fails before mutation, re-import follows chosen dedup policy, and restore failure rolls back all changes.

**Check:** Synthetic Android old/new fixtures without private data, desktop export→restore round-trip, migration rollback and import-preview UI tests.

<a id="t26"></a>

### T26 — Scheduled backups and retention

**P2 · Medium · Depends on: T25.**

Read Android: [A20], [A31].

Existing desktop entry points: `D/crates/core/src/lib.rs`, `D/crates/core/src/store.rs`, `D/src-tauri/src/main.rs`, `D/src/App.svelte`.

1. Add backup destination/frequency/retention preferences with actual scheduler behavior. Start with app-running schedule and startup catch-up; closed-app operation is an opt-in user service rather than a hidden promise.
2. Use temporary output and atomic rename, test destination writeability, enforce bounded retention for app-owned backup names and expose last success/failure. Never delete unrelated files in the chosen directory.
3. Handle missed timers, concurrent manual/scheduled export and low disk space. Preserve secrets exclusion and snapshot consistency from T25.

**Done when:**

- Fake clock triggers one backup per interval; restart catches up without an export loop.
- Retention deletes only eligible owned backups after a new valid backup succeeds; unwritable destination does not damage previous backups.

**Check:** Fake-clock/export/retention filesystem fixtures; settings and failure-display UI test.

<a id="t27"></a>

### T27 — Linux entry points, media integration and external actions

**P2 · High · Depends on: T14.**

Read Android: [A01], [A32], [A36].

Existing desktop entry points: `D/src-tauri/tauri.conf.json`, `D/src-tauri/src/main.rs`, `D/src-tauri/capabilities/main.json`, `D/crates/core/src/lib.rs`, `D/crates/core/src/player.rs`, `D/src/App.svelte`.

1. Add validated OS deep links/file associations and single-instance routing for repository, search, title-share, player and resume actions. Derive parsing from MainActivity/AccountManager/IntentHelpers, not from scheme names alone. Queue entry actions until storage and worker are ready.
2. Add native file/URL entry, clipboard/copy, source website and preferred player selection. Map Android player packages to detected Linux mpv/VLC/custom executable choices and preserve supported headers/subtitle/position semantics. Use argv arrays, never shell interpolation of media or extension data.
3. Implement MPRIS/D-Bus metadata/play/pause/seek/next controls, compositor sleep inhibition while playing, audio-focus policy and opt-in notifications. Integrate real player events; avoid conflicting duplicate mpv MPRIS ownership.
4. Desktop TV/remote mode may retain keyboard focus and optional clock; Android TV channels, battery exemptions, scoped-storage permissions, cast SDK services and APK installation do not transfer to Linux. Document the chosen Linux lifetime when background work is added.

**Done when:**

- Cold-start and running-app links select the correct source/profile and malformed links produce useful errors without accidental install/play.
- Hardware controls change current session; source filenames/URLs containing quotes cannot execute a command. File associations and single-instance behavior work in a packaged Linux smoke test.

**Check:** Deep-link parser fixtures, argv tests, mocked D-Bus/portal tests and packaged .desktop manual smoke.

<a id="t28"></a>

### T28 — Casting: FCast first, Chromecast separately

**P3 · High / specialist · Depends on: T03, T14, T15, T19, T27.**

Read Android: [A33], [A09], [A32].

Existing desktop entry points: `D/crates/core/src/lib.rs`, `D/crates/core/src/models.rs`, `D/src-tauri/src/main.rs`, `D/src/App.svelte`.

1. Implement a cast-backend interface with discovery/device/session/queue/track/control/error capabilities. Start with FCast using the existing protocol source as evidence; add Chromecast as a separate implementation using a supported maintained receiver protocol/library.
2. Cast receiver fetches media independently: expose a LAN-reachable opt-in relay with scoped unpredictable resource tokens, per-session lifetime and supported media/subtitle formats. The loopback-only transport in T19 cannot simply be reused unchanged. Preserve authentication context without leaking cookies to the receiver.
3. Transfer queue/progress, remote pause/seek/volume/next, reconnect/disconnect and cast subtitle style preferences. Gate UI on provider.hasChromecastSupport and actual receiver support. Do not label generic screen mirroring as equivalent.

**Done when:**

- A real device plays a supported authenticated fixture via scoped relay, reports position and disconnects cleanly.
- Device disappearance and blocked LAN discovery are recoverable; sessions revoke relay access on end, and unsupported formats get explicit errors.

**Check:** Protocol packet fixtures and fake receiver; real FCast/Chromecast device smoke separately recorded.

<a id="t29"></a>

### T29 — Live, torrents and DRM: separate optional backends

**P3 · High / specialist · Depends on: T03, T14, T19.**

Read Android: [A05], [A06], [A08], [A34].

Existing desktop entry points: `D/runtimes/jvm/src/main/kotlin/dev/cloudstream/worker/Main.kt`, `D/crates/core/src/models.rs`, `D/crates/core/src/player.rs`, `D/src-tauri/src/main.rs`, `D/src/App.svelte`.

1. Split this package into independent Live, Torrent and DRM patches. First preserve typed stream metadata and report the missing capability rather than attempting ordinary mpv playback.
2. Live: distinguish infinite/unknown duration, live edge and DVR seek window; disable VOD completion/autoplay assumptions and handle reconnect/channel metadata.
3. Torrent: add an optional libtorrent-compatible engine or external torrent integration, explicit session start/stop/storage/progress and magnet/file support. Test with an openly licensed local torrent fixture; close sessions and remove only selected temporary assets.
4. DRM: determine lawful supported runtime/decoder/CDM and license acquisition per DRM type. mpv does not provide Android Media3 DRM equivalence automatically. A real Android backend or licensed supported desktop playback stack may be needed. Preserve license configuration securely and leave unsupported systems explicitly disabled; do not claim generic Widevine support.

**Done when:**

- Each backend has its own capability flag, tests and optional dependency detection; a working live stream does not imply torrent or DRM support.
- Live does not become watched-complete at the edge; torrent cleanup is bounded; DRM only marked supported after a real authorized encrypted fixture succeeds.

**Check:** Local live playlist/DVR fixture, local lawful torrent swarm, authorized DRM test stream only for actually supported scheme.

<a id="t30"></a>

### T30 — Settings UI, appearance and desktop accessibility parity

**P1 · Medium · Depends on: T01, T03.**

Read Android: [A26], [A27], [A28], [A29], [A30], [A31], [A36].

Existing desktop entry points: `D/src/App.svelte`, `D/src/style.css`, `D/src/lib/MediaCard.svelte`, `D/src/lib/Spotlight.svelte`, `D/src-tauri/src/main.rs`.

1. Build searchable settings sections for General, Appearance, Player, Subtitles, Providers, Accounts, Updates/Backup and Diagnostics. Each option must call an implemented settings consumer; hide/defer unsupported controls with clear capability text.
2. Keep the approved desktop design as default. Add accent/theme choices, adjustable poster size/title placement/badge visibility, search view choices and content display toggles. Preserve full media types including Live/Cartoon/OVA/AsianDrama/Documentary/Audio etc as data; never silently cast unknown to Movie.
3. Adapt TV/emulator/phone layout settings into responsive desktop/compact/optional remote navigation. Add predictable focus, keyboard shortcuts, tooltips, readable contrast, reduced-motion support and virtualized lists for large catalogs. Confirm-exit should consider active downloads/playback.
4. Add project/support/legal links with actual destinations and license notices. Banana/Monke easter egg is an optional cosmetic subtask; do not prioritize it over missing functional behavior.

**Done when:**

- Existing density/layout flows remain usable. Settings survive restart and unsupported toggles cannot give false success.
- Desktop and narrow layouts pass existing accessibility tests, keyboard can reach every actionable control and long translated strings fit.

**Check:** Existing 10 browser tests, focused settings persistence tests and npm run check; manual keyboard/large-library smoke.

<a id="t31"></a>

### T31 — Localization and first-run setup

**P2 · Medium · Depends on: T01, T06, T30.**

Read Android: [A35], [A27].

Existing desktop entry points: `D/src/App.svelte`, `D/src/lib/types.ts`, `D/src/style.css`, `D/src-tauri/src/main.rs`.

1. Introduce frontend i18n keys and locale-aware date/number/plural formatting. Port reusable Android translations carefully with attribution; do not assume Android resource placeholders/plurals map directly to the frontend system.
2. Add first-run flow for interface language, provider languages, media preferences, desktop layout and explicit repository/extension selection. Persist completion and allow redo without clearing data.
3. Include language fallback, RTL direction/layout checks and keyboard navigation. Preserve explicit extension install consent and do not revive auto-download-all behavior from old XML settings.

**Done when:**

- English/fallback and an RTL locale render correctly; switching locale preserves browsing state/settings.
- Skipping setup leaves a useful honest empty state; redo does not delete repos/library or install unselected packages.

**Check:** Locale fallback/plural fixtures, RTL/narrow-layout UI tests and setup restart/redo test.

<a id="t32"></a>

### T32 — Diagnostics, app updates and release readiness

**P2 · High · Depends on: T05, T07, T30.**

Read Android: [A31], [A38].

Existing desktop entry points: `D/src-tauri/tauri.conf.json`, `D/src-tauri/src/main.rs`, `D/crates/core/src/worker.rs`, `D/scripts/build-worker.sh`, `D/README.md`, `D/NOTICE.md`.

1. Add bounded log/event collection with a diagnostics view/export: app/worker/runtime versions, package hashes, capability failures and request durations. Redact tokens/cookies/auth headers/PINs/media query secrets; logcat becomes native application diagnostics.
2. Add update availability/channel/skip checks from the desktop project's own configured release channel; do not install Android APK releases. Distinguish distro package-manager updates from a signed supported updater flow. Require configured verification/signing before automatic install.
3. Validate portable and .deb on clean supported Linux machines with missing mpv/runtime/system libraries, correct resource discovery, X11/Wayland, offline start and upgrade preserving user data. AppImage and alternate architectures remain separate targets until tested.
4. Produce release dependency/license inventory including upstream library, shims, dex2jar, Java runtime and frontend fonts/assets. Preserve AI policy disclosure for any upstream contribution; no PR/publish is part of this task by default.

**Done when:**

- Diagnostics export contains useful failures but no fixture secret; bounded log storage cannot fill disk.
- Update UI never calls an Android installer or installs unsigned artifacts. Clean-machine reports distinguish tested distributions/architectures from untested ones.

**Check:** Redaction fixtures, fake update feed/signature errors, clean-machine package checklist and version-1 data upgrade smoke.

<a id="t33"></a>

### T33 — Advanced original-CS3 Android compatibility backend

**P3 · Very high / specialist · Depends on: T04, T05, T19.**

Read Android: [A21], [A22], [A24], [A25], [A32].

Existing desktop entry points: `D/runtimes/jvm/src/main/kotlin/dev/cloudstream/worker/LegacyDex.kt`, `D/runtimes/jvm/src/main/kotlin/dev/cloudstream/worker/Main.kt`, `D/protocol/README.md`, `D/crates/core/src/worker.rs`, `D/src-tauri/src/main.rs`.

1. First build a compatibility corpus and classify Android dependencies by exact API usage: inert data helpers, resources/assets, WebView/network, plugin settings/actions and full Android app services. This task is several milestones, not one large stub patch.
2. For a narrow pure helper API, implement behavior-accurate shim plus unchanged DEX fixture; reject unsupported calls. Do not add a fake Context whose methods silently return null to make loading appear successful. Preserve working DEX/JAR path as the default.
3. For real resources/UI/WebView/app-class dependencies, design a separate Android host (for example opt-in Waydroid) compiled with the pinned compatible app APIs. Host shared providers/extractors there and expose a versioned authenticated local transport using the same logical protocol; include install/lifecycle/data-directory ownership and capability negotiation.
4. WebView support must cover request interception/redirects, headers/cookies, JS callbacks, cancellation and proper origin/session isolation. Android openSettings/actions need real hosted UI or explicit unsupported behavior; a Rust form cannot execute an arbitrary Android View.
5. Prove unchanged representative plugins work end-to-end and define cross-runtime extractor dependencies before marking a class of plugins compatible. Waydroid is a candidate requiring environment support, not a promised automatic solution.

**Done when:**

- Representative resource, WebView and UI plugin fixtures run unchanged in the selected backend; absent Android host yields explicit capability error.
- JAR/data-only CS3 regressions remain green, backend crashes recover and authentication transport is not a public service. No universal compatibility claim from a single package.

**Check:** Dependency-class fixture corpus, isolated Android-host integration and real browser challenge fixture where lawful/reproducible; specialist architecture review.

## Data model and protocol contracts

These are **proposed desktop contracts**, not claims about existing code or JSON emitted verbatim by Android. Implement T01–T03 first and settle field names there. Later tasks consume these contracts rather than inventing parallel representations.

### Identity and persistence

- Keep `InstalledPlugin.id` derived from repository identity and internalName, and `Provider.id` from that package identity plus actual provider identity. Preserve current IDs through migrations; package source format and runtime execution mode are separate fields.
- `MediaRef`: `mediaId`, `providerId`, original `url`, optional `uniqueUrl`, optional external metadata IDs. URL normalization for repository entry must never be applied to provider media URLs or episode payloads.
- `EpisodeRef`: `episodeId`, `mediaId`, `dubStatus`, optional `season`/`episode`, stable fallback identity, current **opaque `data`**, and aliases. Persist the latest payload separately from progress. A stable hash is acceptable for a stable identity tuple, not for token-bearing playback data.
- `WatchProgress`: profileId/mediaId/episodeId, `positionSeconds`, `durationSeconds?`, `updatedAtMs`, explicit watched override and completion reason. New timestamps are epoch milliseconds; legacy `models.rs::timestamp()` returns seconds, Android dates vary by model, and duration fields are not all the same unit. Convert explicitly.
- Local `WatchType`: watching=0, completed=1, onHold=2, dropped=3, planToWatch=4, none=5 in Android. `SyncWatchType` has none=-1 and rewatching=5. **Never persist a bare shared integer enum**; use names plus per-service conversion functions. Favorite/subscription are independent.
- `Profile`: stable ID/name/avatar/startup preference/optional PIN hash. Device-level packages and profile-level progress/tokens have separate ownership. Service token values live in OS credential storage, database stores references.
- Use schemaVersion on backup/large persisted objects; unknown optional fields must not destroy old data on read-modify-write. Migrations test actual v1 records. Failed extensions and unresolved imports remain represented rather than deleted.

### Provider and browse models

`ProviderCapabilities` should expose actual `hasMainPage`, `hasQuickSearch`, download/cast capability, `usesWebView`, supported media/sync types, override support and effective runtime support. Optional plugin timeout hints are capped and honored, not blindly trusted. Retain sequential home delay semantics.

MediaItem adds structured score, quality, year where available, dub/sub episode counts and metadata reference. Use the upstream Score conversion API: Score internally uses a 0..10^9 fixed scale; the old deprecated rating used a different scale. A clear desktop shape such as `{value: 8.4, max: 10}` avoids confusing an integer raw score with a 10-point score.

Preserve all upstream media names: Movie, AnimeMovie, TvSeries, Cartoon, Anime, OVA, Torrent, Documentary, AsianDrama, Live, NSFW, Others, Music, AudioBook, CustomMedia, Audio, Podcast and Video. These are **API categories**, not evidence of a dedicated native audiobook/comic reader. Unknown future values render generically and remain round-trippable. CustomMedia requires an implemented custom action; do not auto-play it as an ordinary movie.

Home rows need stable `rowId`, requested section identity, row name/orientation, items, page/hasNext and section cursor independently. Do not identify a row only by its translated display name.

Details adds backgroundPosterUrl/logoUrl/contentRating/score/status/comingSoon/nextAiring/uniqueUrl/syncData/actors/trailers/recommendations plus structured seasons/dub groups. Episode adds description/date/runtime/score/poster headers/filler source. Optional fields remain absent rather than fake values.

### Streams, resource ownership and players

Resolved streams are a tagged union. Keep upstream type names and separate adapter decisions: VIDEO/direct, M3U8/HLS, DASH, ExtractorLinkPlayList, TORRENT/MAGNET, DRM subtype. A playlist has an ordered timeline, not an empty direct URL. Preserve source/name/quality/referer/headers, extractorData, separate audioTracks and DRM config/resource handles. DRM keys/licenses must not be sprayed into UI events or logs.

Subtitles have resource URL/handle, name/language IETF tag, headers/session context, format, forced/hearing-impaired metadata and source. Headers for one resource do not apply globally to all streams/subtitles/audio. Cookies and interceptors belong to the provider's worker network session. Header maps are opaque values and must preserve commas/backslashes/Unicode literally.

PlayerSession carries sessionId/profileId/MediaRef/EpisodeRef/queueIndex/selected resources. PlayerEvent is authoritative: state, position/duration, pause, volume, speed, tracks, buffering, EOF reason and errors. Commands acknowledge success/failure; UI does not assume success because a click occurred. Display unknown/live duration distinctly.

The existing external mpv backend is the required first implementation. Embedded rendering is an optional architectural project, not necessary for these parity tasks. Browser HTML5 video is not a drop-in replacement for authenticated provider playback or DRM.

### Worker protocol and failure handling

Keep JSON-RPC IDs and stdout-only framed JSON; extension logs use stderr. Proposed capabilities include `protocolVersion`, `upstreamCommit`, methods, runtime modes and resource/session features. Check version at startup and fail incompatibility with a clear message.

A future async envelope needs requestId/sessionGeneration/providerId, success or typed error, and progress/final state. Suggested codes: canceled, timedOut, providerFailed, capabilityUnavailable, authenticationRequired, rateLimited, invalidPackage and workerExited. Keep debug cause separate from user message and redact credentials.

Cancel is an explicit operation, not merely hiding a spinner. Apply cancellation to queued tasks, network requests, verifiers, thumbnails and relay streams. Bound concurrency/timeouts/output sizes; serialize plugin registry mutation safely. Do not let package unload race a running provider/extractor callback.

### Proposed native commands and events

Keep existing commands working until callers migrate. Use one typed API per behavior; these names are a suggested shared vocabulary, to be documented with exact request/response schemas during the owning task. New Tauri plugin permissions go in `D/src-tauri/capabilities/main.json`; do not grant remote webpages the native application interface.

| Owner | Commands / events | Main contract |
|---|---|---|
| T01 | `get_settings`, `update_settings` | Partial validated patch plus scope/profile; response is canonical persisted settings and revision. |
| T04/T08 | `start_search`, `cancel_request`; `source_progress` | Request/generation/provider IDs, independently paginated source results, typed errors and exactly one final state. |
| T10/T12 | `set_watch_status`, `set_favorite`, `set_episode_watched`, `remove_history`; `library_changed` | Stable references plus profile, deterministic bulk operation and updated record. |
| T11 | `list_profiles`, `create_profile`, `update_profile`, `select_profile`, `delete_profile` | Validated profile ID; flush/cancel old-profile work before switching. |
| T13 | `set_subscription`, `check_subscriptions`; `subscription_changed` | Per-profile/media/dub snapshot; deduplicated result/notification. |
| T14–T18 | `player_state`, existing `play`/`player_action` extended; `player_event` | Session ID, acknowledged typed actions and authoritative state. |
| T19 | worker resource/verifier methods; private media transport | Opaque session resource handles. No raw interceptor objects in JSON or arbitrary upstream URL fetch endpoint. |
| T20/T21 | `enqueue_download`, `download_action`, `list_downloads`; `download_progress` | Stable job ID, state machine, bounded events and filesystem ownership. |
| T22/T23 | `list_service_accounts`, `start_service_auth`, `logout_service`, service search/library/sync methods | Adapter capabilities and profile-scoped credential references; redacted errors. |
| T25/T26 | `preview_import`, `apply_import`, `export_backup`, `backup_status` | Preview token/checksum tied to exact validated input; selective atomic application. |
| T27 | deep-link router and MPRIS bridge | Validated intent enters existing typed actions after startup readiness. |
| T28 | `list_cast_devices`, `connect_cast`, `cast_action`; `cast_event` | Backend/device/session identity and actual receiver state. |
| T32 | `diagnostics`, `export_diagnostics`, `check_app_update` | Redacted report, configured desktop release channel and explicit supported installer behavior. |

Do not store transient player/network/job callbacks in persistent records. Source state, account state and offline files have different lifetimes; test app restart and profile switch at each boundary.

## Recommended delivery order


Priorities identify importance, while dependencies determine the actual ready order. P0 does not mean “easy.” A small agent should finish the numbered substeps in a package as separate patches and run its targeted tests before moving on.

| Stage | Work | Result |
|---|---|---|
| 1 | T01 → T02 → T03; then T04, T05 and T14 | Data survives upgrades, richer fields cross boundaries, source jobs/player state become reliable. |
| 2 | T10, T08, T06, T24, T12; then T09, T11, T07, T15, T30 | Daily browsing/search/library/details and extension management approach Android behavior. T09 requires T10; T11 requires T10. |
| 3 | T19 → T17; T16 after T15; T20 after T19; T21 after T17/T20 | Protected playback, tracks, controls, persistent direct/HLS downloads and offline library. T19 is specialist work; do not disguise it as header plumbing. |
| 4 | T25/T31, then T26; T22 and T23 one service adapter per patch; T13/T18/T27/T32 when their dependencies are ready | Transfer data, localization/setup, service accounts, subscriptions and Linux integration. |
| 5 | T28, independent T29 sub-backends, T33 | Device/runtime-specific and specialist compatibility work, with explicit support matrices. |

Recommended first agent assignment: **T01 only**. After its migration and settings foundation is reviewed, T02 is the next assignment. Do not give a small model the instruction “implement all Android features” without bounded task selection.

## Validation and acceptance gates

### Baseline and commands

Run commands in `D/`. Use Node 22+, Rust and Java 21; bundled Java handles packaged execution, but building the worker needs a JDK/jlink. Existing README documents Linux build libraries and mpv. Do not use production user data for fixtures: configure a temporary `CLOUDSTREAM_DATA_DIR` for CLI tests. Tauri app data is independent of CLI development paths; browser tests use a development-only bridge.

```sh
npm ci                         # only when dependencies need installing
npm run check
npm run build
cargo test -p cloudstream-core
(cd runtimes/jvm && ./gradlew test)
bash scripts/smoke-worker.sh
PLAYWRIGHT_CHROMIUM_EXECUTABLE=/usr/bin/chromium npm run test:ui
```

Use the actual Chromium executable path, or install Playwright Chromium once with `npx playwright install chromium`. The browser suite validates UI interactions against fixture bridge responses; it does **not** prove native playback or live extension support.

Native display/mpv check (opens a temporary silent player):

```sh
cargo test -p cloudstream-core --test player_linux -- --ignored
```

Original-package check, when a caller-supplied known package and its required network are available:

```sh
python3 scripts/smoke-legacy.py /absolute/path/to/extension.cs3 --query test
```

This audit reran only the Rust compatibility suite (8 pass; native test ignored) and Svelte check (0 errors/warnings). Earlier development ran the worker/Kotlin/browser/native tests listed above. The Android application itself was **not** built or exercised during this source audit; live third-party services, cast devices and an Android-host backend are unverified.

### Required gates for a task

- Confirm baseline, then change only its scope. Fix regressions caused by the patch; do not mask unrelated baseline failures as success.
- For persistence: old-data upgrade, reopen, failed-transaction rollback and no production-data deletion.
- For protocol: rich fixture round-trip, absent/unknown fields and incompatible version handling. Keep original provider URLs/payload strings exact.
- For source jobs: cancellation, late replies, one unhealthy source, retry and worker restart. Shared extractor behavior remains tested.
- For playback/download: deterministic local HTTP/media fixtures with required headers/cookies, real native smoke where necessary and actual file hashes for transfers. Test normal EOF separately from user stop/error.
- For each new visible action: working success/error/loading/empty states, keyboard access and targeted UI verification. Run existing relevant checks once; repeat/broaden only for new failures or changes.
- For service/device integrations: fixture tests are required; label live smoke separately and never claim configured/connected without a successful actual exchange.
- Completion report: task/substep IDs, changed files, exact checks/results, migrations/capabilities added, known limitations and next ready task. If a specialist decision blocks a substep, preserve working behavior and report the precise blocker instead of implementing a fake success path.

## Settings inventory and stale-source differences

Current primary evidence is shared `DevicePreferenceStore.kt` [A26] plus Compose screens [A27]–[A31], and subtitle screens [A09]. Old XML has 89 entries including category headers and actions. It is useful for migration aliases, but is **not** a list of 89 active switches.

Important differences found during audit:

- `auto_download_plugins_key2` still exists as a stored definition, but shared source explicitly says it is no longer used. The current updates UI comments out auto-install-all mode. Do not restore mass auto-install as parity.
- `pref_filter_search_quality_key2` replaces the older quality-filter key and uses an enum set; import the older shape deliberately.
- Current player settings add `start_paused_key`, `show_name`, `show_resolution`, `show_media_info`. Older combined show-info fields are not sufficient.
- Current appearance settings expose individual `show_hd_key`, `show_dub_key`, `show_sub_key`, `show_rating_key`, `show_title_key`, `show_episode_text_key` rather than a single poster UI action.
- `extra_brightness_key` is the shared current preference; the older XML name differs. TV seek defaults also differ between definitions: use the active shared definition and document desktop defaults instead of copying XML blindly.
- NSFW exists in the media taxonomy and legacy preference/filter code, but the current provider screen does not expose the old standalone NSFW switch. Treat filtering as capability/content preference, not evidence of a current separate toggle.
- Provider language/media preferences appear in both General and Providers screens; they should use one stored value. “Cast in details” means the actors/cast section, not Chromecast permission.
- Search suggestions use TMDB multi-search (minimum two characters, movie/TV titles, deduplicated and capped at ten in this revision); configure suitable credentials rather than copying a hardcoded shared key; this is separate from quickSearch on a provider. Trakt in metadata code is separate from the four registered tracking services.
- ExtractorLinkType comments say TORRENT/MAGNET unsupported, but the app has a TorrServer conversion path in `ui/player/Torrent.kt`; read call sites before inferring behavior from an enum comment. DASH playback exists but its enum comment explicitly says no download support.
- Voting is implemented in legacy PluginDetailsFragment/VotingApi. The active Compose extension detail code exposes package details and plugin settings; no vote control was found there. Voting is optional legacy parity.

### Current shared preference definitions

The table below is extracted from this pinned source, including definitions used outside visible settings screens. An existing key does not prove a setting is wired everywhere. `default` is the source expression; symbolic defaults are explained after the table. On Linux implement the intended behavior with an explicit desktop default, retaining Android aliases only for import.

| Group.property | Stored key | Source default |
|---|---|---|
| `PluginPreferences.autoUpdate` | `auto_update_plugins` | `true` |
| `PluginPreferences.autoDownload` | `auto_download_plugins_key2` | `0` |
| `BackupPreferences.frequency` | `automatic_backup_key` | `0` |
| `BackupPreferences.path` | `backup_path_key` | `(store default)` |
| `BackupPreferences.visualPath` | `backup_dir_key` | `(store default)` |
| `UpdatePreferences.apkInstaller` | `apk_installer_key` | `1` |
| `UpdatePreferences.showAppUpdates` | `auto_update` | `true` |
| `UpdatePreferences.skipUpdate` | `skip_update_key` | `""` |
| `SecurityPreferences.biometrics` | `biometric_key` | `false` |
| `SecurityPreferences.skipAccountSelection` | `skip_startup_account_select_key` | `false` |
| `UIPreferences.primaryColor` | `primary_color_key` | `"Normal"` |
| `UIPreferences.theme` | `app_theme_key` | `"AmoledLight"` |
| `UIPreferences.layout` | `app_layout_key` | `-1` |
| `UIPreferences.bottomTitle` | `bottom_title_key` | `true` |
| `UIPreferences.advancedSearch` | `advanced_search` | `true` |
| `UIPreferences.searchSuggestions` | `search_suggestions_enabled` | `true` |
| `UIPreferences.kitsuPostersEnabled` | `show_kitsu_posters_key` | `true` |
| `UIPreferences.trailersEnabled` | `show_trailers_key` | `true` |
| `UIPreferences.castEnabled` | `show_cast_in_details_key` | `true` |
| `UIPreferences.fillersEnabled` | `show_fillers_key` | `false` |
| `UIPreferences.showMetadataOverlay` | `show_player_metadata_key` | `true` |
| `UIPreferences.overscanDp` | `overscan_key` | `0` |
| `UIPreferences.posterSize` | `poster_size_key` | `0` |
| `UIPreferences.posterShowHd` | `show_hd_key` | `true` |
| `UIPreferences.posterShowDub` | `show_dub_key` | `true` |
| `UIPreferences.posterShowSub` | `show_sub_key` | `true` |
| `UIPreferences.posterShowRating` | `show_rating_key` | `true` |
| `UIPreferences.posterShowTitle` | `show_title_key` | `true` |
| `UIPreferences.posterShowEpisode` | `show_episode_text_key` | `true` |
| `UIPreferences.showClock` | `tv_layout_clock_key` | `false` |
| `UIPreferences.randomButtonEnabled` | `random_button_key` | `false` |
| `UIPreferences.confirmExit` | `confirm_exit_key` | `-1` |
| `UIPreferences.filterQuality` | `pref_filter_search_quality_key2` | `emptySet()` |
| `ProviderPreferences.preferredMedia` | `prefer_media_type_key_2` | `defaultPreferredMedia` |
| `ProviderPreferences.extensionLanguages` | `provider_lang_key` | `setOf(AllLanguagesName)` |
| `ProviderPreferences.displayDubSub` | `display_sub_key` | `defaultDub` |
| `PlayerPreferences.episodeSync` | `episode_sync_enabled_key` | `true` |
| `PlayerPreferences.defaultPlayer` | `player_default_key` | `""` |
| `PlayerPreferences.limitPlayerTitle` | `prefer_limit_title_key` | `0` |
| `PlayerPreferences.hidePlayerControlNames` | `hide_player_control_names_key` | `false` |
| `PlayerPreferences.showName` | `show_name` | `true` |
| `PlayerPreferences.showResolution` | `show_resolution` | `true` |
| `PlayerPreferences.showMediaInfo` | `show_media_info` | `false` |
| `PlayerPreferences.pipEnabled` | `pip_enabled_key` | `true` |
| `PlayerPreferences.resizeEnabled` | `player_resize_enabled_key` | `true` |
| `PlayerPreferences.speedEnabled` | `playback_speed_enabled_key` | `false` |
| `PlayerPreferences.compressorEnabled` | `compressor_enabled_key` | `false` |
| `PlayerPreferences.tiktokEnabled` | `speedup_key` | `false` |
| `PlayerPreferences.autoPlayEnabled` | `autoplay_next_key` | `true` |
| `PlayerPreferences.startPaused` | `start_paused_key` | `false` |
| `PlayerPreferences.skipOpEnabled` | `enable_skip_op_from_database` | `true` |
| `PlayerPreferences.autoRotateEnabled` | `auto_rotate_video_key` | `true` |
| `PlayerPreferences.rotateButtonEnabled` | `rotate_video_key` | `false` |
| `PlayerPreferences.previewBarEnabled` | `preview_seekbar_key` | `true` |
| `PlayerPreferences.softwareDecoding` | `software_decoding_key2` | `-1` |
| `PlayerPreferences.extraBrightnessEnabled` | `extra_brightness_key` | `false` |
| `PlayerPreferences.swipeHorizontalEnabled` | `swipe_enabled_key` | `true` |
| `PlayerPreferences.swipeVerticalEnabled` | `swipe_vertical_enabled_key` | `true` |
| `PlayerPreferences.doubleTapToSeekEnabled` | `double_tap_enabled_key` | `false` |
| `PlayerPreferences.doubleTapToPauseEnabled` | `double_tap_pause_enabled_key` | `false` |
| `PlayerPreferences.doubleTapTime` | `double_tap_seek_time_key2` | `10` |
| `PlayerPreferences.bufferDiskMB` | `video_buffer_disk_key` | `0` |
| `PlayerPreferences.bufferRamMB` | `video_buffer_size_key` | `0` |
| `PlayerPreferences.bufferTimeSec` | `video_buffer_length_key` | `0` |
| `PlayerPreferences.tvSeekOnTime` | `android_tv_interface_on_seek_key` | `10` |
| `PlayerPreferences.tvSeekOffTime` | `android_tv_interface_off_seek_key` | `10` |
| `GeneralPreferences.locale` | `app_locale` | `""` |
| `GeneralPreferences.bananas` | `benene_count` | `0` |
| `GeneralPreferences.parallelDownloads` | `download_parallel_key` | `3` |
| `GeneralPreferences.concurrentConnections` | `download_concurrent_key` | `3` |
| `GeneralPreferences.batterOptimization` | `battery_optimisation_key` | `false` |
| `GeneralPreferences.dns` | `dns_key` | `0` |
| `GeneralPreferences.jsdelivrProxy` | `jsdelivr_proxy_key` | `false` |
| `GeneralPreferences.downloadPath` | `download_path_key` | `""` |
| `GeneralPreferences.downloadPathVisual` | `download_path_key_visual` | `""` |

Symbolic defaults: `defaultPreferredMedia` is all upstream TvType ordinals except NSFW; `defaultDub` is all DubStatus names; `AllLanguagesName` means all extension languages. Do not persist upstream enum ordinals as a new desktop contract. `(store default)` means no default was passed at this call; read the preference-store semantics before importing it.

### Subtitle, player-profile and action preferences outside the shared table

| Feature set | Behavior to implement | Tasks / evidence |
|---|---|---|
| Subtitle fonts/text | Bundled/custom font, size, text color, uppercase, bold, italic, preview, reset/undo | T17; [A09] |
| Subtitle edge/background | Edge type/size/color, window/background colors, background radius, alignment and elevation | T17; [A09] |
| Subtitle language | Auto-select language, download languages, filter by language | T17/T22; [A09] |
| Subtitle cleanup | Remove captions and bloat, optional and reversible | T17; [A09] |
| Cast subtitle style | Font/size/text/edge/background settings sent to a capable receiver | T28; [A09] |
| Source profiles | Named quality/source priorities and associated default source selection | T16; [A10] |
| Player subtitle controls | Track choice, local/remote files, subtitle offset and audio selection | T17/T22; [A07] |
| Setup/action settings | Locale and provider languages, media selection, clone sites, download/backup directories, manual update/test/restore/logging actions | T06/T20/T25/T30/T31/T32; [A27]/[A31]/[A35] |

### Legacy XML inventory for migration and feature lookup

These are the 89 XML entries, including category headers. Titles are the default-language XML strings, not a promise of active navigation or exact desktop labels. Keys can differ from the current shared table above. Source: upstream `app/src/main/res/xml/settings_*.xml`.

**`app/src/main/res/xml/settings_account.xml`**

| Entry type | Key | XML title / default |
|---|---|---|
| Preference | `mal_key` | (dynamic title / no XML title) |
| Preference | `kitsu_key` | (dynamic title / no XML title) |
| Preference | `anilist_key` | (dynamic title / no XML title) |
| Preference | `simkl_key` | (dynamic title / no XML title) |
| Preference | `opensubtitles_key` | (dynamic title / no XML title) |
| Preference | `subdl_key` | (dynamic title / no XML title) |
| Preference | `animeskip_key` | (dynamic title / no XML title) |
| SwitchPreference | `skip_startup_account_select_key` | Skip account selection at startup; default=false |
| PreferenceCategory | `pref_category_security_key` | Security |
| SwitchPreference | `biometric_key` | Lock with Biometrics; default=false |

**`app/src/main/res/xml/settings_general.xml`**

| Entry type | Key | XML title / default |
|---|---|---|
| Preference | `app_locale` | App Language |
| Preference | `legal_notice_key` | Disclaimer |
| Preference | `benene_count` | Give a benene to the devs |
| Preference | `download_path_key` | Download path |
| SeekBarPreference | `download_parallel_key` | Parallel downloads; default=3 |
| SeekBarPreference | `download_concurrent_key` | Concurrent connections; default=3 |
| Preference | `battery_optimisation` | Disable Battery optimization |
| Preference | `override_site_key` | Clone site |
| Preference | `dns_key` | DNS over HTTPS |
| SwitchPreference | `jsdelivr_proxy_key` | GitHub Proxy; default=false |

**`app/src/main/res/xml/settings_player.xml`**

| Entry type | Key | XML title / default |
|---|---|---|
| SwitchPreference | `episode_sync_enabled_key` | Update watch progress; default=true |
| Preference | `player_default_key` | Preferred video player |
| Preference | `prefer_limit_title_key` | Video player title max chars |
| Preference | `prefer_limit_show_player_info` | Show player information |
| SwitchPreference | `hide_player_control_names_key` | Hide names of the player\'s controls; default=false |
| Preference | `subtitle_settings_key` | Subtitles |
| Preference | `subtitle_settings_chromecast_key` | Chromecast Subtitles |
| Preference | `player_source_priority_key` | Source priority |
| SwitchPreference | `pip_enabled_key` | Picture-in-picture; default=true |
| SwitchPreference | `player_resize_enabled_key` | Player resize button; default=true |
| SwitchPreference | `playback_speed_enabled_key` | Playback speed; default=false |
| SwitchPreference | `compressor_enabled_key` | Dynamic Range Compressor; default=false |
| SwitchPreference | `speedup_key` | LongPress Speed Toggle; default=false |
| SwitchPreference | `autoplay_next_key` | Autoplay next episode; default=true |
| SwitchPreference | `enable_skip_op_from_database` | Skip OP; default=true |
| SwitchPreference | `rotate_video_key` | Rotate; default=false |
| SwitchPreference | `auto_rotate_video_key` | Auto rotate; default=true |
| SwitchPreference | `preview_seekbar_key` | Seekbar preview; default=true |
| Preference | `software_decoding_key2` | Software decoding; default=true |
| SwitchPreference | `extra_brightness_enabled` | Extra brightness; default=false |
| PreferenceCategory | `pref_category_gestures_key` | Gestures |
| SwitchPreference | `swipe_enabled_key` | Swipe to seek; default=true |
| SwitchPreference | `swipe_vertical_enabled_key` | Swipe to change settings; default=true |
| SwitchPreference | `double_tap_enabled_key` | Double tap to seek; default=false |
| SwitchPreference | `double_tap_pause_enabled_key` | Double tap to pause; default=false |
| SeekBarPreference | `double_tap_seek_time_key2` | Player seek amount (Seconds); default=10 |
| Preference | `video_buffer_disk_key` | Video cache on disk |
| Preference | `video_buffer_size_key` | Video buffer size |
| Preference | `video_buffer_length_key` | Video buffer length |
| Preference | `video_buffer_clear_key` | Clear video and image cache |
| PreferenceCategory | `pref_category_android_tv_key` | Android TV |
| SeekBarPreference | `android_tv_interface_on_seek_key` | Player Shown - Seek Amount; default=30 |
| SeekBarPreference | `android_tv_interface_off_seek_key` | Player Hidden - Seek Amount; default=10 |

**`app/src/main/res/xml/settings_providers.xml`**

| Entry type | Key | XML title / default |
|---|---|---|
| Preference | `provider_lang_key` | Extension languages |
| Preference | `prefer_media_type_key_2` | Preferred media |
| Preference | `display_sub_key` | Display Dubbed/Subbed Anime |
| SwitchPreference | `enable_nsfw_on_providers_key` | Enable NSFW on supported Extensions; default=false |
| Preference | `test_providers_key` | Test all Extensions |

**`app/src/main/res/xml/settings_ui.xml`**

| Entry type | Key | XML title / default |
|---|---|---|
| Preference | `primary_color_key` | Primary color |
| Preference | `app_theme_key` | App theme |
| Preference | `app_layout_key` | App Layout |
| SwitchPreference | `bottom_title_key` | Poster title location; default=true |
| SeekBarPreference | `overscan_key` | Overscan; default=0 |
| SeekBarPreference | `poster_size_key` | Poster size; default=0 |
| Preference | `poster_ui_key` | Toggle UI elements on poster |
| SwitchPreference | `advanced_search` | Advanced Search; default=true |
| SwitchPreference | `search_suggestions_enabled` | Search Suggestions; default=true |
| SwitchPreference | `show_trailers_key` | Show trailers; default=true |
| SwitchPreference | `show_kitsu_posters_key` | Show posters from Kitsu; default=true |
| SwitchPreference | `show_cast_in_details_key` | Show cast panel; default=true |
| SwitchPreference | `show_fillers_key` | Show filler episode for anime; default=false |
| SwitchPreference | `show_player_metadata_key` | Show Player Metadata Overlay; default=true |
| SwitchPreference | `tv_layout_clock_key` | Show real time clock; default=false |
| SwitchPreference | `random_button_key` | Random Button; default=false |
| Preference | `confirm_exit_key` | Confirm before exiting |
| Preference | `pref_filter_search_quality_key` | Hide selected video quality in search results |

**`app/src/main/res/xml/settings_updates.xml`**

| Entry type | Key | XML title / default |
|---|---|---|
| Preference | `manual_check_update` | Check for Update |
| Preference | `install_prerelease_key` | Install pre-release version |
| Preference | `apk_installer_key` | APK Installer |
| SwitchPreference | `auto_update` | Show app updates; default=true |
| Preference | `backup_key` | Back up data |
| Preference | `automatic_backup_key` | Backup frequency |
| Preference | `restore_key` | Restore data from backup |
| Preference | `backup_path_key` | Backup folder location |
| SwitchPreference | `auto_update_plugins` | Automatic plugin updates; default=true |
| Preference | `auto_download_plugins_key2` | Automatically download plugins |
| Preference | `manual_update_plugins` | Update Plugins |
| Preference | `show_logcat_key` | Show Logcat 🐈 |
| Preference | `redo_setup_key` | Redo setup process |

## Source index and uncertainties

Each source ID below links immutable files at the audited Android revision. Related implementation files are grouped to help the next agent read narrowly. The task JSON repeats these full paths, records current/legacy preference definitions and contains a desktop baseline fingerprint manifest.

This inventory covers user-visible application behavior and extension-facing capabilities found in the source. It is not a byte-for-byte Android clone specification or a claim to enumerate every private helper, dependency extractor, animation or resource string. Follow the linked active handlers when an edge case matters. Android/Compose previews, commented options and unimplemented platform stubs are not working features.

External-service endpoints, credentials, device behavior and Linux system integration need verification at implementation time. No external account login, cast-device test, Android build or universal extension corpus was performed for this audit. Conditional API categories such as CustomMedia, torrent and DRM need an actual backend and representative fixtures. Android biometric, APK installer, WorkManager, scoped storage, TV channels and orientation APIs require platform adaptations, not name-for-name Rust shims.

### A01 — Active navigation and Android entry points

- [`app/src/main/res/navigation/mobile_navigation.xml`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/res/navigation/mobile_navigation.xml)
- [`app/src/main/AndroidManifest.xml`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/AndroidManifest.xml)
- [`app/src/main/java/com/lagradost/cloudstream3/MainActivity.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/MainActivity.kt)

### A02 — Home

- [`app/src/main/java/com/lagradost/cloudstream3/ui/home/HomeViewModel.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/home/HomeViewModel.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/home/HomeFragment.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/home/HomeFragment.kt)

### A03 — Search

- [`app/src/main/java/com/lagradost/cloudstream3/ui/search/SearchViewModel.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/search/SearchViewModel.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/search/SearchFragment.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/search/SearchFragment.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/search/SearchSuggestionApi.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/search/SearchSuggestionApi.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/quicksearch/QuickSearchFragment.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/quicksearch/QuickSearchFragment.kt)

### A04 — Title details and episode behavior

- [`app/src/main/java/com/lagradost/cloudstream3/ui/result/ResultViewModel2.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/result/ResultViewModel2.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/result/ResultFragmentPhone.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/result/ResultFragmentPhone.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/result/EpisodeAdapter.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/result/EpisodeAdapter.kt)

### A05 — Provider API and media models

- [`library/src/commonMain/kotlin/com/lagradost/cloudstream3/MainAPI.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/library/src/commonMain/kotlin/com/lagradost/cloudstream3/MainAPI.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/APIRepository.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/APIRepository.kt)

### A06 — Stream, audio, playlist and DRM models

- [`library/src/commonMain/kotlin/com/lagradost/cloudstream3/utils/ExtractorApi.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/library/src/commonMain/kotlin/com/lagradost/cloudstream3/utils/ExtractorApi.kt)

### A07 — Player orchestration

- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/GeneratorPlayer.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/GeneratorPlayer.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/PlayerGeneratorViewModel.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/PlayerGeneratorViewModel.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/FullScreenPlayer.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/FullScreenPlayer.kt)

### A08 — Player implementation

- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/CS3IPlayer.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/CS3IPlayer.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/IPlayer.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/IPlayer.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/PreviewGenerator.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/PreviewGenerator.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/DynamicRangeCompressor.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/DynamicRangeCompressor.kt)

### A09 — Subtitle preferences and rendering

- [`app/src/main/java/com/lagradost/cloudstream3/ui/subtitles/SubtitlesScreen.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/subtitles/SubtitlesScreen.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/PlayerSubtitleHelper.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/PlayerSubtitleHelper.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/subtitles/ChromecastSubtitlesScreen.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/subtitles/ChromecastSubtitlesScreen.kt)

### A10 — Source and quality preferences

- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/source_priority/QualityDataHelper.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/source_priority/QualityDataHelper.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/source_priority/QualityProfileDialog.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/source_priority/QualityProfileDialog.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/source_priority/SourcePriorityDialog.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/source_priority/SourcePriorityDialog.kt)

### A11 — Skip intro/outro integrations

- [`app/src/main/java/com/lagradost/cloudstream3/utils/videoskip/SkipAPI.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/videoskip/SkipAPI.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/utils/videoskip/AniSkip.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/videoskip/AniSkip.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/utils/videoskip/AnimeSkip.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/videoskip/AnimeSkip.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/utils/videoskip/IntroDbSkip.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/videoskip/IntroDbSkip.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/utils/videoskip/TheIntroDBSkip.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/videoskip/TheIntroDBSkip.kt)

### A12 — Downloads engine

- [`app/src/main/java/com/lagradost/cloudstream3/utils/downloader/DownloadManager.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/downloader/DownloadManager.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/utils/downloader/DownloadFileManagement.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/downloader/DownloadFileManagement.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/utils/downloader/DownloadObjects.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/downloader/DownloadObjects.kt)

### A13 — Downloads queue and interface

- [`app/src/main/java/com/lagradost/cloudstream3/utils/downloader/DownloadQueueManager.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/downloader/DownloadQueueManager.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/download/DownloadFragment.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/download/DownloadFragment.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/download/queue/DownloadQueueFragment.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/download/queue/DownloadQueueFragment.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/download/DownloadChildFragment.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/download/DownloadChildFragment.kt)

### A14 — Offline playback

- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/OfflinePlaybackHelper.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/OfflinePlaybackHelper.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/DownloadFileGenerator.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/DownloadFileGenerator.kt)

### A15 — Library and watch statuses

- [`app/src/main/java/com/lagradost/cloudstream3/ui/library/LibraryViewModel.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/library/LibraryViewModel.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/LocalList.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/LocalList.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/WatchType.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/WatchType.kt)

### A16 — Profiles and locks

- [`app/src/main/java/com/lagradost/cloudstream3/ui/account/AccountScreen.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/account/AccountScreen.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/account/AccountViewModel2.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/account/AccountViewModel2.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/account/AccountSelectActivity2.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/account/AccountSelectActivity2.kt)

### A17 — Persistent Android user data

- [`app/src/main/java/com/lagradost/cloudstream3/utils/DataStoreHelper.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/DataStoreHelper.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/utils/DataStore.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/DataStore.kt)

### A18 — Tracking and subtitle accounts

- [`app/src/main/java/com/lagradost/cloudstream3/syncproviders/AccountManager.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/AccountManager.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/syncproviders/SyncAPI.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/SyncAPI.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/syncproviders/SubtitleAPI.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/SubtitleAPI.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/result/SyncViewModel.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/result/SyncViewModel.kt)

### A19 — Subscriptions

- [`app/src/main/java/com/lagradost/cloudstream3/services/SubscriptionWorkManager.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/services/SubscriptionWorkManager.kt)

### A20 — Backups

- [`app/src/main/java/com/lagradost/cloudstream3/utils/BackupUtils.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/BackupUtils.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/services/BackupWorkManager.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/services/BackupWorkManager.kt)

### A21 — Plugin loading and repositories

- [`app/src/main/java/com/lagradost/cloudstream3/plugins/PluginManager.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/plugins/PluginManager.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/plugins/RepositoryManager.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/plugins/RepositoryManager.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/plugins/Plugin.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/plugins/Plugin.kt)
- [`library/src/commonMain/kotlin/com/lagradost/cloudstream3/plugins/BasePlugin.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/library/src/commonMain/kotlin/com/lagradost/cloudstream3/plugins/BasePlugin.kt)

### A22 — Extension interface

- [`app/src/main/java/com/lagradost/cloudstream3/ui/settings/extensions/ExtensionsScreen.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/extensions/ExtensionsScreen.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/settings/extensions/PluginsViewModel.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/extensions/PluginsViewModel.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/settings/extensions/PluginDetailsFragment.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/extensions/PluginDetailsFragment.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/plugins/VotingApi.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/plugins/VotingApi.kt)
- [`shared/src/commonMain/kotlin/com/lagradost/cloudstream4/viewmodels/ExtensionsViewModel2.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/shared/src/commonMain/kotlin/com/lagradost/cloudstream4/viewmodels/ExtensionsViewModel2.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/settings/extensions/AndroidRepositoryManager.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/extensions/AndroidRepositoryManager.kt)

### A23 — Provider self tests

- [`app/src/main/java/com/lagradost/cloudstream3/ui/settings/testing/TestViewModel2.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/testing/TestViewModel2.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/settings/testing/TestScreen.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/testing/TestScreen.kt)

### A24 — Network helpers

- [`app/src/main/java/com/lagradost/cloudstream3/network/CloudflareKiller.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/network/CloudflareKiller.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/network/DdosGuardKiller.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/network/DdosGuardKiller.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/network/DohProviders.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/network/DohProviders.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/network/RequestsHelper.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/network/RequestsHelper.kt)

### A25 — WebView resolver platform gap

- [`library/src/androidMain/kotlin/com/lagradost/cloudstream3/network/WebViewResolver.android.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/library/src/androidMain/kotlin/com/lagradost/cloudstream3/network/WebViewResolver.android.kt)
- [`library/src/jvmMain/kotlin/com/lagradost/cloudstream3/network/WebViewResolver.jvm.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/library/src/jvmMain/kotlin/com/lagradost/cloudstream3/network/WebViewResolver.jvm.kt)
- [`library/src/commonMain/kotlin/com/lagradost/cloudstream3/network/WebViewResolver.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/library/src/commonMain/kotlin/com/lagradost/cloudstream3/network/WebViewResolver.kt)

### A26 — Current shared preferences

- [`shared/src/commonMain/kotlin/com/lagradost/cloudstream4/DevicePreferenceStore.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/shared/src/commonMain/kotlin/com/lagradost/cloudstream4/DevicePreferenceStore.kt)

### A27 — Current general and provider settings

- [`app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsGeneralScreen.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsGeneralScreen.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsProvidersScreen.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsProvidersScreen.kt)

### A28 — Current appearance settings

- [`app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsUIScreen.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsUIScreen.kt)

### A29 — Current player settings

- [`app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsPlayerScreen.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsPlayerScreen.kt)

### A30 — Current account settings

- [`app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsAccountScreen.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsAccountScreen.kt)

### A31 — Updates and diagnostics

- [`app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsUpdatesScreen.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsUpdatesScreen.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/settings/ApkUpdater.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/ApkUpdater.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/settings/logcat/LogcatDialog.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/logcat/LogcatDialog.kt)

### A32 — External actions

- [`app/src/main/java/com/lagradost/cloudstream3/actions/VideoClickAction.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/actions/VideoClickAction.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/actions/temp/CopyClipboardAction.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/actions/temp/CopyClipboardAction.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/actions/temp/PlayInBrowserAction.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/actions/temp/PlayInBrowserAction.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/actions/temp/ViewM3U8Action.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/actions/temp/ViewM3U8Action.kt)

### A33 — Casting

- [`app/src/main/java/com/lagradost/cloudstream3/utils/CastHelper.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/CastHelper.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/utils/CastOptionsProvider.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/CastOptionsProvider.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/actions/temp/fcast/FcastAction.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/actions/temp/fcast/FcastAction.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/actions/temp/fcast/FcastManager.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/actions/temp/fcast/FcastManager.kt)

### A34 — Live and torrents

- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/live/LiveManager.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/live/LiveManager.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/live/LiveHelper.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/live/LiveHelper.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/Torrent.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/Torrent.kt)

### A35 — Setup

- [`app/src/main/java/com/lagradost/cloudstream3/ui/setup/SetupFragmentLanguage.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/setup/SetupFragmentLanguage.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/setup/SetupFragmentProviderLanguage.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/setup/SetupFragmentProviderLanguage.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/setup/SetupFragmentMedia.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/setup/SetupFragmentMedia.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/setup/SetupFragmentLayout.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/setup/SetupFragmentLayout.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/setup/SetupFragmentExtensions.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/setup/SetupFragmentExtensions.kt)

### A36 — Platform adaptations

- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/PlayerPipHelper.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/PlayerPipHelper.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/ui/player/PlayerGestureHelper.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/PlayerGestureHelper.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/utils/TvChannelUtils.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/TvChannelUtils.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/utils/BiometricAuthenticator.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/BiometricAuthenticator.kt)

### A37 — Metadata providers

- [`app/src/main/java/com/lagradost/cloudstream3/utils/FillerEpisodeCheck.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/FillerEpisodeCheck.kt)
- [`library/src/commonMain/kotlin/com/lagradost/cloudstream3/metaproviders/TmdbProvider.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/library/src/commonMain/kotlin/com/lagradost/cloudstream3/metaproviders/TmdbProvider.kt)
- [`library/src/commonMain/kotlin/com/lagradost/cloudstream3/metaproviders/CrossTmdbProvider.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/library/src/commonMain/kotlin/com/lagradost/cloudstream3/metaproviders/CrossTmdbProvider.kt)
- [`library/src/commonMain/kotlin/com/lagradost/cloudstream3/metaproviders/TraktProvider.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/library/src/commonMain/kotlin/com/lagradost/cloudstream3/metaproviders/TraktProvider.kt)
- [`library/src/commonMain/kotlin/com/lagradost/cloudstream3/metaproviders/MyDramaList.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/library/src/commonMain/kotlin/com/lagradost/cloudstream3/metaproviders/MyDramaList.kt)

### A38 — Upstream contribution and license rules

- [`AI-POLICY.md`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/AI-POLICY.md)
- [`LICENSE`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/LICENSE)

### A39 — Tracking service implementations

- [`app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/AniListApi.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/AniListApi.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/MALApi.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/MALApi.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/KitsuApi.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/KitsuApi.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/SimklApi.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/SimklApi.kt)

### A40 — Subtitle service implementations

- [`app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/OpenSubtitlesApi.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/OpenSubtitlesApi.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/Addic7ed.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/Addic7ed.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/Subdl.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/Subdl.kt)
- [`app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/SubSource.kt`](https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/SubSource.kt)

[A01]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/res/navigation/mobile_navigation.xml
[A02]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/home/HomeViewModel.kt
[A03]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/search/SearchViewModel.kt
[A04]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/result/ResultViewModel2.kt
[A05]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/library/src/commonMain/kotlin/com/lagradost/cloudstream3/MainAPI.kt
[A06]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/library/src/commonMain/kotlin/com/lagradost/cloudstream3/utils/ExtractorApi.kt
[A07]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/GeneratorPlayer.kt
[A08]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/CS3IPlayer.kt
[A09]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/subtitles/SubtitlesScreen.kt
[A10]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/source_priority/QualityDataHelper.kt
[A11]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/videoskip/SkipAPI.kt
[A12]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/downloader/DownloadManager.kt
[A13]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/downloader/DownloadQueueManager.kt
[A14]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/OfflinePlaybackHelper.kt
[A15]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/library/LibraryViewModel.kt
[A16]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/account/AccountScreen.kt
[A17]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/DataStoreHelper.kt
[A18]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/AccountManager.kt
[A19]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/services/SubscriptionWorkManager.kt
[A20]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/BackupUtils.kt
[A21]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/plugins/PluginManager.kt
[A22]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/extensions/ExtensionsScreen.kt
[A23]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/testing/TestViewModel2.kt
[A24]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/network/CloudflareKiller.kt
[A25]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/library/src/androidMain/kotlin/com/lagradost/cloudstream3/network/WebViewResolver.android.kt
[A26]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/shared/src/commonMain/kotlin/com/lagradost/cloudstream4/DevicePreferenceStore.kt
[A27]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsGeneralScreen.kt
[A28]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsUIScreen.kt
[A29]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsPlayerScreen.kt
[A30]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsAccountScreen.kt
[A31]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/settings/SettingsUpdatesScreen.kt
[A32]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/actions/VideoClickAction.kt
[A33]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/CastHelper.kt
[A34]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/live/LiveManager.kt
[A35]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/setup/SetupFragmentLanguage.kt
[A36]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/ui/player/PlayerPipHelper.kt
[A37]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/utils/FillerEpisodeCheck.kt
[A38]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/AI-POLICY.md
[A39]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/AniListApi.kt
[A40]: https://github.com/recloudstream/cloudstream/blob/5024ace2922a148d38df2a816b483b529789cf58/app/src/main/java/com/lagradost/cloudstream3/syncproviders/providers/OpenSubtitlesApi.kt
