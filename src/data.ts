import { invoke } from "@tauri-apps/api/core";

export interface SeriesPoint { label: string; full: string; input: number; cache: number; output: number; reasoning: number }
export interface ProjectStat { projectId: string; projectName: string; worktree: string; tokens: number; cost: number; sessions: number }
export interface ModelStat { name: string; vendor: string; tokens: number; cost: number; color: string; priced: boolean; costSource: string }
export interface NamedCount { name: string; count: number }
export interface AgentStat { agent: string; tokens: number; cost: number; requests: number; sessions: number }
export interface BalanceInfo { currency: string; totalBalance: number; grantedBalance: number; toppedUpBalance: number }
export interface CodeMetrics { additions: number; deletions: number; files: number; diffs: number }
export interface SessionInfo { id: string; sessionTitle: string; agent: string; projectName: string; tokens: number; cost: number; durationSecs: number; timeCreated: string }
export interface Metrics {
  totalTokens: number; inputTokens: number; cacheTokens: number; outputTokens: number; cost: number;
  mcpCalls: number; skillCalls: number; requests: number; sessions: number;
  deltaTokens: number; deltaCost: number; servers: number; skills: number;
  reasoningTokens: number;
}
export interface PeriodReport {
  metrics: Metrics; series: SeriesPoint[]; models: ModelStat[]; agents: AgentStat[];
  mcp: NamedCount[]; skills: NamedCount[]; reqTrend: number[]; costTrend: number[];
  cacheTrend: number[];
  projects: ProjectStat[];
}
export interface HeatDay { date: string; tokens: number; level: number }
export interface Dashboard {
  day: PeriodReport; week: PeriodReport; month: PeriodReport;
  heatmap: HeatDay[]; todayTokens: number; generatedAt: string;
  agents: AgentStat[];
  codeMetrics: CodeMetrics;
  recentSessions: SessionInfo[];
}

export async function fetchDashboard(): Promise<Dashboard> {
  // Inside the Tauri runtime → call the Rust backend.
  const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  if (inTauri) return invoke<Dashboard>("get_dashboard");
  // Browser dev/preview fallback → static snapshot of real data.
  const res = await fetch("/dev-dashboard.json");
  if (!res.ok) throw new Error("not running in Tauri and no dev snapshot found");
  return res.json();
}

// ── formatting helpers ──────────────────────────────────────────
export const fmtTokens = (m: number) => {
  if (m >= 1) return m.toFixed(2) + "M";
  const k = m * 1000;
  // one decimal for sub-1K totals (e.g. "0.4K"), but only when it rounds to a
  // non-zero label — avoid a misleadingly precise "0.0K" for tiny values.
  if (k >= 0.05 && k < 1) return k.toFixed(1) + "K";
  return Math.round(k) + "K";
};
export const fmtInt = (n: number) => n.toLocaleString("en-US");
export const pct = (part: number, whole: number) => (whole > 0 ? Math.round((part / whole) * 100) : 0);
export function fmtMoney(v: number, sym = "$", rate = 1) {
  v *= rate;
  if (v >= 100000) return sym + Math.round(v / 1000) + "K";
  if (v >= 10000) return sym + (v / 1000).toFixed(1) + "K";
  return sym + v.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

export function linePath(values: number[], w: number, h: number, pad = 2) {
  const n = values.length;
  // Self-protect against degenerate inputs: callers pass a fixed-length series
  // today, but a 0-point array threw (pts[0]) and a 1-point array gave NaN (÷0).
  if (n === 0) return { d: "", px: (_i: number) => pad, py: (_v: number) => h / 2, pts: [] as [number, number][] };
  const max = Math.max(...values), min = Math.min(...values);
  const range = max - min || 1;
  const px = (i: number) => (n === 1 ? w / 2 : pad + (i / (n - 1)) * (w - pad * 2));
  const py = (v: number) => pad + (1 - (v - min) / range) * (h - pad * 2);
  const pts = values.map((v, i) => [px(i), py(v)] as [number, number]);
  let d = `M ${pts[0][0].toFixed(1)} ${pts[0][1].toFixed(1)}`;
  for (let i = 0; i < pts.length - 1; i++) {
    const p0 = pts[i - 1] || pts[i], p1 = pts[i], p2 = pts[i + 1], p3 = pts[i + 2] || p2;
    const c1x = p1[0] + (p2[0] - p0[0]) / 6, c1y = p1[1] + (p2[1] - p0[1]) / 6;
    const c2x = p2[0] - (p3[0] - p1[0]) / 6, c2y = p2[1] - (p3[1] - p1[1]) / 6;
    d += ` C ${c1x.toFixed(1)} ${c1y.toFixed(1)}, ${c2x.toFixed(1)} ${c2y.toFixed(1)}, ${p2[0].toFixed(1)} ${p2[1].toFixed(1)}`;
  }
  return { d, px, py, pts };
}

// ── theme ────────────────────────────────────────────────────────
// Full design-token set: colors, type scale, spacing, radii, elevation.
// Components read from `Theme` (t.*) instead of hardcoding values so a
// modernization pass stays centralized here.
export interface Theme {
  ui: string; mono: string; display: string;
  accent: string; accentSoft: string; cacheCol: string; reasoningCol: string;
  success: string; danger: string;
  text: string; dim: string; faint: string;
  gridLine: string; card: string;
  // card elevation: 1px border + soft shadow (+ highlight edge in dark)
  border: string;
  hi: string;           // inset 1px specular highlight (Liquid Glass rim light)
  surface: string;      // elevated card fill (KPI cards)
  surfaceAlt: string;   // subtle fill (chips, hover wells)
  shadow: string;       // panel / card drop shadow
  segBg: string; segBorder: string; segOnBg: string; segOnText: string; segOffText: string; segOnShadow: string;
  tip: string;
  // type scale (px)
  fs: { label: number; small: number; body: number; value: number; hero: number };
  // spacing scale (px)
  sp: { xs: number; sm: number; md: number; lg: number; xl: number };
  // radius scale (px)
  r: { sm: number; md: number; lg: number; xl: number; pill: number };
}
export const TH: Record<"dark" | "light", Theme> = {
  dark: {
    // macOS 27 Liquid Glass direction: system type, system palette, glass surfaces.
    ui: "-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Segoe UI', system-ui, sans-serif",
    mono: "ui-monospace, 'SF Mono', 'Cascadia Mono', Menlo, Consolas, monospace",
    display: "-apple-system, BlinkMacSystemFont, 'SF Pro Display', system-ui, sans-serif",
    accent: "#0a84ff", accentSoft: "#64d2ff", cacheCol: "#5a6660", reasoningCol: "#bf5af2",
    success: "#30d158", danger: "#ff453a",
    text: "rgba(255,255,255,0.97)", dim: "rgba(255,255,255,0.66)", faint: "rgba(255,255,255,0.42)",
    gridLine: "rgba(255,255,255,0.11)", card: "#1f2226",
    border: "rgba(255,255,255,0.18)",
    hi: "rgba(255,255,255,0.28)",
    surface: "linear-gradient(180deg, rgba(255,255,255,0.13), rgba(255,255,255,0.05))",
    surfaceAlt: "rgba(255,255,255,0.08)",
    shadow: "0 22px 52px rgba(0,0,0,0.55), 0 4px 12px rgba(0,0,0,0.28)",
    segBg: "rgba(255,255,255,0.10)", segBorder: "rgba(255,255,255,0.16)",
    segOnBg: "linear-gradient(180deg, rgba(255,255,255,0.26), rgba(255,255,255,0.12))",
    segOnText: "#fff", segOffText: "rgba(255,255,255,0.62)",
    segOnShadow: "inset 0 1px 0 rgba(255,255,255,0.38), 0 2px 6px rgba(0,0,0,0.28)",
    tip: "#34383d",
    fs: { label: 11, small: 10.5, body: 12, value: 20, hero: 40 },
    sp: { xs: 4, sm: 8, md: 12, lg: 16, xl: 24 },
    r: { sm: 8, md: 12, lg: 16, xl: 24, pill: 999 },
  },
  light: {
    ui: "-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Segoe UI', system-ui, sans-serif",
    mono: "ui-monospace, 'SF Mono', 'Cascadia Mono', Menlo, Consolas, monospace",
    display: "-apple-system, BlinkMacSystemFont, 'SF Pro Display', system-ui, sans-serif",
    accent: "#007aff", accentSoft: "#6cc4ff", cacheCol: "#aeb8b2", reasoningCol: "#af52de",
    success: "#34c759", danger: "#ff3b30",
    text: "rgba(20,22,26,0.96)", dim: "rgba(20,22,26,0.58)", faint: "rgba(20,22,26,0.38)",
    gridLine: "rgba(0,0,0,0.09)", card: "#ffffff",
    border: "rgba(255,255,255,0.85)",
    hi: "rgba(255,255,255,0.95)",
    surface: "linear-gradient(180deg, rgba(255,255,255,0.72), rgba(255,255,255,0.45))",
    surfaceAlt: "rgba(0,0,0,0.06)",
    shadow: "0 18px 44px rgba(16,24,40,0.18), 0 3px 10px rgba(16,24,40,0.08)",
    segBg: "rgba(255,255,255,0.55)", segBorder: "rgba(0,0,0,0.10)",
    segOnBg: "linear-gradient(180deg, rgba(255,255,255,0.95), rgba(255,255,255,0.70))",
    segOnText: "#111", segOffText: "rgba(0,0,0,0.55)",
    segOnShadow: "inset 0 1px 0 rgba(255,255,255,0.9), 0 1px 3px rgba(0,0,0,0.16)",
    tip: "#2b2f36",
    fs: { label: 11, small: 10.5, body: 12, value: 20, hero: 40 },
    sp: { xs: 4, sm: 8, md: 12, lg: 16, xl: 24 },
    r: { sm: 8, md: 12, lg: 16, xl: 24, pill: 999 },
  },
};

export function fmtHeatDate(iso: string) {
  const d = new Date(iso + "T00:00:00");
  return d.toLocaleDateString("en-US", { year: "numeric", month: "short", day: "numeric" });
}
