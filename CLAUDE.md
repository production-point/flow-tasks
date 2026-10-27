# Flow Tasks — Claude Code Project Memory

## What this is

Desktop companion app for FLOW's task list. Tauri 2 (Rust) shell, React 19 + Vite 8 + TanStack Query 5 + Zustand 5 + Tailwind 4 inside. Slim always-on-top panel (350×500, decorationless, transparent) that reads/writes against `FLOW /api/v1/tasks`.

- **Repo:** `production-point/flow-tasks` (private)
- **Local path:** Windows `C:\Claude\flow-tasks`; Mac `/Users/ben/Projects/flow-tasks`
- **Current version:** see `package.json` (0.5.0 at time of this note: drafts + FLOW reminders, unreleased; last tag v0.3.6)
- **Default branch:** `main`
- **Release branch:** none — tag-driven via `scripts/release.cjs`
- **Platforms:** Windows **and macOS** (Apple Silicon). One Tauri codebase; CI (`.github/workflows/build.yml`) is a `windows-latest` + `macos-14` matrix.

## Architecture at a glance

```
src/
  App.tsx                 shell, window persistence, notifications, update check
  components/             TitleBar, TaskList, TaskRow, QuickAdd, FilterChips,
                          SearchBar, Settings, TaskEditor, OfflineIndicator,
                          MailConnectorSettings (macOS; status dot in TitleBar)
  hooks/                  useTasks, useProjects, useLabels, useUsers, useSettings,
                          useMailConnector
  lib/
    api.ts                Tauri HTTP client — Bearer auth, browser fetch fallback
    nlp-parser.ts         natural-language task input (has tests)
    offline-queue.ts      replays on window focus
    update-checker.ts     GitHub releases API poller (to be replaced by updater plugin)
    window-bounds.ts      monitor-intersection helper (has tests)
    mail-connector.ts     connector command wrappers + status helpers (has tests)
    flow-nudges.ts        FLOW's Mac reminders: poll, mode switch, guard (has tests)
    date-utils.ts, types.ts, version.ts
  index.css               Tailwind import + CSS vars (--flow-accent etc.)
src-tauri/
  src/lib.rs              plugin registration, global hotkey, show_and_focus helper
  src/tray.rs             system tray, right-click menu, left-click toggle
  src/commands.rs         toggle_pin, store/get/delete_api_key (keyring),
                          mail-connector key helpers
  src/mail_connector/     macOS Apple Mail → FLOW connector (see below):
    mod.rs                shared types, ScriptRunner, list/read fns, mailbox exclusions
    jxa.rs                JXA script text, OsascriptRunner, osascript error classes
    parse.rs              headers/addresses → FLOW wire messages
    state.rs              config + marks JSON files (atomic writes)
    drafts.rs             the draft script (the only write path) + outcome codes
    flow_client.rs        FlowApi trait + HTTP client (reqwest, blocking)
    runner.rs             run_once (one tick), the 5-min loop thread, status
    commands.rs           mail_connector_* Tauri commands
  tauri.conf.json         window config (decorations:false, transparent:true),
                          bundle.macOS signing/entitlements
  Entitlements.plist      hardened-runtime entitlement: automation.apple-events
  Info.plist              merged into the bundle: NSAppleEventsUsageDescription
  capabilities/default.json
scripts/
  release.cjs             bump + commit + tag + push; CI builds installer
  bump-version.cjs        syncs version across package.json / Cargo.toml / tauri.conf.json
  generate-icons.cjs
.github/workflows/build.yml  Windows + macOS CI; PRs into main build only, tags publish the release
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
- **Distribution is signed but not notarised:** releases are signed with one stable **self-signed** code-signing certificate (see "Signing" below), not an Apple Developer ID, so there is no notarisation. The first browser download is still Gatekeeper-quarantined → **System Settings → Privacy & Security → Open Anyway** once (macOS 15+ dropped the right-click → Open bypass; older macOS: right-click → Open), or run `xattr -dr com.apple.quarantine "/Applications/Flow Tasks.app"`. **Auto-updates are unaffected** — updates the app downloads itself aren't quarantined. Moving to a Developer ID later = new cert in the same `APPLE_*` secrets (+ notarisation secrets); no app-code change, but it changes the code identity once (see Signing).
- **Signing (stable identity — why it matters):** macOS ties the TCC "control Mail" (Automation) permission to the app's *designated requirement*. Ad-hoc signing (`signingIdentity: "-"`) makes the requirement a cdhash, so every update is a "new app" and the Mail permission is forgotten. One self-signed certificate (common name "Flow Tasks Signing", ~10 years) keeps the requirement stable, so **the Mail permission survives updates**. What it does *not* promise: the login-keychain items (tasks key, connector key) may still ask once per update, because a self-signed identity has no Team ID for the keychain to trust across versions. Click **"Always Allow"**. (The connector reads its key once per 5-min tick. The UI's "has a key" is an attributes-only keychain search via `security-framework`, which never decrypts the secret, cached once per launch.) CI prints `codesign -d -r-` (the designated requirement) after each macOS build: if it's identical across releases, the identity is stable.
  - CI secrets: `APPLE_CERTIFICATE` (base64 .p12 with the private key), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` (the cert's common name). The `macos-14` leg imports the .p12 into a temporary keychain under `$RUNNER_TEMP`, trusts the cert for code signing (`sudo security add-trusted-cert … -p codeSign`), checks `security find-identity -v -p codesigning`, and writes only `APPLE_SIGNING_IDENTITY` to `$GITHUB_ENV` — the bundler reads it and overrides `"-"`. `APPLE_CERTIFICATE` is deliberately not passed to tauri-action (it would re-import into its own keychain, where a self-signed cert can't be trusted non-interactively). The keychain is deleted after the build.
  - **Fallback:** without `APPLE_CERTIFICATE` the build is ad-hoc signed and the step prints an `Ad-hoc signing` warning (the Mail permission will be lost after that update, and the keychain will ask again). A local `npm run tauri build` is ad-hoc unless `APPLE_SIGNING_IDENTITY` is set in the shell (and that identity is in your keychain).
  - `bundle.macOS`: `hardenedRuntime: true` (Tauri's default anyway — v0.3.x was already `adhoc,runtime`), `entitlements: ./Entitlements.plist` (`com.apple.security.automation.apple-events` only; WKWebView's JIT runs in Apple's WebContent process, so no `allow-jit`). `src-tauri/Info.plist` is merged automatically by the CLI and holds `NSAppleEventsUsageDescription`.
  - **Never regenerate the certificate** — a new cert is a new identity (one more re-allow for every user). Keep the .p12 + password somewhere safe outside the repo.

### Mail connector (macOS)

- **What:** reads new mail from the Apple Mail accounts the user ticks in Settings and posts it to FLOW's connector API, so FLOW can see mail from accounts it isn't connected to directly. Code in `src-tauri/src/mail_connector/`, UI in `MailConnectorSettings.tsx`.
- **Read-only** against Mail, except `drafts.rs`: JXA via `osascript` (no move/delete/flag/send). The draft script may only `reply` (no window) or make an invisible outgoing message, set content/subject/recipients/sender, and `save`; a test pins that allowlist. **Never launches Mail** — every script checks `Mail.running()` first; if Mail is closed it heartbeats and reads nothing.
- **Only ticked accounts.** Accounts on the FLOW domain (`productionpoint.co.uk`) show as "Already connected directly" and are never selected or registered. Per-account refusals from FLOW don't fail the other accounts.
- **Excluded mailboxes:** FLOW's list (Junk Email, Deleted Items, Drafts, Outbox, Conversation History, Sync Issues, Conflicts, Local/Server Failures, Journal, Tasks, Notes, the `[Gmail]/…` + `[Google Mail]/…` All Mail/Important/Starred/Spam/Bin/Trash/Drafts) plus Junk, Trash, Deleted Messages, Spam, Bin; any sub-path of these too (`is_excluded_mailbox`).
- **Loop:** a dedicated Rust thread (`runner::start_loop`), first run 15 s after launch, then every **5 minutes** (single-flight; "Run now" from Settings). Per account per run: ≤2,000 messages or 60 s, then "catching up" next tick. Chunks ≤200, batches ≤100 msgs / ≤8 MB.
- **FLOW's marks win:** on each registration FLOW returns `resumeFrom` per mailbox; that beats local marks (re-read from 10 min before it; FLOW dedupes). Local compound cursor (dateReceived ms + Mail id) only resumes a mailbox that was mid-catch-up. Null → last 60 days.
- **Key:** its own FLOW connector key in the keychain, service `flow-tasks`, account `flow-tasks-mail-connector` (separate from the tasks API key).
- **State files** (app data dir, atomic writes): `mail-connector-config.json` (FLOW URL, selected accounts, connectionIds, sent-today counts) and `mail-connector-marks.json` (per-mailbox read cursors). No key, no message content.
- **Privacy:** no message content (subjects, senders, bodies, headers, recipients, attachment names) is ever logged, written to disk or shown — only counts, states and error classes.
- Off macOS the commands return "Mail connector is available on macOS"; everything else compiles cross-platform.

### Drafts and FLOW reminders (v0.5.0)

- **Drafts:** each tick, per connected account and before reading, `GET /connector/draft-requests`, save ≤10 (inside the 60 s), post results in one call. Reply finds the original by Message-ID (case/brackets ignored) in the named mailbox, then the account's other readable ones. Mail quitting or refusing Automation before a save isn't reported (FLOW re-hands the request); a saved result FLOW didn't take is kept in memory (`ConnectorRuntime`) and re-posted without saving again. Codes only: `not-found`, `timeout`, `error:<n>`.
- Each saved draft leaves an invisible outgoing message in Mail until Mail quits. `close` (JXA or AppleScript, `saving: no`) is a silent no-op on an invisible outgoing message (measured 27 Oct 2026), so it isn't on the allowlist.
- **Reminders:** `App.tsx` polls `GET /api/v1/assistant/nudges` with the tasks key every 5 min. `enabled: true` → FLOW's one bundled notification replaces the local overdue/due-today pings (body ends "Open FLOW to see it": the notification plugin has no click actions on desktop) and `delivered` is posted; `enabled: false`, a 4xx or no key → local pings; unreachable/5xx → previous mode for 30 min. Guard: keys shown today, persisted.

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

- **2026-10-26 — Mail connector + stable macOS signing (v0.4.0, unreleased, production-point/flow-tasks#2):**
  - `src-tauri/src/mail_connector/` — read ticked Apple Mail accounts via JXA, post to FLOW every 5 min; Settings section + title-bar status dot (see "Mail connector (macOS)").
  - Signing: stable self-signed identity from CI secrets with an ad-hoc fallback + warning; `Entitlements.plist` (apple-events), `Info.plist` (`NSAppleEventsUsageDescription`), `hardenedRuntime: true`.
  - CI: `pull_request` into `main` builds both legs without publishing (release inputs empty unless the ref is a tag); `cargo test` on the `macos-14` leg; signature + entitlements printed after the build.
  - Rollout: the first update *onto* the stable identity changes the code identity once, so users re-allow "control Mail" one more time; after that, updates keep the Mail permission. The keychain may still ask once per update ("Always Allow"): a self-signed identity has no Team ID.
  - 2026-10-27 review fixes: only osascript's trailing error code classifies a failure; a whole-post 400 on `/accounts` retries account by account; a property read that fails (anything but -1728 "missing") stops the chunk *before* that message; a rejected message is skipped only once a later one is accepted (3 in a row, or trailing ones, stop the account; the cursor keeps only what FLOW took); the FLOW URL is re-validated each run (`bad-flow-url` status); has-key is an attributes-only keychain search.

## In flight

- **v0.4.0 mail connector** — production-point/flow-tasks#2 from `feat/mail-connector`. Before tagging: Ben creates the "Flow Tasks Signing" certificate and adds the three `APPLE_*` secrets (steps in the PR body), so v0.4.0 is the first release with the stable identity.
- Then: aesthetic work ("Production Console" direction) or whatever's next.

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
