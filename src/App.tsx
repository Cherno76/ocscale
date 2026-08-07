import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
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
  const col = up ? theme.danger : theme.success;
  return (
    <span style={{ font: `600 ${theme.fs.small}px ${theme.mono}`, color: col, display: "inline-flex", alignItems: "center", gap: 3,
      padding: "2px 6px", borderRadius: theme.r.sm,
      background: up ? `${theme.danger}22` : `${theme.success}1f` }}>
      {up ? "▲" : "▼"}{Math.abs(Math.round(v))}%
    </span>
  );
}

// macOS: the panel carries a real NSVisualEffectView frosted-glass backdrop
// (applied in Rust), so the card background is translucent to let the blur
// show through. Elsewhere (Windows/Linux, browser preview) keep it solid.
function panelBackground(dark: boolean, t: Theme): string {
  const isMac = typeof navigator !== "undefined" && navigator.userAgent.includes("Macintosh");
  return isMac ? (dark ? "rgba(31,34,38,0.84)" : "rgba(255,255,255,0.80)") : t.card;
}

// Round each value's share to 1 decimal (%) via largest-remainder apportionment,
// so the displayed percentages sum to exactly 100.0% (plain rounding wouldn't).
function ProjectRow({ p, max, theme, share }: { p: ProjectStat; max: number; theme: Theme; share: number }) {
  const pctStr = share % 1 === 0 ? share.toFixed(0) : share.toFixed(1);
  const PALETTE = ["#1e40af", "#2563eb", "#3b82f6", "#60a5fa", "#4b5a52", "#a78bfa", "#e0795f", "#6ee7b7"];
  const hash = p.projectId.split("").reduce((a, c) => a + c.charCodeAt(0), 0);
  const color = PALETTE[hash % PALETTE.length];
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 9, padding: "6px 0" }}>
      <span style={{ width: 8, height: 8, borderRadius: 3, background: color, flex: "0 0 auto" }} />
      <div style={{ minWidth: 0, flex: "0 0 118px" }}>
        <div style={{ font: `500 ${theme.fs.body}px ${theme.ui}`, color: theme.text, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{p.projectName}</div>
      </div>
      <div style={{ flex: 1, height: 6, borderRadius: 3, background: theme.gridLine, overflow: "hidden" }}>
        <div style={{ width: `${(p.tokens / max) * 100}%`, height: "100%", background: color, borderRadius: 3 }} />
      </div>
      <span style={{ font: `500 ${theme.fs.small}px ${theme.mono}`, color: theme.dim, flex: "0 0 auto", width: 44, textAlign: "right" }}>{fmtTokens(p.tokens)}</span>
      <span style={{ font: `600 ${theme.fs.small}px ${theme.mono}`, color: theme.text, flex: "0 0 auto", width: 42, textAlign: "right" }}>{pctStr}%</span>
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
    <div style={{ display: "flex", alignItems: "center", gap: 9, padding: "6px 0" }}>
      <span style={{ width: 8, height: 8, borderRadius: 3, background: m.color, flex: "0 0 auto" }} />
      <div style={{ minWidth: 0, flex: "0 0 118px" }}>
        <div style={{ font: `500 ${theme.fs.body}px ${theme.ui}`, color: theme.text, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{m.name}</div>
      </div>
      <div style={{ flex: 1, height: 6, borderRadius: 3, background: theme.gridLine, overflow: "hidden" }}>
        <div style={{ width: `${(m.tokens / max) * 100}%`, height: "100%", background: m.color, borderRadius: 3 }} />
      </div>
      <span style={{ font: `500 ${theme.fs.small}px ${theme.mono}`, color: theme.dim, flex: "0 0 auto", width: 44, textAlign: "right" }}>{fmtTokens(m.tokens)}</span>
      <span style={{ font: `600 ${theme.fs.small}px ${theme.mono}`, color: theme.text, flex: "0 0 auto", width: 42, textAlign: "right" }}>{pctStr}%</span>
    </div>
  );
}

// Elevated KPI card: label row (with optional sparkline / adornment on the
// right) over a large mono value and a faint sub-line.
function KpiCard({ label, value, sub, theme, accent, children }:
  { label: string; value: string; sub?: string; theme: Theme; accent?: string; children?: React.ReactNode }) {
  const t = theme;
  return (
    <div style={{
      background: t.surface, border: `1px solid ${t.border}`, borderRadius: t.r.md,
      padding: "11px 12px", minWidth: 0,
    }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 6, marginBottom: 7 }}>
        <span style={{ font: `600 ${t.fs.label}px ${t.ui}`, color: t.dim, letterSpacing: ".03em", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{label}</span>
        {children}
      </div>
      <div style={{ font: `700 ${t.fs.value}px/1 ${t.mono}`, color: accent || t.text, letterSpacing: "-.01em" }}>{value}</div>
      {sub && <div style={{ font: `500 ${t.fs.small}px ${t.mono}`, color: t.faint, marginTop: 5, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{sub}</div>}
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
      font: `500 ${t.fs.small}px ${t.mono}`, color: t.dim, marginBottom: 14, whiteSpace: "nowrap", overflow: "hidden",
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
  <span style={{ font: `600 ${t.fs.label}px ${t.ui}`, color: t.dim, letterSpacing: ".04em", textTransform: "uppercase", whiteSpace: "nowrap" }}>{children}</span>
);
// Tab pill label: small icon + text, inline in the Segmented control.
function TabItem({ icon, label, t }: { icon: React.ReactNode; label: string; t: Theme }) {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 5, verticalAlign: "middle" }}>
      {icon}
      <span>{label}</span>
    </span>
  );
}

function ThemeToggle({ pref, theme, td, onCycle }: { pref: "dark" | "light" | "system"; theme: Theme; td: Dict; onCycle: () => void }) {
  const t = theme;
  // Single button cycling Dark → Light → System; the icon shows the current mode.
  const label = pref === "system" ? td.system : pref === "dark" ? td.dark : td.light;
  return (
    <button onClick={onCycle} title={`Theme: ${label}`} aria-label={`theme: ${label}`} style={{
      display: "inline-flex", alignItems: "center", justifyContent: "center",
      width: 28, height: 28, borderRadius: t.r.sm, cursor: "pointer", padding: 0,
      background: t.segBg, border: `1px solid ${t.segBorder}`, color: t.dim,
      transition: "background .15s, color .15s",
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
      width: 28, height: 28, borderRadius: theme.r.sm, cursor: "pointer", padding: 0,
      background: theme.segBg, border: `1px solid ${theme.segBorder}`, color: theme.dim,
      font: `600 ${theme.fs.small}px ${theme.mono}`, letterSpacing: ".02em",
      transition: "background .15s, color .15s",
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
        position: "relative", width: 28, height: 16, borderRadius: theme.r.pill, padding: 0, cursor: "pointer", flex: "0 0 auto",
        background: on ? theme.accent : theme.segBg,
        border: `1px solid ${on ? theme.accent : theme.segBorder}`,
        transition: "background .15s",
      }}>
      <span style={{
        position: "absolute", top: 2, left: on ? 13 : 2, width: 10, height: 10, borderRadius: "50%",
        background: "#fff", transition: "left .15s",
      }} />
    </button>
  );
}

/// Compact square icon button with hover feedback (danger tint for quit).
function IconButton({ theme, title, onClick, danger, disabled, active, children }:
  { theme: Theme; title: string; onClick: () => void; danger?: boolean; disabled?: boolean; active?: boolean; children: React.ReactNode }) {
  const [h, setH] = useState(false);
  const t = theme;
  return (
    <button onClick={onClick} title={title} aria-label={title} disabled={disabled}
      onMouseEnter={() => setH(true)} onMouseLeave={() => setH(false)}
      style={{
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        width: 30, height: 30, borderRadius: t.r.sm, padding: 0, flex: "0 0 auto",
        cursor: disabled ? "default" : "pointer",
        background: h || active ? t.segOnBg : t.segBg, border: `1px solid ${t.segBorder}`,
        color: danger && h ? t.danger : t.dim,
        transition: "background .15s, color .15s",
      }}>
      {children}
    </button>
  );
}

/// Row inside the overflow (⋯) menu: icon + label, optional right adornment.
function MenuItem({ theme, onClick, danger, disabled, ariaLabel, children, right }:
  { theme: Theme; onClick: () => void; danger?: boolean; disabled?: boolean; ariaLabel?: string; children: React.ReactNode; right?: React.ReactNode }) {
  const [h, setH] = useState(false);
  const t = theme;
  return (
    <button onClick={onClick} disabled={disabled} aria-label={ariaLabel}
      onMouseEnter={() => setH(true)} onMouseLeave={() => setH(false)}
      style={{
        display: "flex", width: "100%", alignItems: "center", gap: 9,
        padding: "6px 9px", border: "none", background: h ? t.surfaceAlt : "transparent",
        borderRadius: t.r.sm, cursor: disabled ? "default" : "pointer",
        font: `500 11.5px ${t.ui}`, color: danger ? t.danger : t.text, textAlign: "left",
        transition: "background .12s, color .12s",
      }}>
      {children}
      {right && <span style={{ marginLeft: "auto", display: "inline-flex", alignItems: "center" }}>{right}</span>}
    </button>
  );
}

function Panel({ dash, dark, themePref, onToggleTheme, openGen, active, lang, toggleLang, onRefresh, version, balance, trayMode, onToggleTrayMode, dayMode, onDayModeChange }:
  { dash: Dashboard; dark: boolean; themePref: "dark" | "light" | "system"; onToggleTheme: () => void; openGen: number; active: boolean; lang: Lang; toggleLang: () => void; onRefresh: () => void; version: string; balance: BalanceInfo | null; trayMode: "tokens" | "balance"; onToggleTrayMode: () => void; dayMode: "local" | "utc"; onDayModeChange: (m: "local" | "utc") => void }) {
  const t = TH[dark ? "dark" : "light"];
  const { t: tr } = useT();
  const panelBg = panelBackground(dark, t);
  const [tab, setTab] = useState<"Overview" | "Agents" | "Sessions">("Overview");
  const periodItems = [tr.day, tr.week, tr.month];
  // Drag the popover by its body (Windows/Linux only — macOS uses the menu-bar
  // NSPanel and is gated out). A real OS window-drag begins only once the
  // pointer moves past a small threshold, so a plain click still clicks through
  // / dismisses and never arms the hide-suppression guard.
  const canDrag = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window && !navigator.userAgent.includes("Macintosh");
  const dragRef = useRef<{ x: number; y: number } | null>(null);
  const [period, setPeriod] = useState<"Day" | "Week" | "Month">("Week");
  const [periodOffset, setPeriodOffset] = useState(0);
  const [pagedReport, setPagedReport] = useState<PeriodReport | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [autostartOn, setAutostartOn] = useState(false);
  const [costTab, setCostTab] = useState<"model" | "project" | "agent">("model");
  // Popover entrance: replay the fade/settle animation each time the panel
  // opens (active flips false→true on focus). Reduced-motion is handled
  // globally via the prefers-reduced-motion media query in main.tsx.
  const [entering, setEntering] = useState(true);
  useEffect(() => {
    if (!active) return;
    setEntering(true);
    const id = window.setTimeout(() => setEntering(false), 280);
    return () => window.clearTimeout(id);
  }, [active]);
  const [menuOpen, setMenuOpen] = useState(false);
  // The popover hides on blur — drop any open menu so the next open is clean.
  useEffect(() => { if (!active) setMenuOpen(false); }, [active]);
  // Close the overflow menu on Escape.
  useEffect(() => {
    if (!menuOpen) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setMenuOpen(false); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [menuOpen]);
  useEffect(() => {
    // Read initial autostart state from the Rust backend.
    if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
      import("@tauri-apps/api/core").then(({ invoke }) =>
        invoke<boolean>("get_autostart").then(setAutostartOn).catch(() => {})
      );
    }
  }, []);
  // ── auto-fit the popover height to its content ──────────────────────────
  // The window is 800 wide; height follows the content (data volume, tab,
  // period) so there's no dead space at the bottom. Debounced re-measure after
  // renders that can change the height; Rust clamps 400–800 and re-pins the
  // macOS panel under the tray icon so the top edge never drifts.
  useEffect(() => {
    if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return;
    const id = window.setTimeout(() => {
      const card = document.querySelector<HTMLElement>(".om-scroll");
      if (!card) return;
      const header = card.firstElementChild as HTMLElement | null;
      const body = header?.nextElementSibling as HTMLElement | null;
      if (!header || !body) return;
      // card border is 1px top + 1px bottom
      // + outer padding (10 top + 10 bottom) so the drop shadow has room to paint.
      const height = Math.round(header.offsetHeight + body.offsetHeight + 22);
      invoke("fit_panel", { height }).catch(() => {});
    }, 120);
    return () => window.clearTimeout(id);
  }, [dash, tab, period, costTab, lang, openGen, pagedReport]);
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
  // Paged week/month views override the live period report (historical data is
  // static; the current period keeps updating live from dashboard-updated).
  const P: PeriodReport = pagedReport && period !== "Day"
    ? pagedReport
    : period === "Day" ? dash.day : period === "Month" ? dash.month : dash.week;
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
  // Color by data source (green = OpenCode, orange = Codex), same as the
  // Agents tab; rows and donut share one mapping.
  const agentColors = agentColorsFor(agentStats);
  const agentColorOf = (a: AgentStat) => agentColors.get(a.agent) ?? "#79817b";
  const agentCostItems: ModelStat[] = [];
  agentStats.forEach((a) => {
    if (a.cost > 0) {
      agentCostItems.push({
        name: a.agent, vendor: "", tokens: a.tokens, cost: a.cost,
        color: agentColorOf(a), priced: true, costSource: "pricing",
      });
    }
  });
  // ── period paging (week/month ‹ ›) ──────────────────────────────
  // Date titles follow the panel language (the system locale would otherwise
  // leak Chinese month names into English mode).
  const dateLocale = lang === "zh" ? "zh-CN" : "en-US";
  const fmtMonthTitle = (offset: number) =>
    new Date(new Date().getFullYear(), new Date().getMonth() + offset, 1)
      .toLocaleDateString(dateLocale, { year: "numeric", month: "long" });
  const fmtWeekTitle = (offset: number) => {
    const d = new Date();
    const mon = new Date(d.getFullYear(), d.getMonth(), d.getDate() - ((d.getDay() + 6) % 7) + offset * 7);
    const base = mon.toLocaleDateString(dateLocale, { month: "short", day: "numeric" });
    return offset === 0 ? `${base} (${tr.thisWeek})` : base;
  };
  const periodTitle = period === "Month" ? fmtMonthTitle(periodOffset) : fmtWeekTitle(periodOffset);
  const goPeriod = (delta: number) => {
    const next = periodOffset + delta;
    setPeriodOffset(next);
    invoke<PeriodReport>("get_period", { period: period.toLowerCase(), offset: next })
      .then(setPagedReport)
      .catch(() => {});
  };
  const trendSub = pagedReport && period !== "Day"
    ? periodTitle
    : { Day: tr.today24h, Week: tr.thisWeek, Month: tr.thisMonth }[period];
  // Average cost per request, kept readable across magnitudes (0.0004 → ¥0.0004).
  const avgPerReq = M.requests > 0 ? (M.cost * tr.exchangeRate) / M.requests : 0;
  const avgPerReqStr = avgPerReq >= 0.01 ? avgPerReq.toFixed(2)
    : avgPerReq >= 0.0001 ? avgPerReq.toFixed(4)
    : avgPerReq.toFixed(6);

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
    // Close the ⋯ overflow menu first — an open dropdown would otherwise be
    // baked into the saved PNG, covering the content beneath it. Wait a couple
    // of frames for React to commit the removal before rasterizing.
    if (menuOpen) {
      setMenuOpen(false);
      await new Promise<void>((r) => requestAnimationFrame(() => r()));
      await new Promise<void>((r) => requestAnimationFrame(() => r()));
    }
    const el = document.querySelector<HTMLElement>(".om-scroll");
    if (!el) return;
    setShotBusy(true);
    try {
      // explicit width/height = full scrollable content, not just the viewport;
      // filter is a safety net (menu is already closed above): drop the
      // screenshot menu item if it is still in the DOM mid-render.
      const dataUrl = await domToPng(el, {
        scale: 2,
        backgroundColor: dark ? "#1f2226" : "#ffffff",
        width: el.scrollWidth,
        height: el.scrollHeight,
        filter: (n) => !(n instanceof HTMLElement && n.getAttribute("aria-label") === tr.screenshotTitle),
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
      background: "transparent", padding: 10,
      fontFamily: t.ui,
      ...({ "--om-hover": t.surfaceAlt, "--om-faint": t.faint } as CSSProperties),
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
        borderRadius: t.r.xl, background: panelBg,
        border: `1px solid ${t.border}`, boxShadow: t.shadow,
        animation: entering ? "om-pop-in 0.22s cubic-bezier(0.22, 1, 0.36, 1)" : undefined,
        transformOrigin: "50% 0",
        padding: 0, color: t.text, cursor: canDrag ? "grab" : undefined,
      }}>
        {/* sticky header — stays put while the body scrolls */}
        <div style={{
          position: "sticky", top: 0, zIndex: 10,
          background: panelBg,
        }}>
          <div style={{
            display: "flex", alignItems: "center", justifyContent: "space-between",
            padding: "16px 18px 12px",
          }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <TokenGlyph color={t.accent} size={18} />
              <span style={{ font: `600 14px ${t.ui}`, color: t.text, letterSpacing: ".01em" }}>{tr.appName}</span>
              <span style={{ font: `500 10.5px ${t.mono}`, color: t.faint, marginLeft: 2 }}>v{version || "dev"} · © 2026 HduSy · Cherno76 · MIT</span>
            </div>
            <div data-no-drag="" style={{ display: "flex", alignItems: "center", gap: 8, cursor: "default" }}>
              <Segmented value={tab} itemValues={["Overview","Agents","Sessions"]} theme={t}
                onSelect={(v) => setTab(v as any)}
                items={[
                  <TabItem key="ov" t={t} icon={
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                      <rect x="3" y="3" width="7" height="7" rx="1.5" />
                      <rect x="14" y="3" width="7" height="7" rx="1.5" />
                      <rect x="3" y="14" width="7" height="7" rx="1.5" />
                      <rect x="14" y="14" width="7" height="7" rx="1.5" />
                    </svg>} label={tr.overview} />,
                  <TabItem key="ag" t={t} icon={
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <circle cx="12" cy="8" r="4" />
                      <path d="M4 21c1.6-3.7 4.6-5.4 8-5.4s6.4 1.7 8 5.4" />
                    </svg>} label={tr.agents} />,
                  <TabItem key="se" t={t} icon={
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                      <circle cx="12" cy="12" r="9" />
                      <path d="M12 7v5l3.2 2" />
                    </svg>} label={tr.sessionsTab} />,
                ]} />
              <ThemeToggle pref={themePref} theme={t} td={tr} onCycle={onToggleTheme} />
              <LangToggle lang={lang} onClick={toggleLang} theme={t} />
            </div>
          </div>
          {/* period row: day/week/month on the left (Overview only), launch-at-
              login + refresh/quit pinned to the far right */}
          <div style={{
            display: "flex", alignItems: "center", justifyContent: "space-between",
            padding: "0 12px 12px",
            borderBottom: `1px solid ${t.gridLine}`,
          }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8, minHeight: 28 }}>
              {tab === "Overview" && (
                <Segmented value={period} items={periodItems} itemValues={["Day","Week","Month"]} theme={t}
                  onSelect={(v) => { setPeriod(v as any); setPeriodOffset(0); setPagedReport(null); }} />
              )}
            </div>
            <div data-no-drag="" style={{ display: "flex", alignItems: "center", gap: 6, cursor: "default" }}>
              <IconButton theme={t} title={tr.refresh} onClick={handleRefresh} disabled={refreshing}>
                <svg className={refreshing ? "om-spin" : undefined} width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M21 12a9 9 0 1 1-2.64-6.36" />
                  <path d="M21 3v6h-6" />
                </svg>
              </IconButton>
              <div style={{ position: "relative" }}>
                <IconButton theme={t} title={tr.moreActions} onClick={() => setMenuOpen((o) => !o)} active={menuOpen} aria-expanded={menuOpen}>
                  <svg width="14" height="14" viewBox="0 0 24 24">
                    <circle cx="5" cy="12" r="1.5" fill="currentColor" />
                    <circle cx="12" cy="12" r="1.5" fill="currentColor" />
                    <circle cx="19" cy="12" r="1.5" fill="currentColor" />
                  </svg>
                </IconButton>
                {menuOpen && (
                  <>
                    {/* click-away layer: below the menu, above everything else */}
                    <div style={{ position: "fixed", inset: 0, zIndex: 9 }} onClick={() => setMenuOpen(false)} />
                    <div style={{
                      position: "absolute", right: 0, top: "calc(100% + 6px)", zIndex: 11, width: 184,
                      background: t.surface, border: `1px solid ${t.border}`, borderRadius: t.r.md,
                      boxShadow: t.shadow, padding: 4,
                    }}>
                      <MenuItem theme={t} onClick={captureScreenshot} disabled={shotBusy} ariaLabel={tr.screenshotTitle}
                        right={shotBusy ? <svg className="om-spin" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke={t.dim} strokeWidth="2.6" strokeLinecap="round"><path d="M12 3a9 9 0 1 0 9 9" /></svg> : undefined}>
                        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round">
                          <path d="M3 8.5A2.5 2.5 0 0 1 5.5 6h1.7l1.1-1.6A1.5 1.5 0 0 1 9.5 4h5a1.5 1.5 0 0 1 1.2.4L16.8 6h1.7A2.5 2.5 0 0 1 21 8.5v8A2.5 2.5 0 0 1 18.5 19h-13A2.5 2.5 0 0 1 3 16.5z" />
                          <circle cx="12" cy="12.2" r="3.4" />
                        </svg>
                        {tr.screenshotTitle}
                      </MenuItem>
                      <MenuItem theme={t} onClick={handleAutostart}
                        right={
                          <span style={{
                            position: "relative", width: 26, height: 15, borderRadius: 999, flex: "0 0 auto",
                            background: autostartOn ? t.accent : t.segBg,
                            border: `1px solid ${autostartOn ? t.accent : t.segBorder}`,
                          }}>
                            <span style={{ position: "absolute", top: 2, left: autostartOn ? 13 : 2, width: 9, height: 9, borderRadius: "50%", background: "#fff" }} />
                          </span>
                        }>
                        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                          <path d="M7 17L17 7" />
                          <path d="M9 7h8v8" />
                        </svg>
                        {tr.launchAtLogin}
                      </MenuItem>
                      <div style={{ height: 1, background: t.gridLine, margin: "3px 4px" }} />
                      <MenuItem theme={t} onClick={handleQuit} danger>
                        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
                          <path d="M12 2v10" />
                          <path d="M6.34 5.34a8 8 0 1 0 11.32 0" />
                        </svg>
                        {tr.quit}
                      </MenuItem>
                    </div>
                  </>
                )}
              </div>
            </div>
          </div>
        </div>
        {/* scrolling body — two columns now the panel is wider: left holds the
            period headline + token bars, right holds cost/stats/tools/activity.
            Inner lists are height-capped so the whole panel usually fits without
            scrolling; the outer container still scrolls as a fallback. */}
        <div style={{ padding: "16px 18px 18px" }}>
        {tab === "Overview" && <>
        <div style={{ display: "grid", gridTemplateColumns: "minmax(0, 1.35fr) minmax(0, 1fr)", gap: "0 18px", alignItems: "start" }}>
          {/* ── left column ── */}
          <div style={{ minWidth: 0, borderRight: `1px solid ${t.gridLine}`, paddingRight: 18 }}>
            {/* hero */}
            <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between", marginBottom: 10 }}>
              <div>
                <div style={{ font: `600 ${t.fs.label}px ${t.ui}`, color: t.dim, letterSpacing: ".04em", textTransform: "uppercase" }}>{tr.totalTokens}</div>
                <div style={{ display: "flex", alignItems: "center", gap: 10, marginTop: 4 }}>
                  <span style={{ font: `700 ${t.fs.hero}px/1 ${t.mono}`, color: t.text, letterSpacing: "-.02em" }}>{animTotal.toFixed(2)}<span style={{ font: `600 17px ${t.mono}`, color: t.dim, marginLeft: 3 }}>M</span></span>
                  {Math.round(M.deltaTokens) !== 0 && <Delta v={M.deltaTokens} theme={t} />}
                </div>
              </div>
              <div style={{ textAlign: "right" }}>
                <div style={{ font: `600 ${t.fs.label}px ${t.ui}`, color: t.dim }}>{tr.estCost}</div>
                <div style={{ font: `700 22px/1 ${t.mono}`, color: t.accent, marginTop: 5 }}>{tr.currencySymbol}{(M.cost * tr.exchangeRate).toFixed(2)}</div>
                <div style={{ font: `500 ${t.fs.small}px ${t.mono}`, color: t.faint, marginTop: 5, display: "flex", alignItems: "center", gap: 6, justifyContent: "flex-end" }}>
                  {tr.balance} {balance ? fmtMoney(balance.totalBalance, balSym) : tr.costDash}
                  <Switch on={trayMode === "balance"} theme={t} onClick={onToggleTrayMode} title={tr.trayModeHint} />
                </div>
              </div>
            </div>
            {/* cached vs rest (uncached input + output) — 2-colour pill. Dark segment
                is the cache share, matching the "% cached" label below. */}
            <div style={{ display: "flex", height: 8, borderRadius: 5, overflow: "hidden", marginBottom: 6, background: t.gridLine }}>
              {M.totalTokens > 0 && <>
                <div style={{ width: `${cachePct}%`, background: t.accent, transition: "width .3s ease" }} />
                <div style={{ width: `${restPct}%`, background: t.accentSoft, transition: "width .3s ease" }} />
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
            {/* period paging: ‹ › for week/month (0 = current) */}
            {period !== "Day" && (
              <div style={{ display: "flex", alignItems: "center", justifyContent: "flex-end", gap: 8, marginBottom: 8 }}>
                <button onClick={() => goPeriod(-1)} title="previous" aria-label="previous period" style={{
                  width: 24, height: 24, borderRadius: t.r.sm, cursor: "pointer", padding: 0,
                  display: "inline-flex", alignItems: "center", justifyContent: "center",
                  font: `600 12px ${t.mono}`, color: t.dim,
                  background: t.segBg, border: `1px solid ${t.segBorder}`,
                }} className="om-iconbtn">‹</button>
                <span style={{ font: `600 12px ${t.mono}`, color: t.dim }}>{periodTitle}</span>
                <button onClick={() => goPeriod(1)} title="next" aria-label="next period" style={{
                  width: 24, height: 24, borderRadius: t.r.sm, cursor: "pointer", padding: 0,
                  display: "inline-flex", alignItems: "center", justifyContent: "center",
                  font: `600 12px ${t.mono}`, color: t.dim,
                  background: t.segBg, border: `1px solid ${t.segBorder}`,
                }} className="om-iconbtn">›</button>
              </div>
            )}
            {/* bar chart */}
            <div key={period} className="om-fade-in">
              <BarChart data={P.series} theme={t} height={96} td={tr} />
            </div>
            <SectionRule t={t} m="16px 0 12px" />
            {/* models / projects — tabbed */}
            <div style={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", marginBottom: 4 }}>
              <Label t={t}>{costTab === "model" ? tr.tokensByModel : costTab === "project" ? tr.byProject : tr.tokensByAgent}</Label>
              <Segmented value={costTab} items={[tr.model, tr.project, tr.byAgent]} itemValues={["model", "project", "agent"]} theme={t} onSelect={(v) => setCostTab(v as "model" | "project" | "agent")} />
            </div>
            {/* capped token rows (~6 rows): the list itself scrolls when there
                are more models/projects/agents, keeping the panel compact */}
            <div className="om-nobar" style={{ maxHeight: 180, overflowY: "auto" }}>
              {costTab === "model" ? (
                <>
                  {tokenModels.length === 0 && <div style={{ font: `500 11.5px ${t.mono}`, color: t.faint, padding: "4px 0" }}>{tr.noUsageInThisPeriod}</div>}
                  {tokenModels.map((m, i) => <ModelRow key={i} m={m} max={maxM} theme={t} share={tokenShares[i]} />)}
                </>
              ) : costTab === "project" ? (
                <>
                  {projectTokens.length === 0 && <div style={{ font: `500 11.5px ${t.mono}`, color: t.faint, padding: "4px 0" }}>{tr.noUsageInThisPeriod}</div>}
                  {projectTokens.map((p, i) => <ProjectRow key={i} p={p} max={maxP} theme={t} share={projectShares[i]} />)}
                </>
              ) : (
                <>
                  {agentStats.length === 0 && <div style={{ font: `500 11.5px ${t.mono}`, color: t.faint, padding: "4px 0" }}>{tr.noUsageInThisPeriod}</div>}
                  {agentStats.map((a, i) => <AgentRow key={i} a={a} max={maxA} theme={t} share={agentShares[i]} color={agentColorOf(a)} />)}
                </>
              )}
              {costTab === "model" && unpricedModels.length > 0 && (
                <div style={{ marginTop: 10, font: `500 10.5px/1.5 ${t.mono}`, color: t.faint }}>
                  {tr.modelsWithoutPricing(unpricedModels.length)}{" "}
                  <span style={{ color: t.dim }}>{unpricedModels.map((m) => m.name).join(", ")}</span>
                </div>
              )}
            </div>
            {/* cost donut — same costTab toggle as the token rows above, so the
                two "by model/project/agent" breakdowns stay together */}
            <SectionRule t={t} m="10px 0 12px" />
            <div style={{ marginBottom: 6 }}><Label t={t}>{costTab === "model" ? tr.costByModel : costTab === "project" ? tr.costByProject : tr.costByAgent}</Label></div>
            {costTab === "model" ? (
              costModels.length > 0
                ? <div key={`donut:${period}`} className="om-fade-in">
                    <CostDonut models={costModels} theme={t} size={104} thickness={16}
                      currencySymbol={tr.currencySymbol} exchangeRate={tr.exchangeRate} />
                  </div>
                : <div style={{ font: `500 11.5px ${t.mono}`, color: t.faint }}>{tr.costDash}</div>
            ) : costTab === "project" ? (
              projectCostItems.length > 0
                ? <div key={`donut:${period}`} className="om-fade-in">
                    <CostDonut models={projectCostItems} theme={t} size={104} thickness={16}
                      currencySymbol={tr.currencySymbol} exchangeRate={tr.exchangeRate} />
                  </div>
                : <div style={{ font: `500 11.5px ${t.mono}`, color: t.faint }}>{tr.costDash}</div>
            ) : (
              agentCostItems.length > 0
                ? <div key={`donut:${period}`} className="om-fade-in">
                    <CostDonut models={agentCostItems} theme={t} size={104} thickness={16}
                      currencySymbol={tr.currencySymbol} exchangeRate={tr.exchangeRate} preserveColors />
                  </div>
                : <div style={{ font: `500 11.5px ${t.mono}`, color: t.faint }}>{tr.costDash}</div>
            )}
          </div>
          {/* ── right column ── */}
          <div style={{ minWidth: 0 }}>
            {/* KPI cards — 2×2 elevated grid */}
            <div key={period} className="om-fade-in" style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 10 }}>
              <KpiCard label={tr.requests} value={fmtInt(M.requests)} sub={`${M.sessions} ${tr.sessions}`} theme={t}>
                <Sparkline values={P.reqTrend.length ? P.reqTrend : [0, 0]} theme={t} width={54} height={22} accent={t.accent} />
              </KpiCard>
              <KpiCard label={tr.costTrend} value={`${tr.currencySymbol}${(M.cost * tr.exchangeRate).toFixed(2)}`} sub={trendSub} theme={t} accent={t.accent}>
                <Sparkline values={P.costTrend.length ? P.costTrend : [0, 0]} theme={t} width={54} height={22} accent={t.accent} />
              </KpiCard>
              <KpiCard label={tr.cacheHit} value={`${Math.round(cachePct)}%`} sub={`${tr.cached} · ${fmtTokens(M.cacheTokens)}`} theme={t} accent={t.accentSoft} />
              <KpiCard label={tr.avgPerReq} value={`${tr.currencySymbol}${avgPerReqStr}`} sub={tr.estCost} theme={t} />
            </div>
            {/* MCP — shown whenever the user has installed MCP servers */}
            {M.servers > 0 && (
              <>
                <SectionRule t={t} m="12px 0 10px" />
                <div style={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", marginBottom: 7 }}>
                  <Label t={t}>{tr.mcpCalls}</Label>
                  <span style={{ font: `500 ${t.fs.small}px ${t.mono}`, color: t.faint, whiteSpace: "nowrap" }}><span style={{ color: t.text, fontWeight: 600 }}>{fmtInt(M.mcpCalls)}</span> · {M.servers} {tr.servers}</span>
                </div>
                {P.mcp.length > 0
                  ? <BarList key={period} items={P.mcp} theme={t} accent={t.accent} td={tr} />
                  : <div style={{ font: `500 ${t.fs.small}px ${t.mono}`, color: t.faint, padding: "2px 0" }}>{tr.noMcpCalls}</div>}
              </>
            )}
            {/* Skill — shown whenever the user has installed skills */}
            {M.skills > 0 && (
              <>
                <SectionRule t={t} m="12px 0 10px" />
                <div style={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", marginBottom: 7 }}>
                  <Label t={t}>{tr.skillCalls}</Label>
                  <span style={{ font: `500 ${t.fs.small}px ${t.mono}`, color: t.faint, whiteSpace: "nowrap" }}><span style={{ color: t.text, fontWeight: 600 }}>{fmtInt(M.skillCalls)}</span> · {M.skills} {tr.skills}</span>
                </div>
                {P.skills.length > 0
                  ? <BarList key={period} items={P.skills} theme={t} accent={t.accent} td={tr} />
                  : <div style={{ font: `500 ${t.fs.small}px ${t.mono}`, color: t.faint, padding: "2px 0" }}>{tr.noSkillCalls}</div>}
              </>
            )}
            {/* heatmap — fills the right column's full width */}
            <SectionRule t={t} m="12px 0 10px" />
            <div style={{ marginBottom: 7 }}><Label t={t}>{tr.dailyActivity}</Label></div>
            <Heatmap days={dash.heatmap} theme={t} accent={t.accent} td={tr} />
          </div>
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
    <div style={{ display: "flex", alignItems: "center", gap: 9, padding: "6px 0" }}>
      <span style={{ width: 8, height: 8, borderRadius: 3, background: color, flex: "0 0 auto" }} />
      <div style={{ minWidth: 0, flex: "0 0 118px" }}>
        <div style={{ font: `500 ${theme.fs.body}px ${theme.ui}`, color: theme.text, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{a.agent}</div>
      </div>
      <div style={{ flex: 1, height: 6, borderRadius: 3, background: theme.gridLine, overflow: "hidden" }}>
        <div style={{ width: `${(a.tokens / max) * 100}%`, height: "100%", background: color, borderRadius: 3 }} />
      </div>
      <span style={{ font: `500 ${theme.fs.small}px ${theme.mono}`, color: theme.dim, flex: "0 0 auto", width: 44, textAlign: "right" }}>{fmtTokens(a.tokens)}</span>
      <span style={{ font: `600 ${theme.fs.small}px ${theme.mono}`, color: theme.text, flex: "0 0 auto", width: 42, textAlign: "right" }}>{pctStr}%</span>
    </div>
  );
}

// Agent colors by data source: green shades for OpenCode, orange for Codex,
// darker → lighter by rank within the source (keeps per-source distinction
// while still telling individual agents apart).
const OPENCODE_AGENT_COLORS = ["#047857", "#059669", "#10b981", "#34d399", "#6ee7b7", "#a7f3d0"];
const CODEX_AGENT_COLORS = ["#c2410c", "#ea580c", "#f97316", "#fb923c", "#fdba74", "#fed7aa"];
function agentColorBySource(agent: string, rankInSource: number): string {
  const pal = agent.startsWith("OpenCode-") ? OPENCODE_AGENT_COLORS : CODEX_AGENT_COLORS;
  return pal[rankInSource % pal.length];
}
/// Precompute one color per agent (rows and donut must share the same mapping).
function agentColorsFor(list: AgentStat[]): Map<string, string> {
  const out = new Map<string, string>();
  const rank = new Map<string, number>();
  for (const a of list) {
    const key = a.agent.startsWith("OpenCode-") ? "opencode" : "codex";
    const i = rank.get(key) ?? 0;
    rank.set(key, i + 1);
    out.set(a.agent, agentColorBySource(a.agent, i));
  }
  return out;
}

function AgentsTab({ dash, theme, tr }: { dash: Dashboard; theme: Theme; tr: Dict }) {
  const agents = dash.agents || [];
  const max = Math.max(...agents.map(a => a.tokens), 1e-9);
  const shares = sharePcts(agents.map(a => a.tokens));
  // Color by data source (green = OpenCode, orange = Codex), darker → lighter
  // by rank within the source; rows and donut share the same mapping.
  const colors = agentColorsFor(agents);
  const colorOf = (a: AgentStat) => colors.get(a.agent) ?? "#79817b";
  // Build donut items from agents with cost > 0 (keeping row colors).
  const costAgents: ModelStat[] = [];
  agents.forEach((a) => {
    if (a.cost > 0) {
      costAgents.push({
        name: a.agent, vendor: "", tokens: a.tokens, cost: a.cost,
        color: colorOf(a), priced: true, costSource: "pricing",
      });
    }
  });
  return (
    <div style={{ display: "grid", gridTemplateColumns: "minmax(0, 1.35fr) minmax(0, 1fr)", gap: "0 18px", alignItems: "start" }}>
      <div style={{ minWidth: 0, borderRight: `1px solid ${theme.gridLine}`, paddingRight: 18 }}>
        <div style={{ marginBottom: 9 }}><Label t={theme}>{tr.tokensByAgent}</Label></div>
        <div style={{ display: "flex", gap: 14, marginBottom: 8, font: `500 ${theme.fs.small}px ${theme.mono}`, color: theme.dim }}>
          <span><span style={{ color: "#10b981" }}>●</span> {tr.sourceOpenCode}</span>
          <span><span style={{ color: "#f97316" }}>●</span> {tr.sourceCodex}</span>
        </div>
        {agents.length === 0 ? (
          <div style={{ font: `500 11.5px ${theme.mono}`, color: theme.faint, padding: "4px 0" }}>{tr.noUsageInThisPeriod}</div>
        ) : (
          <div className="om-nobar" style={{ maxHeight: 232, overflowY: "auto" }}>
            {agents.map((a, i) => <AgentRow key={i} a={a} max={max} theme={theme} share={shares[i]} color={colorOf(a)} />)}
          </div>
        )}
      </div>
      <div style={{ minWidth: 0 }}>
        {costAgents.length > 0 && (
          <>
            <div style={{ marginBottom: 8 }}><Label t={theme}>{tr.costByAgent}</Label></div>
            <CostDonut models={costAgents} theme={theme} size={100} thickness={15}
              currencySymbol={tr.currencySymbol} exchangeRate={tr.exchangeRate}
              preserveColors />
          </>
        )}
      </div>
    </div>
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
      display: "flex", alignItems: "center", gap: 8, padding: "8px 0",
      borderBottom: `1px solid ${theme.gridLine}`,
    }}>
      <div style={{ minWidth: 0, flex: 1 }}>
        <div style={{ font: `500 ${theme.fs.body}px ${theme.ui}`, color: theme.text, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{title}</div>
        <div style={{ display: "flex", gap: 6, marginTop: 2, flexWrap: "wrap" }}>
          {s.projectName && (
            <span style={{ font: `500 10px ${theme.mono}`, color: theme.dim, background: theme.gridLine, padding: "1px 6px", borderRadius: 5 }}>{s.projectName}</span>
          )}
          {s.agent && (
            <span style={{ font: `500 10px ${theme.mono}`, color: theme.accent, background: `${theme.accent}22`, padding: "1px 6px", borderRadius: 5 }}>{s.agent}</span>
          )}
          <span style={{ font: `500 10px ${theme.mono}`, color: theme.faint }}>{fmtTimeAgo(s.timeCreated)}</span>
          <span style={{ font: `500 10px ${theme.mono}`, color: theme.faint }}>{fmtDuration(s.durationSecs)}</span>
        </div>
      </div>
      <div style={{ textAlign: "right", flex: "0 0 auto" }}>
        <div style={{ font: `600 ${theme.fs.small}px ${theme.mono}`, color: theme.text }}>{fmtTokens(s.tokens)}</div>
        {s.cost > 0 && (
          <div style={{ font: `500 10px ${theme.mono}`, color: theme.accent }}>{tr.currencySymbol}{s.cost.toFixed(2)}</div>
        )}
      </div>
    </div>
  );
}

function SessionsTab({ dash, theme, tr }: { dash: Dashboard; theme: Theme; tr: Dict }) {
  const sessions = dash.recentSessions || [];
  const [q, setQ] = useState("");
  const [sort, setSort] = useState<"time" | "tokens">("time");
  const query = q.trim().toLowerCase();
  const filtered = sessions.filter((s) =>
    !query || `${s.sessionTitle} ${s.projectName} ${s.agent}`.toLowerCase().includes(query)
  );
  const shown = [...filtered].sort((a, b) =>
    sort === "tokens" ? b.tokens - a.tokens : b.timeCreated.localeCompare(a.timeCreated)
  );
  return (
    <>
      <div style={{ marginBottom: 9 }}><Label t={theme}>{tr.recentSessions}</Label></div>
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 10 }}>
        <div style={{ position: "relative", flex: 1, minWidth: 0 }}>
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke={theme.dim} strokeWidth="2" strokeLinecap="round"
            style={{ position: "absolute", left: 9, top: "50%", transform: "translateY(-50%)", pointerEvents: "none" }}>
            <circle cx="11" cy="11" r="7" />
            <path d="M21 21l-4.3-4.3" />
          </svg>
          <input className="om-search" value={q} onChange={(e) => setQ(e.target.value)}
            placeholder={tr.searchSessions} aria-label={tr.searchSessions}
            style={{
              width: "100%", background: theme.surface, border: `1px solid ${theme.border}`, borderRadius: theme.r.sm,
              padding: "6px 26px 6px 26px", font: `500 11.5px ${theme.ui}`, color: theme.text, outline: "none",
            }} />
          {q && (
            <button onClick={() => setQ("")} title={tr.clearSearch} aria-label={tr.clearSearch}
              style={{
                position: "absolute", right: 5, top: "50%", transform: "translateY(-50%)",
                width: 18, height: 18, borderRadius: 5, border: "none", background: "transparent", cursor: "pointer",
                color: theme.dim, display: "inline-flex", alignItems: "center", justifyContent: "center",
              }}>
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round">
                <path d="M18 6L6 18M6 6l12 12" />
              </svg>
            </button>
          )}
        </div>
        <Segmented value={sort} items={[tr.sortRecent, tr.sortTokens]} itemValues={["time", "tokens"]} theme={theme}
          onSelect={(v) => setSort(v as "time" | "tokens")} />
      </div>
      {shown.length === 0 ? (
        <div style={{ font: `500 11.5px ${theme.mono}`, color: theme.faint, padding: "4px 0" }}>{tr.noSessions}</div>
      ) : (
        // Two columns of rows use the extra width; the list is height-capped so
        // a long history scrolls inside the list, not the whole panel.
        <div className="om-nobar" style={{ maxHeight: 440, overflowY: "auto", display: "grid", gridTemplateColumns: "1fr 1fr", columnGap: 14 }}>
          {shown.map((s, i) => <SessionRow key={i} s={s} theme={theme} tr={tr} />)}
        </div>
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
        ? <div style={{ padding: 20, font: `500 12px ${t.mono}`, color: t.danger }}>{tr.failedToLoad} {err}</div>
        : !dash
        ? <div style={{ height: "100vh", padding: 10, boxSizing: "border-box", background: "transparent" }}>
            <div style={{ height: "100%", borderRadius: t.r.xl, background: panelBackground(dark, t), border: `1px solid ${t.border}`, boxShadow: t.shadow,
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
