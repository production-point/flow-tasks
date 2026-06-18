# Flow Tasks — Claude Code Project Memory

## What this is

Desktop companion app for FLOW's task list. Tauri 2 (Rust) shell, React 19 + Vite 8 + TanStack Query 5 + Zustand 5 + Tailwind 4 inside. Slim always-on-top panel (350×500, decorationless, transparent) that reads/writes against `FLOW /api/v1/tasks`.

- **Repo:** `production-point/flow-tasks` (private)
- **Local path:** Windows `C:\Claude\flow-tasks`; Mac `/Users/ben/Projects/flow-tasks`
- **Current version:** see `package.json` (was 0.2.1 at time of this note)
- **Default branch:** `main`
- **Release branch:** none — tag-driven via `scripts/release.cjs`
- **Platforms:** Windows **and macOS** (Apple Silicon). One Tauri codebase; CI (`.github/workflows/build.yml`) is a `windows-latest` + `macos-14` matrix.

## Architecture at a glance

```
src/
  App.tsx                 shell, window persistence, notifications, update check
  components/             TitleBar, TaskList, TaskRow, QuickAdd, FilterChips,
                          SearchBar, Settings, TaskEditor, OfflineIndicator
  hooks/                  useTasks, useProjects, useLabels, useUsers, useSettings
  lib/
    api.ts                Tauri HTTP client — Bearer auth, browser fetch fallback
    nlp-parser.ts         natural-language task input (has tests)
    offline-queue.ts      replays on window focus
    update-checker.ts     GitHub releases API poller (to be replaced by updater plugin)
    window-bounds.ts      monitor-intersection helper (has tests)
    date-utils.ts, types.ts, version.ts
  index.css               Tailwind import + CSS vars (--flow-accent etc.)
src-tauri/
  src/lib.rs              plugin registration, global hotkey, show_and_focus helper
  src/tray.rs             system tray, right-click menu, left-click toggle
  src/commands.rs         toggle_pin, store/get/delete_api_key (keyring)
  tauri.conf.json         window config (decorations:false, transparent:true)
  capabilities/default.json
scripts/
  release.cjs             bump + commit + tag + push; CI builds installer
  bump-version.cjs        syncs version across package.json / Cargo.toml / tauri.conf.json
  generate-icons.cjs
.github/workflows/build.yml  Windows CI; tauri-action publishes release on tag
```

## Commands

```bash
npm install              # bootstrap
npm run dev              # vite dev server (browser only)
npm run tauri dev        # full Tauri dev shell
npm run build            # tsc + vite build
npm test                 # vitest run
npx vitest run <name>    # single test
cd src-tauri && cargo check     # Rust typecheck
cd src-tauri && cargo build --release

# Release (patch bump, tag, push, CI builds installer)
node scripts/release.cjs
node scripts/release.cjs minor
node scripts/release.cjs major

# Watch a CI run
gh run list --repo production-point/flow-tasks --limit 1
gh run view <run-id> --repo production-point/flow-tasks --log-failed

# Releases
gh release list --repo production-point/flow-tasks
gh release view <tag> --repo production-point/flow-tasks
```

## Key design decisions

- **API key in OS keychain** (Windows Credential Manager / macOS login keychain via the `keyring` crate), NOT in localStorage. Surviving a full webview wipe is the feature. The crate is declared per-platform in `Cargo.toml` (`windows-native` vs `apple-native`); `commands.rs` uses the generic `keyring::Entry` API unchanged.
- **Tauri HTTP plugin for all requests** — bypasses webview CORS. Falls back to `globalThis.fetch` when not in Tauri context (dev browser).
- **Window state persisted via Zustand `persist`** → localStorage inside the webview. Stale monitor coordinates are now clamped (see "Completed work" below).
- **FLOW brand colour:** `#3972C5` (currently the `--flow-accent`). Light theme, Todoist-inspired.

### macOS specifics

- **Form factor (hybrid):** menu-bar dropdown by default, tear-off to a pinned floating window. The app runs as an **Accessory** (no Dock/⌘-Tab icon — set in `lib.rs` setup, macOS only). Left-clicking the menu-bar icon anchors the panel beneath it via `tauri-plugin-positioner` (`Position::TrayBottomCenter`); the tray rect is cached through `positioner::on_tray_event` in `tray.rs`.
- **Pin = tear-off:** the `toggle_pin` command writes a shared `PinState` (`AtomicBool`) **and** sets always-on-top. A macOS `WindowEvent::Focused(false)` handler hides the window **only when not pinned**, so a torn-off floating panel stays put while a dropdown auto-hides on blur. `App.tsx` routes the pin toggle *and* the startup restore through `invoke("toggle_pin")` so the Rust flag never drifts from the persisted `isPinned`. Windows is unchanged (explicit tray-toggle + hotkey, no hide-on-blur).
- **Global hotkey:** Cmd+Option+T on macOS (`Modifiers::SUPER | ALT`), Ctrl+Alt+T elsewhere — both registered natively in `lib.rs`. The `settings.hotkey` string is display-only; keep it in sync.
- **Distribution is unsigned (for now):** no Apple Developer ID / notarization. The arm64 binary is ad-hoc-signed at link time so it runs, but the first browser download is Gatekeeper-quarantined → **right-click the app → Open** once (or `xattr -dr com.apple.quarantine "/Applications/Flow Tasks.app"`). **Auto-updates are unaffected** — updates the app downloads itself aren't quarantined. Adding signing later = install an Apple Developer cert + set `APPLE_*` secrets on the `macos-14` CI leg; no app-code change.

## Current UX gaps / known rough edges

- **Design is competent but generic** — reads like any Linear/Todoist/Notion clone. Aesthetic refinement was discussed ("Production Console" direction — warm-tungsten dark, amber accent, JetBrains Mono for numeric metadata, completion-gesture delight, quick-add as terminal prompt). Paused pending auto-update work.
- Emoji `🔍` `🔁` mixed with stroke SVG icons — jarring visual grammar.
- No delight moment on task completion — binary toggle, zero motion.
- Priority P1–P4 are structurally identical (just different dot colour) — no hierarchy of attention.
- ~~`update-checker.ts` is a nag banner, not actual auto-update~~ — fixed 2026-04-14 pending signing key (see "In flight" below).

## Completed work (recent)

- **2026-04-14 — window-rescue fix (commit `a7d7ad3`, shipped in v0.2.1):** saved window coordinates now validated against currently-attached monitors via `src/lib/window-bounds.ts` (+10 tests). Off-screen coords are discarded and the window recenters. Rust `show_and_focus(window)` helper: unminimizes + recenters if off-screen + shows + focuses; used by global hotkey (always a rescue, never hides) and tray right-click menu. Tray gained a right-click Show/Hide/Quit menu as the guaranteed escape hatch.
- **2026-04-14 — release script fix (commit `94ecea0`):** renamed `scripts/release.js` / `bump-version.js` to `.cjs` because `package.json` has `"type": "module"`.
- **2026-04-14 — v0.2.1 released:** first installer published by CI (previous v0.1.0 run failed pre-permissions-fix). `Flow.Tasks_0.2.1_x64-setup.exe` on releases page.

- **2026-04-14 — auto-update via `tauri-plugin-updater` (v0.3.0):**
  - `tauri_plugin_updater::Builder::new().build()` registered in `lib.rs`.
  - `plugins.updater` block in `tauri.conf.json` — endpoint `https://github.com/production-point/flow-tasks/releases/latest/download/latest.json`, `dialog: false`, real minisign pubkey (key ID `937B6041C0392019`).
  - `bundle.createUpdaterArtifacts: true` so `tauri-action` emits `latest.json` + `.sig` alongside the installer.
  - `"updater:default"` in `capabilities/default.json`.
  - `src/lib/update-checker.ts` wraps plugin's `check()` / `download()` / `install()`.
  - Banner UX in `App.tsx`: silent background download on startup (`Downloading v0.x.y — 42%`), then flips to `Install & restart` button; separate error state; all states dismissible.
  - CI passes `TAURI_SIGNING_PRIVATE_KEY` + `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` from GitHub secrets to `tauri-action`.
  - Signing key artefacts: private key lives at `~/.tauri/flow-tasks.key` (password-protected; password stored by user outside repo). Public key embedded in `tauri.conf.json` as base64 of the `.pub` file. If the private key or password is ever lost, a new keypair must be generated and all existing users will have to manually reinstall once to pick up the new pubkey.
  - Rollout: v0.2.1 installs do NOT have the plugin — those users must manually download v0.3.0 once. From v0.3.0 onwards every tagged release auto-installs on existing users' next launch.

## In flight

- Nothing. Ready to pick up aesthetic work ("Production Console" direction) or whatever's next.

## Rules / style

- All Rust changes must pass `cargo check` before commit.
- All TS changes must pass `npx tsc --noEmit` and `npx vitest run` before commit.
- Never commit the Tauri signing private key (or any `.key` file). Private key lives outside the repo (e.g. `~/.tauri/flow-tasks.key`); public key is safe in `tauri.conf.json`.
- `package.json` is `"type": "module"` — any Node scripts must be `.cjs` or use ESM syntax.
- Prefer editing existing files. Don't add README / *.md files unless requested.
- Co-author trailer on commits: `Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>`

## User preferences

- Ship first, polish later — prefers to unblock the user with a working installer, then iterate.
- Clear concise summaries; wants the "why" not the "what."
- Ask before major architectural moves.
- Track completed work in this file for future sessions.
