# AGENTS.md — OCScale

Menu-bar/tray app for monitoring OpenCode token usage. **Tauri 2 + React 18 (TS) + Rust**.

## Commands

```bash
pnpm install                     # must use pnpm (pnpm 10 in CI)
pnpm tauri dev                   # full app (requires Rust toolchain)
pnpm dev                         # frontend-only preview at http://localhost:1420 (mock data)
pnpm tauri build                 # production .app/.dmg (macOS) or .exe (Windows)
cargo test -p ocscale         # Rust unit tests (src-tauri/src/lib.rs inline)
# regenerate dev mock snapshot:
cd src-tauri && cargo run --example dump > ../public/dev-dashboard.json
```

- `pnpm build` runs `tsc && vite build` — typecheck comes first, then bundle.
- `strictPort: true` on port 1420 — nothing else should bind that port.
- There is **no lint/formatter config** (no ESLint, Prettier, or pre-commit hooks).
- The repo is a Cargo workspace (`core` + `src-tauri`); the workspace
  `Cargo.lock` lives at the repo root (not `src-tauri/`), and the shared target
  dir is `src-tauri/target` (set in `.cargo/config.toml`).

## Versioning

Version is `MAJOR.MINOR.PATCH` starting at `0.1.0`. Every code change bumps `PATCH` by 1. After `0.1.9` comes `0.2.0` (patch wraps to 0, minor +1). After `0.9.9` comes `1.0.0`.

Three files must be updated together:
- `package.json` (top-level `"version"`)
- `src-tauri/tauri.conf.json` (`"version"`)
- `src-tauri/Cargo.toml` (`version`)

The frontend reads version dynamically via the `get_version` Tauri command (reads `CARGO_PKG_VERSION` at compile time).

**Auto-commit on minor bump**: when PATCH wraps (0.1.9 → 0.2.0, 0.2.9 → 0.3.0, etc.), commit the version change and push to GitHub automatically — no need to wait for the user to ask. Routine patch bumps (0.1.0 → 0.1.1) are committed alongside the code changes that triggered them.

Keep the root `Cargo.lock` in sync when the version changes (`cargo generate-lockfile`).

## Architecture

```
OpenCode SQLite DB (~/.local/share/opencode/opencode.db)
    ↓ (core/src/store.rs — query_events → RawEvent[], compare-by-value for change detection)
Codex transcripts (~/.codex/sessions/** + archived_sessions/)
    ↓ (core/src/store_codex.rs — mtime-memoized parse → RawEvent)
DeepSeek Harness session logs (~/.dsh/sessions/**/session.jsonl[.zstd], or $DSH_HOME)
    ↓ (core/src/store_dsh.rs — mtime-memoized parse → RawEvent)
ocscale-core parser::build_dashboard() ← serialised by BUILD_LOCK (Mutex), merges all sources
    ├── core/src/config.rs             → MCP/Skill whitelists (opencode.json `mcp` keys + skills dir)
    └── core/src/pricing.rs::Pricing::shared() → Arc memoized, loaded off-main-thread, refreshed every 24h
    ↓
core/src/model.rs (Dashboard → serde JSON)
    ↓ Tauri command get_dashboard() (src-tauri/src/lib.rs)
src/data.ts (fetchDashboard → auto-detects Tauri vs browser dev mock)
    ↓
App.tsx + charts.tsx (React, custom SVG charts — no chart library)

```

**Frontend files** (5): `src/App.tsx`, `charts.tsx`, `data.ts`, `i18n.ts`, `main.tsx`.

**Rust crates**:
- `core` (ocscale-core): `store.rs` (SQLite→RawEvent), `store_codex.rs` (Codex data source — parses `~/.codex` transcripts), `store_dsh.rs` (DeepSeek Harness data source — parses `~/.dsh` session logs), `parser.rs` (aggregation), `pricing.rs` (price loading/cost), `model.rs` (serializable structs), `config.rs` (user MCP/Skill whitelist).
- `src-tauri` (ocscale app): `lib.rs` (app setup, tray, commands, 100M celebration), `balance.rs` (DeepSeek balance), `main.rs` (entry).

## Data sources & pricing

- **Primary**: OpenCode SQLite database at `$XDG_DATA_HOME/opencode/opencode.db` or `~/.local/share/opencode/opencode.db`.
- **Pricing**: models.dev API → LiteLLM → built-in snapshot (`core/snapshots/litellm.json`). Cached at `~/Library/Caches/ocscale/` (macOS) / platform cache dir, refreshed every 24h.
- **DeepSeek pricing override** (built into `pricing.rs`): official `deepseek-v4-flash` / `deepseek-v4-pro` rates with a Beijing-time peak/off-peak split (CNY per 1M tokens) — flash off-peak ¥1.5 cache-miss / ¥0.05 cache-hit / ¥4.5 output, peak ¥3.0 / ¥0.10 / ¥9.0; pro off-peak ¥4.5 / ¥0.15 / ¥13.5, peak ¥9.0 / ¥0.30 / ¥27.0. Peak = Beijing 09:00–12:00 and 14:00–18:00 (UTC+8, no DST); `pricing::cost` picks the rate from the event timestamp (`ts_ms`). Cache-write tokens bill at the cache-miss input rate. Stored as USD at the zh UI's 7.2 rate so `cost × 7.2` shows exact CNY; overrides the live LiteLLM entry, which prices cache writes at 0.
- **MCP/Skill tracking**: implemented for OpenCode. `config.rs` reads user MCP server names from `~/.config/opencode/opencode.json` (`mcp` object keys) and skill names from the `~/.config/opencode/skills/` directory. `store.rs` classifies tool calls from the `part` table: built-in tools are filtered, `{server}_{tool}` names whose prefix matches a configured MCP server count as MCP, and the `skill` tool's `state.input.name` counts as a Skill call.
- Price matching: exact model name → normalized (strip vendor prefix after `/`, `.`↔`p`). Unmatched models still count tokens but show "no price" label.
- OpenCode's per-message `cost` field is used as a fallback when the pricing module doesn't recognise a model.
- **DeepSeek balance (UI)**: the hero shows `当前余额` after the est. cost. Rust `get_balance` command → GET `https://api.deepseek.com/user/balance`, cached 5 min. The API key comes **only** from the key the user enters in the panel (`set_deepseek_key`), stored as `data_dir/ocscale/deepseek_key` (0600) — there is deliberately **no discovery** from env vars or other apps' configs. `get_deepseek_key_status` reports "missing" when none is saved, and the hero shows a 设置 API Key prompt. Never logged or printed.
- **Codex data source (merged)**: `build_dashboard` combines OpenCode + Codex `RawEvent`s; `store_codex.rs` parses `~/.codex/sessions/**` + `archived_sessions/` transcripts (mtime-memoized). Per-turn tokens come from `token_count` (`total_token_usage` is session-cumulative → use deltas; **`input_tokens` includes `cached_input_tokens` as a subset — uncached input = input − cached, never add both**). MCP tools come from the `function_call` `namespace` field (`mcp__<server>`, e.g. `mcp__node_repl`); there is no per-message model/cost (session-level model, pricing-module fallback) and no Skill equivalent. Standalone dump: `cd src-tauri && cargo run --example dump_codex > /tmp/codex-dashboard.json`. Merged display groups projects by name; the Overview tab can toggle Model / Project / Agent.
- **DeepSeek Harness data source (merged)**: `build_dashboard` also merges DSH session logs; `store_dsh.rs` parses `~/.dsh/sessions/**/session.jsonl[.zstd]` (or `$DSH_HOME/sessions`, mtime-memoized, zstd-decompressed). Each `assistant/message` with a `usage` payload is one `RawEvent` (source `"dsh"`); its `usage` maps directly — `inputTokens` is already uncached input, `cacheReadTokens`→`cr`, `cacheWriteTokens`→`cc`, `reasoningTokens`→`reasoning` (no subtraction, unlike Codex). The model comes from `request/context` (`provider` + `model`); MCP tools are `tool/call` names `mcp__<server>__<tool>` (built-in tools like `read`/`bash` are ignored); the header's `cwd` is the project and `agentPreset` the agent. There is no per-message cost or Skill equivalent. Standalone dump: `cd src-tauri && cargo run --example dump_dsh > /tmp/dsh-dashboard.json`.

## Key gotchas

- **`tauri.conf.json` productName is `"OCScale"`**, npm package name is `ocscale`, identifier is `com.ocscale.app`. Be careful not to rename one without the others.
- **`tauri-nspanel`** is a macOS-only git dependency (`ahkohd/tauri-nspanel` branch `v2`). Any code importing it must be `#[cfg(target_os = "macos")]`. It uses the deprecated `objc` crate — the `allow(deprecated)` in `lib.rs:2` is intentional.
- **`macos-private-api`** Tauri feature is enabled for tray operations.
- **macOS tray label**: shown next to the icon via `set_title()`. **Windows**: tray has no label — uses `set_tooltip()` instead. Both must be updated together.
- **Tray label mode**: persisted pref `data_dir/ocscale/tray_mode.json` switches the tray label between today's tokens and the DeepSeek balance (`get_tray_mode` / `set_tray_mode` commands; toggle button in the panel footer). Both title and tooltip follow the mode.
- **Day boundary mode**: persisted pref `data_dir/ocscale/day_mode.json` (`get_day_mode` / `set_day_mode`; toggle in the Day view). `"utc"` makes the Day report + tray "today" use the UTC day boundary to match DeepSeek's platform usage dashboard (early local-morning usage then lands on the previous platform day); week/month/heatmap stay on the local calendar.
- **Period paging**: the Week/Month views have ‹ › arrows (`get_period` command → `parser::period_report(period, offset)`; offset 0 = current, −1 = previous). Week/month reports take an `offset` param; the paged report is fetched on demand and shown statically while the current period keeps live updates.
- **NSPanel**: the macOS window is converted to a non-activating NSPanel (level 25 = NSMainMenuWindowLevel + 1) so it floats over fullscreen apps without stealing focus. Panel hides on resign-key, Space change, and app activation.
- **BUILD_LOCK**: `parser::build_dashboard()` is the single entry point for all data. It holds a Mutex — call it from `spawn_blocking`, never inline on the async runtime.
- **30s background refresh**: the polling loop in `lib.rs` calls `refresh()` (build_dashboard + emit + tray update). The tray also refreshes on panel open via `get_dashboard` command.
- **System theme**: macOS NSPanel doesn't reliably get `prefers-color-scheme`, so an `AppleInterfaceThemeChangedNotification` observer pushes `system-is-dark` to the frontend. On non-macOS, the webview's native `prefers-color-scheme` should work.
- **Unsigned builds**: no code signing or notarization. macOS users must right-click→Open or `xattr -cr`. Windows gets SmartScreen warning. CI has the secrets commented out — uncommenting them without real secrets will break the build (Tauri's bundler treats empty `APPLE_CERTIFICATE` as "a certificate is present").
- **CI**: triggers on `git push --tags` with `v*` tag. `fail-fast: false` — macOS and Windows legs are independent. The macOS leg also updates the Homebrew Cask tap.

## Design & UI conventions

- **No chart library** — all charts are custom SVG in `charts.tsx` (BarChart, CostDonut, Sparkline, Heatmap, Segmented). All token values are in **millions (M)**.
- **Dark/light themes**: defined in `src/data.ts` as `TH` object. Theme switching uses `.ts-no-transition` class to prevent CSS transition cross-fade during the switch.
- **i18n**: React context (`I18nContext`) with EN/ZH dictionaries in `i18n.ts`. Both languages must be kept in sync.
- **Fonts**: IBM Plex Sans, IBM Plex Mono, Space Grotesk — loaded from Google Fonts in `index.html`.
- **Scrollbars hidden**: `.om-scroll` class disables scrollbars globally; `.om-nobar` does the same for capped inner lists.
- **Panel sizing**: window is 800 wide × ~660 default, user-non-resizable. Height auto-fits content via the `fit_panel` command (frontend measures the card after data/tab changes; Rust clamps 400–800 logical px). Resizing keeps the current top-left corner — the position is only set on open, never re-pinned to the tray anchor, so the popover grows downward in place. Overview/Agents are two-column grids split by a `gridLine` divider; model/project/agent and session lists are height-capped with internal (hidden-scrollbar) scrolling.
- **Count-up animation**: `useCountUp` hook with `useLayoutEffect` reset-to-0 before counting up. `resetKey` tracks popover open + period switch.
- **Confetti celebration**: triggers at every 100M token milestone (per week/month). Milestones persisted in `~/Library/Application Support/ocscale/milestones.json` (data dir, not cache — survives purges).
- **Screenshot**: `domToPng` (modern-screenshot) captures the panel to Desktop — bypasses macOS Screen Recording permission.
- **Launch-at-login**: managed via `tauri-plugin-autostart` with a persisted preference (data dir). First run defaults to on.

## Tests

- **Rust**: `cargo test -p ocscale` — unit tests inline in `src-tauri/src/lib.rs` (milestone logic, `fmt_tokens_m`).
- **Frontend**: no tests. No test framework configured.
- No CI test step — the release workflow only builds.

## Adding new Tauri commands

Register in `lib.rs` `invoke_handler`, then in `src-tauri/capabilities/default.json` add the corresponding permission if needed. The frontend calls via `invoke` from `@tauri-apps/api/core`.

## Relevant docs

- `README.md` — user-facing install and feature overview
- `PRD.md` — product requirements (Chinese); for feature scope decisions
- `docs/BUGFIXES.md` — known bugs found during development, with root cause and fix
