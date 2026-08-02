# OCScale

**English** · [中文](README-zh.md)

A **menu-bar / system-tray app for macOS and Windows** that shows your OpenCode CLI
**daily token usage, estimated cost, and per-model / project / agent / MCP / Skill
breakdown**.

Stack: **Tauri 2 + React + TypeScript** (frontend) / **Rust** (data layer).

![OCScale panel](docs/screenshot.png)

## What it does

- Shows today's token count next to the menu-bar icon (e.g. `⬡ 14.00M`); on Windows the
  same number is exposed through the tray tooltip, since the tray API has no text label
- Click the tray icon to toggle the panel: Day / Week / Month, each compared against the
  previous period with a percentage delta
- Metrics: total tokens (input / cache / output / reasoning), estimated cost, requests /
  sessions
- Breakdowns: **by model**, **by project**, **by agent**, **by MCP call**, **by Skill
  call** — with a cost donut (hover for a single entry) and a ~26-week activity heatmap
- Three tabs: **Overview / Agents / Sessions** (the Code tab was removed — OpenCode's DB
  never populates code stats)
- **Counts only the MCP servers / Skills you installed yourself** — all OpenCode
  built-in tools are filtered out
- Extras: 100M-token milestone confetti, save-screenshot to Desktop, launch-at-login
  preference, dark/light/system theme, EN/中文 UI

## Data sources (zero-intrusion)

The app only ever **reads** OpenCode's data — it never writes to or modifies it.

| Purpose | Source |
|---------|--------|
| Messages (tokens / model / tool calls) | OpenCode SQLite DB — `$XDG_DATA_HOME/opencode/opencode.db` or `~/.local/share/opencode/opencode.db` |
| User MCP whitelist | `~/.config/opencode/opencode.json` → `mcp` object keys |
| User Skill whitelist | `~/.config/opencode/skills/` directory |
| Model prices | **Primary**: [models.dev](https://models.dev/api.json) → **Fallback**: [LiteLLM](https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json) → built-in snapshot. Cached in `~/Library/Caches/ocscale/` (platform cache dir), refreshed every 24h, offline fallback |

### Key processing

- One **assistant message** = one event; per-message timestamps give accurate hourly
  charts, while session-level metadata (agent, project, title) comes from the `session`
  / `project` tables
- Token split: `input` (uncached) / `cache` (creation + read) / `output` / `reasoning`;
  the UI folds cache into "In" and shows a separate "cached %"
- Price matching: exact model name → normalized name (strip vendor prefix + `.`↔`p`,
  e.g. `glm-5.1`⇄`glm-5p1`); models.dev's official bare-name price wins
- Cost is priced per token type; each model carries a `priced` flag — **models not found
  in any source still count tokens but are labelled "no price"**. OpenCode's own
  per-message `cost` field is used as a fallback for unrecognised models
- Tool classification: `{server}_{tool}` names whose prefix is in your OpenCode config
  → MCP; the `skill` tool's `state.input.name` matching your skills directory → Skill;
  built-in tools (`read`, `write`, `edit`, `bash`, `grep`, `glob`, `task`, `todowrite`,
  `question`, `webfetch`, …) are ignored

> Cost is an **estimate** based on public prices; treat it as "equivalent spend value".

### Token types & cost formula

Every assistant message's `tokens` object reports mutually exclusive counts:

| Stage | DB field | What it is |
|-------|----------|------------|
| **Input** (uncached) | `tokens.input` | New prompt tokens sent this turn |
| **Cache write** | `tokens.cache.write` | Context written into the prompt cache |
| **Cache read** (hit) | `tokens.cache.read` | Context replayed from the cache |
| **Output** | `tokens.output` | Tokens the model generated |
| **Reasoning** | `tokens.reasoning` | Reasoning tokens, billed at the output rate |

```
total = input + cache.write + cache.read + output + reasoning

cost  = input      × p.input
      + cache.write × p.cache_create
      + cache.read  × p.cache_read    # cache hits billed at the discounted read rate
      + output      × p.output
      + reasoning   × p.output
```

Cache hits are **not** billed as normal input — they use the dedicated (cheaper)
`cache_read` rate, which is why heavily-cached usage shows a large token count but a
modest cost.

## Install

Download the latest release from
[GitHub Releases](https://github.com/Cherno76/ocscale/releases) (`.dmg` on macOS, NSIS
`.exe` on Windows), or build from source (below).

Because builds are **unsigned / unnotarized**:

- **macOS**: Gatekeeper blocks the first launch — right-click the app → **Open** →
  confirm, or run `xattr -cr /Applications/OCScale.app` once
- **Windows**: SmartScreen warns on first run — click **More info → Run anyway**

The app installs per-user, registers launch-at-login, and starts in the menu bar / tray
(no Dock icon, no window at launch).

## Develop

```bash
pnpm install
pnpm tauri dev         # launch the desktop app (requires the Rust toolchain)
```

Frontend-only preview (uses a real-data snapshot `public/dev-dashboard.json`, which is
gitignored and machine-specific):

```bash
pnpm dev               # http://localhost:1420
# refresh the snapshot:
cd src-tauri && cargo run --example dump > ../public/dev-dashboard.json
```

Rust unit tests (milestone logic):

```bash
cargo test -p ocscale
```

## Build

```bash
pnpm tauri build       # .app / .dmg on macOS, .exe (NSIS) on Windows
```

Artifacts land in `src-tauri/target/release/bundle/`. CI builds on `git push --tags`
with a `v*` tag; the macOS leg also updates the Homebrew Cask tap. See `AGENTS.md` for
the versioning rules (every code change bumps PATCH; the three version files stay in
sync).

## Structure

```
src/                  React frontend (5 files)
  data.ts             types + Tauri bridge + theme + formatting
  charts.tsx          chart primitives (bars / donut / sparkline / heatmap / segmented)
  App.tsx             main panel
  i18n.ts             EN/ZH dictionaries
  main.tsx            entry

src-tauri/src/        Rust backend (7 files)
  store.rs            OpenCode SQLite → RawEvent (+ tool-call classification)
  parser.rs           aggregation (Day/Week/Month + heatmap)
  pricing.rs          models.dev / LiteLLM price loading and costing
  config.rs           user MCP / Skill whitelist
  model.rs            data structures returned to the frontend
  lib.rs              Tauri commands + menu-bar tray + NSPanel
  main.rs             entry
```

## Bug log

Notable bugs found during development — symptom, root cause, and fix — are
collected in [docs/BUGFIXES.md](docs/BUGFIXES.md).
