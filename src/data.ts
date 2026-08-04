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
    ui: "'IBM Plex Sans', system-ui, sans-serif",
    mono: "'IBM Plex Mono', ui-monospace, monospace",
    display: "'Space Grotesk', system-ui, sans-serif",
    accent: "#3b82f6", accentSoft: "#60a5fa", cacheCol: "#5a6660", reasoningCol: "#a78bfa",
    success: "#27b06e", danger: "#e0795f",
    text: "rgba(255,255,255,0.94)", dim: "rgba(255,255,255,0.52)", faint: "rgba(255,255,255,0.32)",
    gridLine: "rgba(255,255,255,0.06)", card: "#1f2226",
    border: "rgba(255,255,255,0.10)",
    surface: "#24282d", surfaceAlt: "rgba(255,255,255,0.045)",
    shadow: "0 10px 30px rgba(0,0,0,0.40), 0 2px 8px rgba(0,0,0,0.25)",
    segBg: "rgba(255,255,255,0.06)", segBorder: "rgba(255,255,255,0.09)",
    segOnBg: "rgba(255,255,255,0.15)", segOnText: "#fff", segOffText: "rgba(255,255,255,0.55)",
    segOnShadow: "0 1px 2px rgba(0,0,0,0.35)", tip: "#34383d",
    fs: { label: 11, small: 10.5, body: 12, value: 20, hero: 40 },
    sp: { xs: 4, sm: 8, md: 12, lg: 16, xl: 24 },
    r: { sm: 8, md: 12, lg: 16, xl: 20, pill: 999 },
  },
  light: {
    ui: "'IBM Plex Sans', system-ui, sans-serif",
    mono: "'IBM Plex Mono', ui-monospace, monospace",
    display: "'Space Grotesk', system-ui, sans-serif",
    accent: "#2563eb", accentSoft: "#93c5fd", cacheCol: "#aeb8b2", reasoningCol: "#7c3aed",
    success: "#16a34a", danger: "#dc5a4e",
    text: "rgba(17,22,19,0.94)", dim: "rgba(17,22,19,0.5)", faint: "rgba(17,22,19,0.32)",
    gridLine: "rgba(0,0,0,0.06)", card: "#ffffff",
    border: "rgba(0,0,0,0.09)",
    surface: "#ffffff", surfaceAlt: "rgba(0,0,0,0.03)",
    shadow: "0 1px 2px rgba(16,24,40,0.05), 0 10px 28px rgba(16,24,40,0.10)",
    segBg: "rgba(0,0,0,0.05)", segBorder: "rgba(0,0,0,0.07)",
    segOnBg: "#ffffff", segOnText: "#111", segOffText: "rgba(0,0,0,0.5)",
    segOnShadow: "0 1px 2px rgba(0,0,0,0.12)", tip: "#1d2420",
    fs: { label: 11, small: 10.5, body: 12, value: 20, hero: 40 },
    sp: { xs: 4, sm: 8, md: 12, lg: 16, xl: 24 },
    r: { sm: 8, md: 12, lg: 16, xl: 20, pill: 999 },
  },
};

export function fmtHeatDate(iso: string) {
  const d = new Date(iso + "T00:00:00");
  return d.toLocaleDateString("en-US", { year: "numeric", month: "short", day: "numeric" });
}
