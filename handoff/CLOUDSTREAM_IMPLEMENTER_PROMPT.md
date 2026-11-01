# Prompt for the implementing agent

You are continuing an existing Linux Cloudstream desktop app. Use Rust + Tauri 2 + Svelte 5 and its Kotlin compatibility worker. The user approved its current Cloudstream-inspired desktop design. Preserve existing extension usability.

Read `CLOUDSTREAM_ANDROID_PARITY_HANDOFF.md`: first “Scope, evidence and baseline,” “Rules for the implementing agent,” and “Data model and protocol contracts.” The source is in the parent directory (`../`) of this `handoff/` folder. Android evidence is pinned to commit 5024ace2922a148d38df2a816b483b529789cf58; immutable file links are in the source index. Local reference checkout, if present, is `../reference/cloudstream-android` relative to this handoff folder.

Your first assignment is **T01 only**, unless the user explicitly chooses another ready task. Read its Android evidence and desktop entry points; do not read/rewrite the whole project. For later work, select one task whose dependencies are completed from `CLOUDSTREAM_PARITY_TASKS.json`. Treat each numbered step of a large task as a separate reviewable patch. High/specialist tasks need a real supported implementation, not a placeholder.

Before editing, inspect repository instructions and current changes, then establish the relevant baseline. Existing desktop files may be untracked in this workspace; do not assume an empty diff means there is no work to preserve. Never delete/reset user code or production data. Use temporary databases for tests.

Make the selected change end-to-end, including typed models, persistence migrations, worker/Tauri/UI boundary where affected, error states and task-specific acceptance checks. Preserve exact provider URLs and opaque episode data. Keep repository/provider identities stable. Do not upgrade the pinned upstream dependency as an unrelated change.

Do not require existing extension authors to rewrite their plugins in Rust. The present bridge supports JAR and some unchanged data-only CS3, not full Android. Keep honest diagnostics for Android UI/resources/WebView dependencies. Shared extractor registrations, atomic package update rollback and restart restoration must keep working.

Use deterministic fixture tests for the selected behavior, then run the existing relevant checks. Browser fixture tests alone cannot prove native playback, actual downloads or live providers. Never add production mock results, no-op controls or fake connected states. Do not claim support for a third-party service/device/runtime without representative validation.

At completion report: (1) task and completed substeps, (2) files and behavior changed, (3) migrations/protocol changes, (4) exact tests/results, (5) remaining limitations and next ready task. Update a separate progress record with task IDs; preserve the audit baseline. Do not claim the entire parity project is done when one task passes. Do not publish or submit upstream PRs unless separately requested.
