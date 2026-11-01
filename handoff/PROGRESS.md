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

## Next Ready Tasks
- **[T02]** Stable media/episode identity and progress migration (Depends on: T01)
