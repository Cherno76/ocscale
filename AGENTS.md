# AGENTS.md — Tokenscope / OCScale

Menu-bar/tray app for monitoring OpenCode token usage. **Tauri 2 + React 18 (TS) + Rust**.

## Commands

```bash
pnpm install                     # must use pnpm (pnpm 10 in CI)
pnpm tauri dev                   # full app (requires Rust toolchain)
pnpm dev                         # frontend-only preview at http://localhost:1420 (mock data)
pnpm tauri build                 # production .app/.dmg (macOS) or .exe (Windows)
cargo test -p tokenscope         # Rust unit tests (src-tauri/src/lib.rs inline)
# regenerate dev mock snapshot:
cargo run --example dump > public/dev-dashboard.json  # in src-tauri/
```

- `pnpm build` runs `tsc && vite build` — typecheck comes first, then bundle.
- `strictPort: true` on port 1420 — nothing else should bind that port.
- There is **no lint/formatter config** (no ESLint, Prettier, or pre-commit hooks).

## Architecture

```
OpenCode SQLite DB (~/.local/share/opencode/opencode.db)
    ↓ (store.rs — query_events → RawEvent[], compare-by-value for change detection)
parser.rs::build_dashboard()          ← serialised by BUILD_LOCK (Mutex)
    ├── config.rs                     → MCP/Skill whitelists (CURRENTLY EMPTY — not yet wired)
    └── pricing.rs::Pricing::shared() → Arc memoized, loaded off-main-thread, refreshed every 24h
    ↓
model.rs (Dashboard → serde JSON)
    ↓ Tauri command get_dashboard()
src/data.ts (fetchDashboard → auto-detects Tauri vs browser dev mock)
    ↓
App.tsx + charts.tsx (React, custom SVG charts — no chart library)
```

**Frontend files** (5): `src/App.tsx`, `charts.tsx`, `data.ts`, `i18n.ts`, `main.tsx`.

**Rust files** (7): `lib.rs` (app setup, tray, commands, 100M celebration), `store.rs` (SQLite→RawEvent), `parser.rs` (aggregation), `pricing.rs` (price loading/cost), `model.rs` (serializable structs), `config.rs` (user MCP/Skill — placeholder), `main.rs` (entry).

## Data sources & pricing

- **Primary**: OpenCode SQLite database at `$XDG_DATA_HOME/opencode/opencode.db` or `~/.local/share/opencode/opencode.db`.
- **Pricing**: models.dev API → LiteLLM → built-in snapshot (`src-tauri/snapshots/litellm.json`). Cached at `~/Library/Caches/tokenscope/` (macOS) / platform cache dir, refreshed every 24h.
- **MCP/Skill tracking**: NOT YET IMPLEMENTED for OpenCode. `config.rs` returns empty whitelists. The `mcp`/`skills` fields in `RawEvent` are always empty. User config reading from OpenCode config is future work.
- Price matching: exact model name → normalized (strip vendor prefix after `/`, `.`↔`p`). Unmatched models still count tokens but show "no price" label.
- OpenCode's per-message `cost` field is used as a fallback when the pricing module doesn't recognise a model.

## Key gotchas

- **`tauri.conf.json` productName is `"OCScale"`**, but npm package name is `tokenscope`, identifier is `com.tokenscope.app`. Be careful not to rename one without the others. The tray tooltip also says "OCScale".
- **`tauri-nspanel`** is a macOS-only git dependency (`ahkohd/tauri-nspanel` branch `v2`). Any code importing it must be `#[cfg(target_os = "macos")]`. It uses the deprecated `objc` crate — the `allow(deprecated)` in `lib.rs:2` is intentional.
- **`macos-private-api`** Tauri feature is enabled for tray operations.
- **macOS tray label**: shown next to the icon via `set_title()`. **Windows**: tray has no label — uses `set_tooltip()` instead. Both must be updated together.
- **NSPanel**: the macOS window is converted to a non-activating NSPanel (level 25 = NSMainMenuWindowLevel + 1) so it floats over fullscreen apps without stealing focus. Panel hides on resign-key, Space change, and app activation.
- **BUILD_LOCK**: `parser::build_dashboard()` is the single entry point for all data. It holds a Mutex — call it from `spawn_blocking`, never inline on the async runtime.
- **30s background refresh**: the polling loop in `lib.rs:943-947` calls `refresh()` (build_dashboard + emit + tray update). The tray also refreshes on panel open via `get_dashboard` command.
- **System theme**: macOS NSPanel doesn't reliably get `prefers-color-scheme`, so an `AppleInterfaceThemeChangedNotification` observer pushes `system-is-dark` to the frontend. On non-macOS, the webview's native `prefers-color-scheme` should work.
- **Unsigned builds**: no code signing or notarization. macOS users must right-click→Open or `xattr -cr`. Windows gets SmartScreen warning. CI has the secrets commented out — uncommenting them without real secrets will break the build (Tauri's bundler treats empty `APPLE_CERTIFICATE` as "a certificate is present").
- **CI**: triggers on `git push --tags` with `v*` tag. `fail-fast: false` — macOS and Windows legs are independent. The macOS leg also updates the Homebrew Cask tap.

## Design & UI conventions

- **No chart library** — all charts are custom SVG in `charts.tsx` (BarChart, CostDonut, Sparkline, Heatmap, Segmented). All token values are in **millions (M)**.
- **Dark/light themes**: defined in `src/data.ts` as `TH` object. Theme switching uses `.ts-no-transition` class to prevent CSS transition cross-fade during the switch.
- **i18n**: React context (`I18nContext`) with EN/ZH dictionaries in `i18n.ts`. Both languages must be kept in sync.
- **Fonts**: IBM Plex Sans, IBM Plex Mono, Space Grotesk — loaded from Google Fonts in `index.html`.
- **Scrollbars hidden**: `.om-scroll` class disables scrollbars globally. The panel is non-resizable (400×660 fixed).
- **Count-up animation**: `useCountUp` hook with `useLayoutEffect` reset-to-0 before counting up. `resetKey` tracks popover open + period switch.
- **Confetti celebration**: triggers at every 100M token milestone (per week/month). Milestones persisted in `~/Library/Application Support/tokenscope/milestones.json` (data dir, not cache — survives purges).
- **Screenshot**: `domToPng` (modern-screenshot) captures the panel to Desktop — bypasses macOS Screen Recording permission.
- **Launch-at-login**: managed via `tauri-plugin-autostart` with a persisted preference (data dir). First run defaults to on.

## Tests

- **Rust**: `cargo test -p tokenscope` — unit tests inline in `src-tauri/src/lib.rs` (milestone logic, `fmt_tokens_m`).
- **Frontend**: no tests. No test framework configured.
- No CI test step — the release workflow only builds.

## Adding new Tauri commands

Register in `lib.rs` `invoke_handler`, then in `src-tauri/capabilities/default.json` add the corresponding permission if needed. The frontend calls via `invoke` from `@tauri-apps/api/core`.

## Relevant docs

- `README.md` — user-facing install and feature overview
- `PRD.md` — product requirements (Chinese); for feature scope decisions
- `docs/BUGFIXES.md` — known bugs found during development, with root cause and fix
- `docs/REVIEW.md` — code review report (June 2026), includes known issues and risk areas
