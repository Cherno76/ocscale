# Bug Log

A record of bugs hit during development, each with its symptom, root cause, and
fix. Newest first. Useful as a reference for similar issues.

> 项目前身为 TokenScope（监控 Claude CLI 的 JSONL 日志），2026 年 7 月迁移到
> OpenCode SQLite 数据源并更名 OCScale。「OpenCode era」为本数据源下遇到的问题；
> 「Early entries」为更名前记录，其中与前端 / 发布 / 行为相关的部分仍适用于当前代码。

---

## OpenCode era

### 1. macOS Dock 里出现了菜单栏应用不应有的图标

- **Symptom**: OCScale 是 Accessory（菜单栏）应用，却在 Dock 中显示图标。
- **Cause**: 运行时设置了 `ActivationPolicy::Accessory`，但打包的
  `src-tauri/Info.plist` 缺少 `LSUIElement=true`，系统仍按普通 App 处理。
- **Fix**: 在 `Info.plist` 中加入 `LSUIElement=true`（de2aa5b）。

### 2. Code 页签永远显示 0（OpenCode 不填充代码统计）

- **Symptom**: 代码活动（新增 / 删除行、文件数、diff 数）在任何会话中都为 0。
- **Cause**: OpenCode 的 `session` 表从不填充 `summary_additions / summary_deletions /
  summary_files / summary_diffs`，也没有可用的逐消息代码变更数据。
- **Fix**: 先移除恒为 0 的列（ceba409），随后整体移除 Code 页签（a30f6b3）。

### 3. 会话列表按 token 排序，而非按最近活动

- **Symptom**: 「最近会话」的顺序按 token 量排序，活跃会话沉底，列表不稳定。
- **Cause**: `recent_sessions` 按 `tokens` 排序，且每会话只累积了部分消息的元数据。
- **Fix**: 改为按 `last_active_ms`（会话内最新消息时间）降序，并跨所有消息累加
  每会话的 token / cost（7cbfb05、f8dd242）。

### 4. Agents 页签甜甜圈颜色与 token 条不一致

- **Symptom**: 同一 Agent 在 token 排行和费用甜甜圈里颜色不同。
- **Cause**: `CostDonut` 默认按花费重排并重新配色，覆盖了调用方传入的颜色。
- **Fix**: 增加 `preserveColors` 让 Agents 页签保留与 token 条一致的配色，并改用
  按排名（深 → 浅）的蓝色系调色板（7936f67、55224e3）。

### 5. `cargo run --example dump` 在更名后编译失败

- **Symptom**: 文档中的快照生成命令直接编译报错——示例引用
  `tokenscope_lib::dashboard_json()`，而 crate 的 lib 名已改为 `ocscale_lib`。
- **Cause**: tokenscope → ocscale 更名时漏改了 `examples/dump.rs`。
- **Fix**: 改为 `ocscale_lib`；顺带移除残留的 `tokenscope-panel.html` 设计稿、
  清理源码里的 TokenScope 字样，并重新生成本地 `public/dev-dashboard.json`
  （gitignored、随机器而异）。

---

## Early entries（更名前，仍适用）

### 6. Delta percentage was ~100× too small (and hidden)

- **Symptom**: The Day view showed no change percentage next to Total tokens.
- **Cause**: `pct_delta` computed `((cur-prev)/prev*100).round()/100`, which
  cancels the `×100` and returns a **fraction** (e.g. `0.2`) instead of a
  **percentage** (`20`). The UI rounds the value to an integer for display, so
  `0.2 → 0%`, and the "hide when 0" rule then hid it entirely.
- **Fix**: Changed `pct_delta` to `((cur-prev)/prev*10000).round()/100`, which
  returns a real percentage with 2 decimals (e.g. `20.47`).

### 7. Release CI failed: empty Apple signing env var

- **Symptom**: The `v0.1.1` build failed at the bundle step with
  `security: SecKeychainItemImport: ... parameters ... not valid` /
  `failed to import keychain certificate`.
- **Cause**: The workflow passed `APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}`,
  but the secret didn't exist, so it became an **empty string**. Tauri's bundler
  treats the env var's *presence* as "a certificate was provided" and tries to
  `security import` empty data, which fails. (Local builds were fine because the
  var was *unset*, not empty.)
- **Fix**: Commented out the Apple signing/notarization env in `release.yml`
  until the real secrets exist. The build now does ad-hoc signing, like local.

### 8. GitHub Release had no .dmg / .app — it was a draft

- **Symptom**: "The release has no artifacts."
- **Cause**: `releaseDraft: true` — the build *did* succeed and attach the
  `.dmg` + `.app.tar.gz`, but a **draft release is invisible in the public
  Releases list** and its asset URLs return 404, so Homebrew can't download it.
  (The artifacts were also not in the Actions "Artifacts" tab, because
  `tauri-action` uploads to Releases.)
- **Fix**: Set `releaseDraft: false` so each tag publishes immediately and the
  asset URL is live for the Homebrew step and `brew install`.

### 9. Homebrew Cask step would hash a 404 page

- **Symptom**: Latent — the cask `sha256` could be computed from an error page.
- **Cause**: The cask step fetched the asset with `curl -sL` (no `-f`), so a 404
  returned `0` and the GitHub error HTML got hashed into a bogus checksum,
  breaking `brew install` with a `sha256 mismatch`.
- **Fix**: Use `curl -fsSL` so a missing asset fails and retries; fail loudly
  (`exit 1`) if the asset never appears.

### 10. DMG name didn't match the tag

- **Symptom**: A `v0.1.1` tag would build a DMG named after the previous
  version, which the cask step (computing the name from the tag) couldn't
  download → 404.
- **Cause**: Tauri names the artifact from the version in `tauri.conf.json`,
  which lagged behind the tag.
- **Fix**: Bump the version in `package.json`, `tauri.conf.json`, and
  `Cargo.toml` (+ `Cargo.lock`) so the built artifact name matches the tag the
  cask step expects.

### 11. Two menu-bar icons after reinstall

- **Symptom**: Reinstalling/relaunching left two OCScale icons in the menu bar.
- **Cause**: No single-instance guard — a second launch started a second
  process with its own tray icon.
- **Fix**: Added `tauri-plugin-single-instance` (registered first) so a second
  launch hands off to the running instance (showing the popover) and exits.

### 12. Unsigned app blocked by Gatekeeper on first open

- **Symptom**: "Apple cannot verify OCScale.app is free of malware."
- **Cause**: The build is unsigned/unnotarized, and Homebrew adds a quarantine
  attribute to installed apps.
- **Fix**: The cask's `postflight` runs `xattr -cr` on the installed app so
  `brew install` opens cleanly. (The `.dmg` path still needs a manual
  right-click → Open or `xattr -cr`; a full fix needs Developer ID signing +
  notarization.)

### 13. App icon had opaque white corners

- **Symptom**: The rounded app icon showed white square corners in Launchpad.
- **Cause**: The icon PNGs had a white (opaque) background in the corners
  instead of transparent alpha.
- **Fix**: Regenerated the icon from a clean transparent source
  (`scripts/gen_icon.py`, 4× supersampled), then ran `pnpm tauri icon` to
  produce every size + `icon.icns` / `icon.ico` with transparent corners.

### 14. Bar-chart tooltip overlapped the legend above it

- **Symptom**: Hovering a token bar showed its tooltip floating up over the
  Total-tokens "Input … cached" legend, even for short bars.
- **Cause**: To make short bars easy to hover, the hit area was stretched to the
  full column height (`alignSelf: stretch`). The tooltip then anchored to the
  column's `top` — i.e. the top of the chart, right under the legend — so every
  bar's tooltip appeared at the same high spot.
- **Fix** (`charts.tsx`): anchor the tooltip to the *visible bar top*
  (`r.bottom − barPx`, baseline minus bar height) instead of the column top, so
  short bars get a low tooltip clear of the legend.

### 15. Total-tokens bar showed slivers when usage was zero

- **Symptom**: With no usage in the period (Total = 0.00M), the input/output
  split bar still showed a small coloured sliver instead of being empty.
- **Cause**: Each segment had `minWidth: 4`, so even a negligible share
  rendered a 4px block — two slivers when everything was zero.
- **Fix** (`App.tsx`): give the bar a track background and only render the
  coloured segments when `totalTokens > 0`; otherwise the bar is just the empty
  track.

### 16. Total-tokens split bar didn't fill — gray track showed through

- **Symptom**: The 2-colour split bar under Total tokens read as only partly
  filled — coloured segments on the left, gray track visible on the right —
  instead of always 100% when there was usage. Most visible when the split was
  lopsided (e.g. right after clearing stats, with output ≈ 0).
- **Cause**: The two segments used `flexGrow` + `flexBasis: 0` (+ `minWidth: 4`).
  In the WebKit webview that combination sizes each segment to roughly **its own
  grow factor as an absolute fraction of the bar**, not the **grow-factor ratio**
  — so a small `flexGrow` covered ~10% of the bar, not ~100%, and the track
  showed through. The data was correct (verified by dumping `build_dashboard`'s
  JSON and computing the expected ratio); only the rendering was wrong.
- **Fix** (`src/App.tsx`): use explicit `width: X%` instead of `flexGrow`
  (interpreted correctly, always sums to exactly 100%). While here, re-purposed
  the bar from input+cache vs output to **cached vs rest (uncached input +
  output)**: the dark segment is the cache share (matching the "% cached" label),
  and "rest" is wider than output-alone, so a small non-cached share still reads
  past the pill's rounded corner without distorting the ratio. The `SplitLegend`
  below was changed from "Input / Output" to "Cached / New" to match.

### 17. "System" theme mode didn't follow the macOS appearance

- **Symptom**: On macOS, the "System" theme option didn't track the OS dark/light
  mode — neither when toggling system appearance with the popover open, nor after
  quitting and relaunching the app (it stayed on the launch-time appearance).
  Windows was unaffected.
- **Cause**: The frontend derived the system appearance entirely from
  `window.matchMedia("(prefers-color-scheme: dark)")` (`App.tsx`). But OCScale
  is an `Accessory` (menu-bar) app whose popover is a **non-activating `NSPanel`**
  that is `order_out`'d (hidden) most of the time. In that configuration
  WKWebView's `prefers-color-scheme` is unreliable: it doesn't reliably fire the
  `change` event on a system theme switch while the webview is hidden, and at
  launch an Accessory app's `NSApp.effectiveAppearance` (what WKWebView reports)
  may not be synced to the current system value — so even a fresh restart reads
  the wrong appearance.
- **Fix**: Read the OS dark-mode setting natively in Rust and push it to the
  frontend via a Tauri event, bypassing the webview. `system_is_dark()` reads
  `NSUserDefaults`'s `AppleInterfaceStyle` (the user's **global** system
  preference, independent of app focus). `watch_system_theme()` listens on
  `NSDistributedNotificationCenter` for `AppleInterfaceThemeChangedNotification`
  — delivered to every registered app regardless of activation policy or
  frontmost status — and `emit("system-theme", dark)`. `setup()` also emits once
  at startup to correct any stale webview value. The frontend's
  `listen("system-theme")` updates `systemDark`; the existing `matchMedia`
  listener stays as the source of truth on Windows / browser preview. macOS-only
  (`#[cfg(target_os = "macos")]`), no new dependencies — uses the `objc`/`cocoa`/
  `block` re-exports already imported in `lib.rs`. (`src-tauri/src/lib.rs`,
  `src/App.tsx`)

### 18. Selected period pill flashed white→transparent on a light→dark switch

- **Symptom**: Switching the system theme from light to dark while the popover
  was hidden, then opening it, showed a brief "white → transparent" fade on the
  currently-selected period pill (Day/Week/Month) for a moment — most visible
  element of an otherwise-instant flip.
- **Cause**: The `Segmented` selected pill carries
  `transition: "color .15s, background .15s"` (`charts.tsx`), wanted for smooth
  period-switching. On a *whole-theme* flip this turns every color change into a
  cross-fade; the white selected background fading into the dark one was the most
  jarring. Because the panel is hidden when the theme change lands, the first
  painted frame on open is still the old light theme, then the new theme is
  applied and the transition animates the change visibly.
- **Fix**: Suppress per-property transitions across a theme flip so the panel
  repaints in the new theme in one step. Added a global `.ts-no-transition` rule
  (`main.tsx`) and an effect (`App.tsx`) that adds it to `<html>` when `dark`
  changes and removes it after two `requestAnimationFrame`s. Because rAF callbacks
  don't run while the window is hidden, the class stays on until the popover is
  shown — so the first visible frame is already the new theme with no transition,
  then transitions are restored for normal interactions (e.g. clicking
  Day/Week/Month still animates). Skipped on the very first render.
  (`src/main.tsx`, `src/App.tsx`)

---

## Notes

- "Month" was changed from a rolling 30-day window to the **current calendar
  month vs the previous calendar month** — a behavior change requested during
  testing, not a bug.
- "Week" was likewise changed from a rolling last-7-days window to the **current
  calendar week (Monday–Sunday) vs the previous calendar week**, so the delta
  compares this week against last week.
- Delta colors were swapped so usage/cost **up = red** (bad), **down = green**
  (good).
