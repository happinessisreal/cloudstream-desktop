# Release checklist

What has been checked, on what, and what still needs a clean machine. Update this file whenever a
row changes; do not claim a distribution or architecture that was not actually run.

## Environments actually used for development checks

| Environment | State |
|---|---|
| Arch Linux, x86_64, Wayland session, mpv 0.41, OpenJDK 21+ | All automated checks below run here |

No other distribution, architecture, display server or clean machine has been tested.

## Automated checks (run on every change)

- `cargo test -p cloudstream-core` — core logic, real headless mpv, a private D-Bus session bus.
- `cd runtimes/jvm && ./gradlew test` — worker and media transport.
- `npm run check` and `npm run test:ui` — Svelte types and the browser UI contract tests (WCAG 2 A/AA scans included).
- `bash scripts/smoke-worker.sh` — install, search, details and stream resolution through the real worker.
- `python3 scripts/license-inventory.py --check` — `THIRD_PARTY_LICENSES.md` is current and has no unverified licence.
- `desktop-file-validate` on the rendered `src-tauri/linux/cloudstream-desktop.desktop`.

## Before publishing a build (not yet done)

Each item needs a person on a clean machine; mark it with the date, distribution and architecture.

- [ ] `.deb` installs on a fresh supported distribution with only declared dependencies; `mpv` and `libnotify-bin` are pulled in.
- [ ] Launch with **mpv missing**: the app starts and explains that playback needs mpv (no crash).
- [ ] Launch with **no Java**: the app starts, explains that extensions need Java 21+, and browsing local data still works.
- [ ] Bundled resources are found when installed to `/usr` and when run portably from another folder.
- [ ] Starts offline: library, history and downloads list are usable; network features report clear errors.
- [ ] X11 session and Wayland session: window, fullscreen, file chooser and notifications behave.
- [ ] Upgrade from the previous version keeps library, history, settings, profiles and downloads (the version-1 database fixture is covered by automated migration tests; a real upgrade of an installed copy is not).
- [ ] Links: `cloudstreamsearch://hello` and "open with" a local video reach the running app and ask before acting.
- [ ] Media keys / `playerctl` control playback on the desktop actually used.
- [ ] A new-episode notification with a real notification daemon shows an **Open** action and opens the title.
- [ ] Update channel: with the release signing key and feed address configured, a signed feed is accepted, a tampered one is refused, and no update is ever installed by the app.
- [ ] Diagnostics export contains no addresses with tokens, cookies, PINs or e-mail addresses (spot-check a real session).
- [ ] `THIRD_PARTY_LICENSES.md` regenerated; the Java runtime's `legal/` folder and corresponding source offers are included in the package.

## Not covered yet

- AppImage: built by Tauri with its own desktop file, so link/file handlers are only guaranteed for the `.deb`.
- Any CPU architecture other than x86_64.
- Signed packages / repository metadata: no signing key exists for this project yet.

## Upstream contributions

No pull request to the upstream Cloudstream project is part of this work. Any future upstream contribution must follow the
upstream project's AI-use policy (`AI-POLICY.md` at the pinned commit) and disclose AI assistance as it requires.
