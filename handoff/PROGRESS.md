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

## Next Ready Tasks
- **[T03]** Complete typed provider/media/stream bridge (Depends on: T01, T02)
