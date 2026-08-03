import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { domToPng } from "modern-screenshot";
import {
  Dashboard, PeriodReport, ModelStat, ProjectStat, AgentStat, BalanceInfo, SessionInfo, Theme, TH,
  fetchDashboard, fmtInt, fmtTokens, pct, fmtMoney,
} from "./data";
import {
  TokenGlyph, Segmented, BarChart, Sparkline, CostDonut, BarList, Heatmap,
} from "./charts";
import { I18nContext, DICT, useT, type I18nCtx, type Lang, type Dict } from "./i18n";

// Count up to `target`. Restarts from 0 whenever `resetKey` changes (popover
// open / period switch); on a live value change it eases from the current
// value to the new one instead of snapping back to 0.
function useCountUp(target: number, resetKey: string, active: boolean, duration = 850): number {
  const [val, setVal] = useState(0);
  const valRef = useRef(0);
  const keyRef = useRef<string | null>(null);
  const rafRef = useRef(0);
  // useLayoutEffect so the reset-to-0 is committed *before* the browser paints
  // (otherwise the old/final value flashes for a frame before counting up).
  useLayoutEffect(() => {
    cancelAnimationFrame(rafRef.current);
    const set = (v: number) => { valRef.current = v; setVal(v); };
    // while the popover is hidden, hold at 0 so the next open starts clean
    if (!active) { keyRef.current = null; set(0); return; }
    const reset = keyRef.current !== resetKey;
    keyRef.current = resetKey;
    // open / period switch → start from 0 (paint it now); live update → ease
    // from the current value to the new one.
    let from = valRef.current;
    if (reset) { from = 0; set(0); }
    const start = performance.now();
    const ease = (t: number) => 1 - Math.pow(1 - t, 3); // easeOutCubic
    const tick = (now: number) => {
      const p = Math.min(1, (now - start) / duration);
      set(from + (target - from) * ease(p));
      if (p < 1) rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [resetKey, target, active, duration]);
  return val;
}

function Delta({ v, theme }: { v: number; theme: Theme }) {
  const up = v >= 0;
  // Usage/cost going up is "bad" → red; going down is "good" → green.
  const col = up ? "#e0795f" : "#27b06e";
  return (
    <span style={{ font: `600 10px ${theme.mono}`, color: col, display: "inline-flex", alignItems: "center", gap: 2,
      padding: "1.5px 5px", borderRadius: 5, background: up ? "rgba(224,121,95,0.16)" : "rgba(39,176,110,0.14)" }}>
      {up ? "▲" : "▼"}{Math.abs(Math.round(v))}%
    </span>
  );
}

// Round each value's share to 1 decimal (%) via largest-remainder apportionment,
// so the displayed percentages sum to exactly 100.0% (plain rounding wouldn't).
function ProjectRow({ p, max, theme, share }: { p: ProjectStat; max: number; theme: Theme; share: number }) {
  const pctStr = share % 1 === 0 ? share.toFixed(0) : share.toFixed(1);
  const PALETTE = ["#1e40af", "#2563eb", "#3b82f6", "#60a5fa", "#4b5a52", "#a78bfa", "#e0795f", "#6ee7b7"];
  const hash = p.projectId.split("").reduce((a, c) => a + c.charCodeAt(0), 0);
  const color = PALETTE[hash % PALETTE.length];
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 9, padding: "5px 0" }}>
      <span style={{ width: 7, height: 7, borderRadius: 2, background: color, flex: "0 0 auto" }} />
      <div style={{ minWidth: 0, flex: "0 0 118px" }}>
        <div style={{ font: `500 11.5px ${theme.ui}`, color: theme.text, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{p.projectName}</div>
      </div>
      <div style={{ flex: 1, height: 5, borderRadius: 3, background: theme.gridLine, overflow: "hidden" }}>
        <div style={{ width: `${(p.tokens / max) * 100}%`, height: "100%", background: color, borderRadius: 3 }} />
      </div>
      <span style={{ font: `500 10.5px ${theme.mono}`, color: theme.dim, flex: "0 0 auto", width: 42, textAlign: "right" }}>{fmtTokens(p.tokens)}</span>
      <span style={{ font: `600 10.5px ${theme.mono}`, color: theme.text, flex: "0 0 auto", width: 40, textAlign: "right" }}>{pctStr}%</span>
    </div>
  );
}
function sharePcts(values: number[]): number[] {
  const total = values.reduce((s, v) => s + v, 0);
  if (total <= 0) return values.map(() => 0);
  const UNITS = 1000; // work in 0.1% units; target is 100.0%
  const raw = values.map((v) => (v / total) * UNITS);
  const units = raw.map(Math.floor);
  const left = Math.round(UNITS - units.reduce((s, f) => s + f, 0));
  raw
    .map((r, i) => ({ i, frac: r - Math.floor(r) }))
    .sort((a, b) => b.frac - a.frac)
    .slice(0, left)
    .forEach(({ i }) => (units[i] += 1));
  return units.map((u) => u / 10);
}

function ModelRow({ m, max, theme, share }: { m: ModelStat; max: number; theme: Theme; share: number }) {
  // 1-decimal share; whole numbers drop the ".0" (100% not 100.0%).
  const pctStr = share % 1 === 0 ? share.toFixed(0) : share.toFixed(1);
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 9, padding: "5px 0" }}>
      <span style={{ width: 7, height: 7, borderRadius: 2, background: m.color, flex: "0 0 auto" }} />
      <div style={{ minWidth: 0, flex: "0 0 118px" }}>
        <div style={{ font: `500 11.5px ${theme.ui}`, color: theme.text, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{m.name}</div>
      </div>
      <div style={{ flex: 1, height: 5, borderRadius: 3, background: theme.gridLine, overflow: "hidden" }}>
        <div style={{ width: `${(m.tokens / max) * 100}%`, height: "100%", background: m.color, borderRadius: 3 }} />
      </div>
      <span style={{ font: `500 10.5px ${theme.mono}`, color: theme.dim, flex: "0 0 auto", width: 42, textAlign: "right" }}>{fmtTokens(m.tokens)}</span>
      <span style={{ font: `600 10.5px ${theme.mono}`, color: theme.text, flex: "0 0 auto", width: 40, textAlign: "right" }}>{pctStr}%</span>
    </div>
  );
}

function MiniStat({ label, value, sub, theme, accent, children }:
  { label: string; value: string; sub?: string; theme: Theme; accent?: string; children?: React.ReactNode }) {
  return (
    <div style={{ background: theme.gridLine, borderRadius: 9, padding: "9px 10px", minWidth: 0 }}>
      <div style={{ font: `500 9.5px ${theme.ui}`, color: theme.dim, letterSpacing: ".04em", textTransform: "uppercase" }}>{label}</div>
      <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", marginTop: 3, gap: 6 }}>
        <span style={{ font: `600 17px/1 ${theme.mono}`, color: accent || theme.text }}>{value}</span>
        {children}
      </div>
      {sub && <div style={{ font: `500 9px ${theme.mono}`, color: theme.faint, marginTop: 3 }}>{sub}</div>}
    </div>
  );
}

// Cached/Rest legend: full words by default, abbreviated when the row would
// otherwise overflow. Mirrors the split bar above (dark = cached, light = rest).
// When reasoning tokens are present, they appear inline before the cache split.
function SplitLegend({ t, tr, cacheM, restM, cachedPct, reasoningM }:
  { t: Theme; tr: Dict; cacheM: number; restM: number; cachedPct: number; reasoningM?: number }) {
  const ref = useRef<HTMLDivElement>(null);
  const [compact, setCompact] = useState(false);
  const key = `${reasoningM ?? 0}|${cacheM}|${restM}|${cachedPct}`;
  // reset to full labels whenever the numbers change, then re-measure
  useLayoutEffect(() => { setCompact(false); }, [key]);
  useLayoutEffect(() => {
    const el = ref.current;
    if (el && !compact && el.scrollWidth > el.clientWidth + 1) setCompact(true);
  });
  return (
    <div ref={ref} style={{
      display: "flex", alignItems: "center", gap: 14,
      font: `500 10px ${t.mono}`, color: t.dim, marginBottom: 14, whiteSpace: "nowrap", overflow: "hidden",
    }}>
      {reasoningM !== undefined && reasoningM > 0 && (
        <span><span style={{ color: t.reasoningCol }}>●</span> {tr.reasoning} {reasoningM.toFixed(2)}M</span>
      )}
      <span><span style={{ color: t.accent }}>●</span> {tr.cached} {cacheM.toFixed(2)}M</span>
      <span><span style={{ color: t.accentSoft }}>●</span> {tr.new_} {restM.toFixed(2)}M</span>
      <span style={{ color: t.faint }}>{cachedPct}{tr.pctCached}</span>
    </div>
  );
}

const SectionRule = ({ t, m = "12px 0 10px" }: { t: Theme; m?: string }) => (
  <div style={{ height: 1, background: t.gridLine, margin: m }} />
);
const Label = ({ t, children }: { t: Theme; children: React.ReactNode }) => (
  <span style={{ font: `600 10px ${t.ui}`, color: t.dim, letterSpacing: ".05em", textTransform: "uppercase", whiteSpace: "nowrap" }}>{children}</span>
);

function ThemeToggle({ pref, theme, td, onCycle }: { pref: "dark" | "light" | "system"; theme: Theme; td: Dict; onCycle: () => void }) {
  const t = theme;
  // Single button cycling Dark → Light → System; the icon shows the current mode.
  const label = pref === "system" ? td.system : pref === "dark" ? td.dark : td.light;
  return (
    <button onClick={onCycle} title={`Theme: ${label}`} aria-label={`theme: ${label}`} style={{
      display: "inline-flex", alignItems: "center", justifyContent: "center",
      width: 26, height: 26, borderRadius: 7, cursor: "pointer", padding: 0,
      background: t.segBg, border: `1px solid ${t.segBorder}`, color: t.dim,
    }}>
      {pref === "light" ? (
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={t.dim} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <circle cx="12" cy="12" r="4.2" />
          <path d="M12 2.5v2.2M12 19.3v2.2M2.5 12h2.2M19.3 12h2.2M5.1 5.1l1.6 1.6M17.3 17.3l1.6 1.6M18.9 5.1l-1.6 1.6M6.7 17.3l-1.6 1.6" />
        </svg>
      ) : pref === "dark" ? (
        <svg width="14" height="14" viewBox="0 0 24 24" fill={t.dim} stroke="none">
          <path d="M21 12.9A9 9 0 1 1 11.1 3a7.2 7.2 0 0 0 9.9 9.9z" />
        </svg>
      ) : (
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={t.dim} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <rect x="3" y="4.5" width="18" height="12.5" rx="1.6" />
          <path d="M8.5 20.5h7M12 17v3.5" />
        </svg>
      )}
    </button>
  );
}

function LangToggle({ lang, onClick, theme }: { lang: Lang; onClick: () => void; theme: Theme }) {
  return (
    <button onClick={onClick} title={lang === "en" ? "中文" : "English"} aria-label="switch language" style={{
      display: "inline-flex", alignItems: "center", justifyContent: "center",
      width: 26, height: 26, borderRadius: 7, cursor: "pointer", padding: 0,
      background: theme.segBg, border: `1px solid ${theme.segBorder}`, color: theme.dim,
      font: `600 11px ${theme.mono}`, letterSpacing: ".02em",
    }}>
      {lang === "en" ? "中" : "EN"}
    </button>
  );
}

/// Small pill switch used for on/off states (menu-bar mode, launch-at-login).
function Switch({ on, theme, onClick, title }: { on: boolean; theme: Theme; onClick: () => void; title: string }) {
  return (
    <button onClick={onClick} role="switch" aria-checked={on} aria-label={title} title={title}
      style={{
        position: "relative", width: 26, height: 15, borderRadius: 8, padding: 0, cursor: "pointer", flex: "0 0 auto",
        background: on ? theme.accent : theme.segBg,
        border: `1px solid ${on ? theme.accent : theme.segBorder}`,
        transition: "background .15s",
      }}>
      <span style={{
        position: "absolute", top: 2, left: on ? 12 : 2, width: 9, height: 9, borderRadius: "50%",
        background: "#fff", transition: "left .15s",
      }} />
    </button>
  );
}

/// Compact square icon button with hover feedback (danger tint for quit).
function IconButton({ theme, title, onClick, danger, disabled, children }:
  { theme: Theme; title: string; onClick: () => void; danger?: boolean; disabled?: boolean; children: React.ReactNode }) {
  const [h, setH] = useState(false);
  const t = theme;
  return (
    <button onClick={onClick} title={title} aria-label={title} disabled={disabled}
      onMouseEnter={() => setH(true)} onMouseLeave={() => setH(false)}
      style={{
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        width: 28, height: 28, borderRadius: 8, padding: 0, flex: "0 0 auto",
        cursor: disabled ? "default" : "pointer",
        background: h ? t.segOnBg : t.segBg, border: `1px solid ${t.segBorder}`,
        color: danger && h ? "#e0795f" : t.dim,
        transition: "background .15s, color .15s",
      }}>
      {children}
    </button>
  );
}

function ScreenshotButton({ theme, busy, onClick, td }: { theme: Theme; busy: boolean; onClick: () => void; td: Dict }) {
  const t = theme;
  return (
    <button onClick={onClick} disabled={busy} title={td.screenshotTitle} aria-label="save screenshot" style={{
      display: "inline-flex", alignItems: "center", justifyContent: "center",
      width: 26, height: 26, borderRadius: 7, cursor: busy ? "default" : "pointer", padding: 0,
      background: t.segBg, border: `1px solid ${t.segBorder}`, color: t.dim,
    }}>
      {busy ? (
        <svg className="om-spin" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={t.dim} strokeWidth="2.6" strokeLinecap="round">
          <path d="M12 3a9 9 0 1 0 9 9" />
        </svg>
      ) : (
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke={t.dim} strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round">
          <path d="M3 8.5A2.5 2.5 0 0 1 5.5 6h1.7l1.1-1.6A1.5 1.5 0 0 1 9.5 4h5a1.5 1.5 0 0 1 1.2.4L16.8 6h1.7A2.5 2.5 0 0 1 21 8.5v8A2.5 2.5 0 0 1 18.5 19h-13A2.5 2.5 0 0 1 3 16.5z" />
          <circle cx="12" cy="12.2" r="3.4" />
        </svg>
      )}
    </button>
  );
}

function Panel({ dash, dark, themePref, onToggleTheme, openGen, active, lang, toggleLang, onRefresh, version, balance, trayMode, onToggleTrayMode, dayMode, onDayModeChange }:
  { dash: Dashboard; dark: boolean; themePref: "dark" | "light" | "system"; onToggleTheme: () => void; openGen: number; active: boolean; lang: Lang; toggleLang: () => void; onRefresh: () => void; version: string; balance: BalanceInfo | null; trayMode: "tokens" | "balance"; onToggleTrayMode: () => void; dayMode: "local" | "utc"; onDayModeChange: (m: "local" | "utc") => void }) {
  const t = TH[dark ? "dark" : "light"];
  const { t: tr } = useT();
  const [tab, setTab] = useState<"Overview" | "Agents" | "Sessions">("Overview");
  const periodItems = [tr.day, tr.week, tr.month];
  // Drag the popover by its body (Windows/Linux only — macOS uses the menu-bar
  // NSPanel and is gated out). A real OS window-drag begins only once the
  // pointer moves past a small threshold, so a plain click still clicks through
  // / dismisses and never arms the hide-suppression guard.
  const canDrag = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window && !navigator.userAgent.includes("Macintosh");
  const dragRef = useRef<{ x: number; y: number } | null>(null);
  const [period, setPeriod] = useState<"Day" | "Week" | "Month">("Week");
  const [refreshing, setRefreshing] = useState(false);
  const [autostartOn, setAutostartOn] = useState(false);
  const [costTab, setCostTab] = useState<"model" | "project" | "agent">("model");
  useEffect(() => {
    // Read initial autostart state from the Rust backend.
    if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
      import("@tauri-apps/api/core").then(({ invoke }) =>
        invoke<boolean>("get_autostart").then(setAutostartOn).catch(() => {})
      );
    }
  }, []);
  const handleRefresh = () => {
    if (refreshing) return;
    setRefreshing(true);
    onRefresh();
    setTimeout(() => setRefreshing(false), 1200);
  };
  const handleAutostart = async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const next = await invoke<boolean>("set_autostart", { on: !autostartOn }).catch(() => null);
    if (next !== null) setAutostartOn(next);
  };
  const handleQuit = () => {
    import("@tauri-apps/api/core").then(({ invoke }) => invoke("quit_app"));
  };
  const P: PeriodReport = period === "Day" ? dash.day : period === "Month" ? dash.month : dash.week;
  const M = P.metrics;
  // animated Total tokens: counts up from 0 on each open / period switch;
  // held at 0 while the popover is hidden so it never flashes the final value.
  const animTotal = useCountUp(M.totalTokens, `${period}:${openGen}`, active);
  // Split bar = cached portion vs the rest (uncached input + output), as exact
  // width percentages. Width% (not flexGrow + flexBasis:0): in the WebKit webview
  // that combination sizes each segment to roughly its own grow factor — an
  // absolute fraction — instead of the grow-factor *ratio*, so a lopsided split
  // left the gray track showing through. Width% sums to exactly 100%. The dark
  // segment is the cache share (matching the "% cached" label); "rest" is wider
  // than output-alone would be, so a small non-cached share still reads on the
  // pill-shaped bar. Ratios are exact, never floored.
  const splitTot = M.inputTokens + M.cacheTokens + M.outputTokens + M.reasoningTokens;
  const cachePct = splitTot > 0 ? (M.cacheTokens / splitTot) * 100 : 0;
  const restPct = splitTot > 0 ? ((M.inputTokens + M.outputTokens + M.reasoningTokens) / splitTot) * 100 : 0;
  const models = P.models;
  // Hide noise: 0% token-share rows, and $0 entries in the cost donut.
  // Show models whose share is at least 0.1% when rounded to 1 decimal; below
  // that it'd render a meaningless "0.0%" (a negligible token share). Such a
  // model can still appear under Cost if it has a non-zero cost.
  const tokenModels = models.filter(
    (m) => Math.round((m.tokens / (M.totalTokens || 1)) * 1000) / 10 >= 0.1
  );
  const costModels = models.filter((m) => m.cost > 0);
  const projectCostItems: ModelStat[] = P.projects
    .filter(p => p.cost > 0)
    .map(p => ({
      name: p.projectName,
      vendor: "",
      tokens: p.tokens,
      cost: p.cost,
      color: "",
      priced: true,
      costSource: "pricing",
    }));
  // models that were used but have no LiteLLM pricing (cost unknown, not $0)
  const unpricedModels = models.filter((m) => !m.priced && m.tokens > 0);
  const maxM = Math.max(...tokenModels.map((m) => m.tokens), 1e-9);
  // Per-row shares that sum to exactly 100.0% (largest-remainder over visible rows).
  const tokenShares = sharePcts(tokenModels.map((m) => m.tokens));

  // Project token rows — same filtering and share logic as model rows.
  const projectTokens = P.projects.filter(
    (p) => Math.round((p.tokens / (M.totalTokens || 1)) * 1000) / 10 >= 0.1
  );
  const maxP = Math.max(...projectTokens.map((p) => p.tokens), 1e-9);
  const projectShares = sharePcts(projectTokens.map((p) => p.tokens));
  // Period-scoped agent stats (the Agents tab shows the all-time view).
  const agentStats = P.agents || [];
  const maxA = Math.max(...agentStats.map((a) => a.tokens), 1e-9);
  const agentShares = sharePcts(agentStats.map((a) => a.tokens));
  // Blue palette by rank (darkest → lightest), matching the Agents tab.
  const AGENT_BLUES = ["#1e3a8a", "#1e40af", "#2563eb", "#3b82f6", "#60a5fa", "#93c5fd", "#bfdbfe"];
  const agentColor = (i: number) => AGENT_BLUES[i % AGENT_BLUES.length];
  // Keep donut colors aligned with the row colors by mapping over the full
  // (already sorted) list and keeping only entries with cost > 0.
  const agentCostItems: ModelStat[] = [];
  agentStats.forEach((a, i) => {
    if (a.cost > 0) {
      agentCostItems.push({
        name: a.agent, vendor: "", tokens: a.tokens, cost: a.cost,
        color: agentColor(i), priced: true, costSource: "pricing",
      });
    }
  });
  const trendSub = { Day: tr.today24h, Week: tr.thisWeek, Month: tr.thisMonth }[period];

  // screenshot capture: rasterize the full panel card to a PNG and hand it to
  // the Rust `save_screenshot` command (browser preview falls back to a download).
  const [shotBusy, setShotBusy] = useState(false);
  const [toast, setToast] = useState<{ msg: string; ok: boolean } | null>(null);
  const toastTimer = useRef<number | null>(null);
  // ¥ for CNY, $ for USD, otherwise the currency code itself.
  const balSym = balance ? (balance.currency === "CNY" ? "¥" : balance.currency === "USD" ? "$" : balance.currency + " ") : "";
  const showToast = (msg: string, ok: boolean) => {
    if (toastTimer.current) window.clearTimeout(toastTimer.current);
    setToast({ msg, ok });
    toastTimer.current = window.setTimeout(() => setToast(null), 1800);
  };
  const captureScreenshot = async () => {
    if (shotBusy) return;
    const el = document.querySelector<HTMLElement>(".om-scroll");
    if (!el) return;
    setShotBusy(true);
    try {
      // explicit width/height = full scrollable content, not just the viewport;
      // filter drops the capture button itself (and its in-flight spinner) so
      // the saved image is a clean dashboard, not a shot of the button.
      const dataUrl = await domToPng(el, {
        scale: 2,
        backgroundColor: dark ? "#1f2226" : "#ffffff",
        width: el.scrollWidth,
        height: el.scrollHeight,
        filter: (n) => !(n instanceof HTMLElement && n.getAttribute("aria-label") === "save screenshot"),
      });
      const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
      if (inTauri) {
        await invoke<string>("save_screenshot", { dataUrl });
        showToast(tr.savedToDesktop, true);
      } else {
        const a = document.createElement("a");
        a.href = dataUrl;
        a.download = "ocscale.png";
        document.body.appendChild(a);
        a.click();
        a.remove();
        showToast(tr.downloaded, true);
      }
    } catch {
      showToast(tr.screenshotFailed, false);
    } finally {
      setShotBusy(false);
    }
  };

  return (
    <div style={{
      width: "100%", height: "100vh", overflow: "hidden", boxSizing: "border-box",
      position: "relative",
      background: "transparent", padding: 0,
      fontFamily: t.ui,
    }}>
      <div className="om-scroll"
        onMouseDown={canDrag ? (e) => {
          // Record the press; the real drag only starts once the pointer moves
          // past the threshold (onMouseMove). Skip interactive controls
          // (data-no-drag) and non-left buttons so clicks still register.
          if (e.button !== 0) return;
          if ((e.target as HTMLElement).closest("[data-no-drag]")) return;
          dragRef.current = { x: e.clientX, y: e.clientY };
        } : undefined}
        onMouseMove={canDrag ? (e) => {
          const s = dragRef.current;
          if (!s) return;
          const dx = e.clientX - s.x, dy = e.clientY - s.y;
          if (dx * dx + dy * dy >= 16) { // ~4px → a drag, not a click
            dragRef.current = null;
            invoke("begin_drag").catch(() => {});
          }
        } : undefined}
        onMouseUp={canDrag ? () => { dragRef.current = null; } : undefined}
        style={{
        width: "100%", height: "100%", overflowY: "auto",
        borderRadius: 12, background: dark ? "#1f2226" : "#ffffff",
        border: `1px solid ${dark ? "rgba(255,255,255,0.10)" : "rgba(0,0,0,0.08)"}`,
        padding: 0, color: t.text, cursor: canDrag ? "grab" : undefined,
      }}>
        {/* sticky header — stays put while the body scrolls */}
        <div style={{
          position: "sticky", top: 0, zIndex: 10,
          background: dark ? "#1f2226" : "#ffffff",
        }}>
          <div style={{
            display: "flex", alignItems: "center", justifyContent: "space-between",
            padding: "15px 15px 10px",
          }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <TokenGlyph color={t.accent} size={16} />
              <span style={{ font: `600 13px ${t.ui}`, color: t.text, letterSpacing: ".01em" }}>{tr.appName}</span>
            </div>
            <div data-no-drag="" style={{ display: "flex", alignItems: "center", gap: 8, cursor: "default" }}>
              {tab === "Overview" && (
                <Segmented value={period} items={periodItems} itemValues={["Day","Week","Month"]} theme={t}
                  onSelect={(v) => setPeriod(v as any)} />
              )}
              <ThemeToggle pref={themePref} theme={t} td={tr} onCycle={onToggleTheme} />
              <LangToggle lang={lang} onClick={toggleLang} theme={t} />
              <ScreenshotButton theme={t} busy={shotBusy} onClick={captureScreenshot} td={tr} />
            </div>
          </div>
          {/* tab bar */}
          <div style={{
            padding: "0 10px 10px",
            borderBottom: `1px solid ${t.gridLine}`,
          }}>
            <Segmented value={tab} items={[tr.overview, tr.agents, tr.sessionsTab]}
              itemValues={["Overview","Agents","Sessions"]} theme={t}
              onSelect={(v) => setTab(v as any)} />
          </div>
        </div>
        {/* scrolling body */}
        <div style={{ padding: "14px 15px 15px" }}>
        {tab === "Overview" && <>
        {/* hero */}
        <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", marginBottom: 10 }}>
          <div>
            <div style={{ font: `500 10px ${t.ui}`, color: t.dim, letterSpacing: ".04em", textTransform: "uppercase" }}>{tr.totalTokens}</div>
            <div style={{ display: "flex", alignItems: "baseline", gap: 8, marginTop: 3 }}>
              <span style={{ font: `600 30px ${t.mono}`, color: t.text, letterSpacing: "-.01em" }}>{animTotal.toFixed(2)}<span style={{ font: `500 15px ${t.mono}`, color: t.dim, marginLeft: 2 }}>M</span></span>
              {Math.round(M.deltaTokens) !== 0 && <Delta v={M.deltaTokens} theme={t} />}
            </div>
          </div>
          <div style={{ textAlign: "right" }}>
            <div style={{ font: `500 10px ${t.ui}`, color: t.dim }}>{tr.estCost}</div>
            <div style={{ font: `600 18px ${t.mono}`, color: t.accent, marginTop: 2 }}>{tr.currencySymbol}{(M.cost * tr.exchangeRate).toFixed(2)}</div>
            <div style={{ font: `500 9.5px ${t.mono}`, color: t.faint, marginTop: 3, display: "flex", alignItems: "center", gap: 6, justifyContent: "flex-end" }}>
              {tr.balance} {balance ? fmtMoney(balance.totalBalance, balSym) : tr.costDash}
              <Switch on={trayMode === "balance"} theme={t} onClick={onToggleTrayMode} title={tr.trayModeHint} />
            </div>
          </div>
        </div>
        {/* cached vs rest (uncached input + output) — 2-colour pill. Dark segment
            is the cache share, matching the "% cached" label below. */}
        <div style={{ display: "flex", height: 7, borderRadius: 4, overflow: "hidden", marginBottom: 5, background: t.gridLine }}>
          {M.totalTokens > 0 && <>
            <div style={{ width: `${cachePct}%`, background: t.accent }} />
            <div style={{ width: `${restPct}%`, background: t.accentSoft }} />
          </>}
        </div>
        <SplitLegend t={t} tr={tr} cacheM={M.cacheTokens} restM={M.inputTokens + M.outputTokens}
          cachedPct={pct(M.cacheTokens, M.totalTokens)}
          reasoningM={M.reasoningTokens > 0 ? M.reasoningTokens : undefined} />
        {/* day boundary: local calendar vs UTC "platform day" (matches DeepSeek) */}
        {period === "Day" && (
          <div style={{ display: "flex", justifyContent: "flex-end", marginBottom: 8 }}>
            <Segmented value={dayMode} items={[tr.dayLocal, tr.dayUtc]} itemValues={["local", "utc"]} theme={t}
              onSelect={(v) => onDayModeChange(v as "local" | "utc")} />
          </div>
        )}
        {/* bar chart */}
        <BarChart data={P.series} theme={t} height={84} td={tr} />
        <SectionRule t={t} m="14px 0 10px" />
        {/* models / projects — tabbed */}
        <div style={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", marginBottom: 4 }}>
          <Label t={t}>{costTab === "model" ? tr.tokensByModel : costTab === "project" ? tr.byProject : tr.tokensByAgent}</Label>
          <Segmented value={costTab} items={[tr.model, tr.project, tr.byAgent]} itemValues={["model", "project", "agent"]} theme={t} onSelect={(v) => setCostTab(v as "model" | "project" | "agent")} />
        </div>
        {costTab === "model" ? (
          <>
            {tokenModels.length === 0 && <div style={{ font: `500 10.5px ${t.mono}`, color: t.faint, padding: "4px 0" }}>{tr.noUsageInThisPeriod}</div>}
            {tokenModels.map((m, i) => <ModelRow key={i} m={m} max={maxM} theme={t} share={tokenShares[i]} />)}
          </>
        ) : costTab === "project" ? (
          <>
            {projectTokens.length === 0 && <div style={{ font: `500 10.5px ${t.mono}`, color: t.faint, padding: "4px 0" }}>{tr.noUsageInThisPeriod}</div>}
            {projectTokens.map((p, i) => <ProjectRow key={i} p={p} max={maxP} theme={t} share={projectShares[i]} />)}
          </>
        ) : (
          <>
            {agentStats.length === 0 && <div style={{ font: `500 10.5px ${t.mono}`, color: t.faint, padding: "4px 0" }}>{tr.noUsageInThisPeriod}</div>}
            {agentStats.map((a, i) => <AgentRow key={i} a={a} max={maxA} theme={t} share={agentShares[i]} color={agentColor(i)} />)}
          </>
        )}
        <SectionRule t={t} m="10px 0 10px" />
        {/* cost donut — same tab */}
        <div style={{ marginBottom: 8 }}><Label t={t}>{costTab === "model" ? tr.costByModel : costTab === "project" ? tr.costByProject : tr.costByAgent}</Label></div>
        {costTab === "model" ? (
          costModels.length > 0
            ? <CostDonut models={costModels} theme={t} size={100} thickness={15}
                currencySymbol={tr.currencySymbol} exchangeRate={tr.exchangeRate} />
            : <div style={{ font: `500 10.5px ${t.mono}`, color: t.faint }}>{tr.costDash}</div>
        ) : costTab === "project" ? (
          projectCostItems.length > 0
            ? <CostDonut models={projectCostItems} theme={t} size={100} thickness={15}
                currencySymbol={tr.currencySymbol} exchangeRate={tr.exchangeRate} />
            : <div style={{ font: `500 10.5px ${t.mono}`, color: t.faint }}>{tr.costDash}</div>
        ) : (
          agentCostItems.length > 0
            ? <CostDonut models={agentCostItems} theme={t} size={100} thickness={15}
                currencySymbol={tr.currencySymbol} exchangeRate={tr.exchangeRate} preserveColors />
            : <div style={{ font: `500 10.5px ${t.mono}`, color: t.faint }}>{tr.costDash}</div>
        )}
        {costTab === "model" && unpricedModels.length > 0 && (
          <div style={{ marginTop: 9, font: `500 9.5px/1.5 ${t.mono}`, color: t.faint }}>
            {tr.modelsWithoutPricing(unpricedModels.length)}{" "}
            <span style={{ color: t.dim }}>{unpricedModels.map((m) => m.name).join(", ")}</span>
          </div>
        )}
        <SectionRule t={t} m="12px 0 12px" />
        {/* footer stats */}
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
          <MiniStat label={tr.requests} value={fmtInt(M.requests)} sub={`${M.sessions} ${tr.sessions}`} theme={t}>
            <Sparkline values={P.reqTrend.length ? P.reqTrend : [0, 0]} theme={t} width={52} height={20} accent={t.accent} />
          </MiniStat>
          <MiniStat label={tr.costTrend} value={`${tr.currencySymbol}${(M.cost * tr.exchangeRate).toFixed(2)}`} sub={trendSub} theme={t} accent={t.accent}>
            <Sparkline values={P.costTrend.length ? P.costTrend : [0, 0]} theme={t} width={52} height={20} accent={t.accent} />
          </MiniStat>
        </div>
        {/* MCP — shown whenever the user has installed MCP servers */}
        {M.servers > 0 && (
          <>
            <SectionRule t={t} />
            <div style={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", marginBottom: 7 }}>
              <Label t={t}>{tr.mcpCalls}</Label>
              <span style={{ font: `500 10px ${t.mono}`, color: t.faint, whiteSpace: "nowrap" }}><span style={{ color: t.text, fontWeight: 600 }}>{fmtInt(M.mcpCalls)}</span> · {M.servers} {tr.servers}</span>
            </div>
            {P.mcp.length > 0
              ? <BarList key={period} items={P.mcp} theme={t} accent={t.accent} td={tr} />
              : <div style={{ font: `500 10px ${t.mono}`, color: t.faint, padding: "2px 0" }}>{tr.noMcpCalls}</div>}
          </>
        )}
        {/* Skill — shown whenever the user has installed skills */}
        {M.skills > 0 && (
          <>
            <SectionRule t={t} />
            <div style={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", marginBottom: 7 }}>
              <Label t={t}>{tr.skillCalls}</Label>
              <span style={{ font: `500 10px ${t.mono}`, color: t.faint, whiteSpace: "nowrap" }}><span style={{ color: t.text, fontWeight: 600 }}>{fmtInt(M.skillCalls)}</span> · {M.skills} {tr.skills}</span>
            </div>
            {P.skills.length > 0
              ? <BarList key={period} items={P.skills} theme={t} accent={t.accent} td={tr} />
              : <div style={{ font: `500 10px ${t.mono}`, color: t.faint, padding: "2px 0" }}>{tr.noSkillCalls}</div>}
          </>
        )}
        {/* heatmap */}
        <SectionRule t={t} />
        <div style={{ marginBottom: 9 }}><Label t={t}>{tr.dailyActivity}</Label></div>
        <Heatmap days={dash.heatmap} theme={t} accent={t.accent} td={tr} />
        {/* refresh, autostart, quit */}
        <SectionRule t={t} m="14px 0 10px" />
        {/* footer: refresh (icon) · launch-at-login (label + switch) · quit (icon) */}
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <IconButton theme={t} title={tr.refresh} onClick={handleRefresh} disabled={refreshing}>
            <svg className={refreshing ? "om-spin" : undefined} width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M21 12a9 9 0 1 1-2.64-6.36" />
              <path d="M21 3v6h-6" />
            </svg>
          </IconButton>
          <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center", gap: 7 }}>
            <span style={{ font: `600 11px ${t.ui}`, color: t.dim }}>{tr.launchAtLogin}</span>
            <Switch on={autostartOn} theme={t} onClick={handleAutostart} title={tr.launchAtLogin} />
          </div>
          <IconButton theme={t} title={tr.quit} onClick={handleQuit} danger>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12 2v10" />
              <path d="M6.34 5.34a8 8 0 1 0 11.32 0" />
            </svg>
          </IconButton>
        </div>
        <div style={{ marginTop: 10, textAlign: "center", font: `500 9px ${t.ui}`, color: t.faint }}>
          OCScale v{version || "dev"} · © 2026
        </div>
        </>}
        {tab === "Agents" && <AgentsTab dash={dash} theme={t} tr={tr} />}
        {tab === "Sessions" && <SessionsTab dash={dash} theme={t} tr={tr} />}
      </div>{/* /scrolling body */}
      </div>
      {toast && (
        <div className="om-toast" style={{
          position: "absolute", top: 58, left: "50%", transform: "translateX(-50%)",
          zIndex: 20, whiteSpace: "nowrap", pointerEvents: "none",
          font: `600 12px ${t.mono}`, color: "#fff",
          background: toast.ok ? t.accent : "#e0795f",
          padding: "7px 13px", borderRadius: 9,
          boxShadow: "0 8px 22px rgba(0,0,0,0.34)",
        }}>
          {toast.msg}
        </div>
      )}
    </div>
  );
}
// ── Agents tab ────────────────────────────────────────────────────
function AgentRow({ a, max, theme, share, color }: { a: AgentStat; max: number; theme: Theme; share: number; color: string }) {
  const pctStr = share % 1 === 0 ? share.toFixed(0) : share.toFixed(1);
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 9, padding: "5px 0" }}>
      <span style={{ width: 7, height: 7, borderRadius: 2, background: color, flex: "0 0 auto" }} />
      <div style={{ minWidth: 0, flex: "0 0 118px" }}>
        <div style={{ font: `500 11.5px ${theme.ui}`, color: theme.text, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{a.agent}</div>
      </div>
      <div style={{ flex: 1, height: 5, borderRadius: 3, background: theme.gridLine, overflow: "hidden" }}>
        <div style={{ width: `${(a.tokens / max) * 100}%`, height: "100%", background: color, borderRadius: 3 }} />
      </div>
      <span style={{ font: `500 10.5px ${theme.mono}`, color: theme.dim, flex: "0 0 auto", width: 42, textAlign: "right" }}>{fmtTokens(a.tokens)}</span>
      <span style={{ font: `600 10.5px ${theme.mono}`, color: theme.text, flex: "0 0 auto", width: 40, textAlign: "right" }}>{pctStr}%</span>
    </div>
  );
}

function AgentsTab({ dash, theme, tr }: { dash: Dashboard; theme: Theme; tr: Dict }) {
  const agents = dash.agents || [];
  const max = Math.max(...agents.map(a => a.tokens), 1e-9);
  const shares = sharePcts(agents.map(a => a.tokens));
  // Blue palette by rank (darkest → lightest), consistent between rows and donut
  const BLUES = ["#1e3a8a", "#1e40af", "#2563eb", "#3b82f6", "#60a5fa", "#93c5fd", "#bfdbfe"];
  const agentColor = (i: number) => BLUES[i % BLUES.length];
  // Build donut items from agents with cost > 0
  const costAgents: ModelStat[] = agents
    .filter(a => a.cost > 0)
    .map((a, i) => ({
        name: a.agent,
        vendor: "",
        tokens: a.tokens,
        cost: a.cost,
        color: agentColor(i),
        priced: true,
        costSource: "pricing",
      }
    ));
  return (
    <>
      <div style={{ marginBottom: 9 }}><Label t={theme}>{tr.tokensByAgent}</Label></div>
      {agents.length === 0 ? (
        <div style={{ font: `500 10.5px ${theme.mono}`, color: theme.faint, padding: "4px 0" }}>{tr.noUsageInThisPeriod}</div>
      ) : (
        agents.map((a, i) => <AgentRow key={i} a={a} max={max} theme={theme} share={shares[i]} color={agentColor(i)} />)
      )}
      {costAgents.length > 0 && (
        <>
          <SectionRule t={theme} m="10px 0 10px" />
          <div style={{ marginBottom: 8 }}><Label t={theme}>{tr.costByAgent}</Label></div>
          <CostDonut models={costAgents} theme={theme} size={100} thickness={15}
            currencySymbol={tr.currencySymbol} exchangeRate={tr.exchangeRate}
            preserveColors />
        </>
      )}
      <SectionRule t={theme} />
    </>
  );
}

// ── Sessions tab ──────────────────────────────────────────────────
function fmtDuration(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m`;
}
function fmtTimeAgo(iso: string): string {
  const d = new Date(iso);
  const now = Date.now();
  const diff = now - d.getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "now";
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  if (days < 7) return `${days}d ago`;
  return d.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}
function SessionRow({ s, theme, tr }: { s: SessionInfo; theme: Theme; tr: Dict }) {
  const title = s.sessionTitle || "Untitled";
  return (
    <div style={{
      display: "flex", alignItems: "center", gap: 8, padding: "7px 0",
      borderBottom: `1px solid ${theme.gridLine}`,
    }}>
      <div style={{ minWidth: 0, flex: 1 }}>
        <div style={{ font: `500 11.5px ${theme.ui}`, color: theme.text, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{title}</div>
        <div style={{ display: "flex", gap: 6, marginTop: 2, flexWrap: "wrap" }}>
          {s.projectName && (
            <span style={{ font: `500 9px ${theme.mono}`, color: theme.dim, background: theme.gridLine, padding: "1px 5px", borderRadius: 4 }}>{s.projectName}</span>
          )}
          {s.agent && (
            <span style={{ font: `500 9px ${theme.mono}`, color: theme.accent, background: `${theme.accent}22`, padding: "1px 5px", borderRadius: 4 }}>{s.agent}</span>
          )}
          <span style={{ font: `500 9px ${theme.mono}`, color: theme.faint }}>{fmtTimeAgo(s.timeCreated)}</span>
          <span style={{ font: `500 9px ${theme.mono}`, color: theme.faint }}>{fmtDuration(s.durationSecs)}</span>
        </div>
      </div>
      <div style={{ textAlign: "right", flex: "0 0 auto" }}>
        <div style={{ font: `600 10.5px ${theme.mono}`, color: theme.text }}>{fmtTokens(s.tokens)}</div>
        {s.cost > 0 && (
          <div style={{ font: `500 9px ${theme.mono}`, color: theme.accent }}>{tr.currencySymbol}{s.cost.toFixed(2)}</div>
        )}
      </div>
    </div>
  );
}

function SessionsTab({ dash, theme, tr }: { dash: Dashboard; theme: Theme; tr: Dict }) {
  const sessions = dash.recentSessions || [];
  return (
    <>
      <div style={{ marginBottom: 9 }}><Label t={theme}>{tr.recentSessions}</Label></div>
      {sessions.length === 0 ? (
        <div style={{ font: `500 10.5px ${theme.mono}`, color: theme.faint, padding: "4px 0" }}>{tr.noSessions}</div>
      ) : (
        sessions.map((s, i) => <SessionRow key={i} s={s} theme={theme} tr={tr} />)
      )}
      <SectionRule t={theme} />
    </>
  );
}

export default function App() {
  const [curLang, setCurLang] = useState<Lang>(() => {
    const saved = typeof localStorage !== "undefined" ? localStorage.getItem("ocscale-lang") : null;
    return saved === "zh" ? "zh" : "en";
  });
  const toggleLang = () =>
    setCurLang((p) => {
      const n = p === "en" ? "zh" : "en";
      try { localStorage.setItem("ocscale-lang", n); } catch {}
      return n;
    });
  const tr = DICT[curLang];
  const [dash, setDash] = useState<Dashboard | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [version, setVersion] = useState("");
  const [balance, setBalance] = useState<BalanceInfo | null>(null);
  const [trayMode, setTrayMode] = useState<"tokens" | "balance">("tokens");
  const [dayMode, setDayMode] = useState<"local" | "utc">("local");
  const applyDash = (d: Dashboard) => { setDash(d); setErr(null); };
  const [openGen, setOpenGen] = useState(0);
  const [focused, setFocused] = useState(true); // browser preview: always "focused"
  // Theme preference: explicit Dark / Light, or System (follows the OS
  // appearance live on both macOS and Windows via prefers-color-scheme). First
  // run defaults to System.
  const [themePref, setThemePref] = useState<"dark" | "light" | "system">(() => {
    const saved = typeof localStorage !== "undefined" ? localStorage.getItem("ocscale-theme") : null;
    if (saved === "dark" || saved === "light" || saved === "system") return saved;
    return "system";
  });
  const [systemDark, setSystemDark] = useState<boolean>(
    () => typeof window !== "undefined" && !!window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches
  );
  // Follow the OS appearance live while in System mode (and keep it current for
  // an instant switch back to System).
  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);
  const dark = themePref === "system" ? systemDark : themePref === "dark";
  // Cycle Dark → Light → System on each click; persist the choice.
  const cycleTheme = () =>
    setThemePref((p) => {
      const n = p === "dark" ? "light" : p === "light" ? "system" : "dark";
      try { localStorage.setItem("ocscale-theme", n); } catch {}
      return n;
    });
  // Switch what the menu-bar / tray label shows (today's tokens vs balance).
  const toggleTrayMode = () => {
    const next = trayMode === "tokens" ? "balance" : "tokens";
    invoke<string>("set_tray_mode", { mode: next })
      .then((m) => setTrayMode(m === "balance" ? "balance" : "tokens"))
      .catch(() => {});
  };
  // Switch the Day view (and tray "today") between the local calendar day and
  // the UTC boundary DeepSeek's platform uses, then refetch with the new mode.
  const changeDayMode = (m: "local" | "utc") => {
    setDayMode(m);
    invoke<string>("set_day_mode", { mode: m }).catch(() => {});
    fetchDashboard().then(applyDash).catch(() => {});
  };

  useEffect(() => {
    // initial load (shows the Loading state only until the first data arrives)
    fetchDashboard().then(applyDash).catch((e) => setErr(String(e)));

    const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    if (inTauri) {
      invoke<string>("get_version").then(setVersion).catch(() => {});
      invoke<BalanceInfo>("get_balance").then(setBalance).catch(() => {});
      invoke<string>("get_tray_mode").then((m) => setTrayMode(m === "balance" ? "balance" : "tokens")).catch(() => {});
      invoke<string>("get_day_mode").then((m) => setDayMode(m === "utc" ? "utc" : "local")).catch(() => {});
    } else {
      setVersion("dev");
    }
    if (!inTauri) return;

    // Under StrictMode the effect mounts → cleans up → remounts; the async
    // listen()/onFocusChanged() promises can resolve after the first cleanup,
    // so unregister any late arrival immediately instead of leaking a duplicate.
    let dead = false;
    const unlisten: Array<() => void> = [];
    const track = (u: () => void) => {
      if (dead) u();
      else unlisten.push(u);
    };
    // live updates pushed from the background refresh thread — swaps the data in
    // place (no Loading), so values update without any flicker.
    listen<Dashboard>("dashboard-updated", (e) => applyDash(e.payload)).then(track);
    // System appearance pushed natively from Rust (macOS). The webview's
    // prefers-color-scheme is unreliable for our hidden, non-activating menu-bar
    // panel, so the native event is the source of truth for System mode there;
    // it fires once at startup (correcting any stale launch value) and on every
    // OS theme change. Harmlessly never fires on Windows, where matchMedia works.
    listen<boolean>("system-theme", (e) => setSystemDark(e.payload)).then(track);
    // refetch the instant the popover gains focus (i.e. is opened)
    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        setFocused(focused);
        if (focused) {
          setOpenGen((g) => g + 1); // re-run the count-up on each open
          fetchDashboard().then(applyDash).catch(() => {});
          invoke<BalanceInfo>("get_balance").then(setBalance).catch(() => {});
        }
      })
      .then(track);
    return () => {
      dead = true;
      unlisten.forEach((u) => u());
    };
  }, []);

  // window is transparent; the rounded card paints its own background
  useEffect(() => {
    document.body.style.background = "transparent";
  }, [dark]);

  // Suppress per-property CSS transitions across a theme flip so the panel
  // repaints in the new theme in one step instead of cross-fading each color
  // (see .ts-no-transition in main.tsx). A background light→dark switch lands
  // while the panel is hidden; rAF callbacks don't run while hidden, so the
  // class stays on until the popover is shown — the first painted frame is
  // already the new theme with no transition, then we strip it a couple of
  // frames later so live interactions (e.g. switching the period) animate as
  // before. Skipped on the very first render (no prior frame to cross-fade).
  const firstThemeRun = useRef(true);
  useEffect(() => {
    if (firstThemeRun.current) {
      firstThemeRun.current = false;
      return;
    }
    const el = document.documentElement;
    el.classList.add("ts-no-transition");
    const id = requestAnimationFrame(() =>
      requestAnimationFrame(() => el.classList.remove("ts-no-transition"))
    );
    return () => cancelAnimationFrame(id);
  }, [dark]);

  const t = TH[dark ? "dark" : "light"];
  const i18nCtx: I18nCtx = { lang: curLang, t: tr, toggleLang };
  return (
    <I18nContext.Provider value={i18nCtx}>
      {err
        ? <div style={{ padding: 20, font: `500 12px ${t.mono}`, color: "#e0795f" }}>{tr.failedToLoad} {err}</div>
        : !dash
        ? <div style={{ height: "100vh", padding: 10, boxSizing: "border-box", background: "transparent" }}>
            <div style={{ height: "100%", borderRadius: 14, background: dark ? "#1f2226" : "#ffffff",
              display: "flex", alignItems: "center", justifyContent: "center",
              font: `500 12px ${t.mono}`, color: t.dim }}>{tr.loading}</div>
          </div>
        : <Panel dash={dash} dark={dark} themePref={themePref} onToggleTheme={cycleTheme}
            openGen={openGen} active={focused} lang={curLang} toggleLang={toggleLang}
            version={version} balance={balance} trayMode={trayMode} onToggleTrayMode={toggleTrayMode}
            dayMode={dayMode} onDayModeChange={changeDayMode}
            onRefresh={() => fetchDashboard().then(applyDash)} />}
    </I18nContext.Provider>
  );
}
