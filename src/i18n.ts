import { createContext, useContext } from "react";

export type Lang = "en" | "zh";

export type Dict = typeof EN;

const EN = {
  // Header
  appName: "Tokenscope",
  // Periods
  day: "Day",
  week: "Week",
  month: "Month",
  // Hero
  totalTokens: "Total tokens",
  estCost: "Est. cost",
  // Cache split
  cached: "Cached",
  new_: "New",
  pctCached: "% cached",
  // Model section
  tokensByModel: "Tokens by model",
  noUsageInThisPeriod: "No usage in this period",
  costByModel: "Cost by model",
  costDash: "—",
  modelsWithoutPricing: (n: number) =>
    `${n} model${n > 1 ? "s" : ""} without pricing data (cost not counted):`,
  // Stats
  requests: "Requests",
  sessions: "sessions",
  costTrend: "Cost trend",
  today24h: "today 24h",
  thisWeek: "this week",
  thisMonth: "this month",
  // MCP
  mcpCalls: "MCP calls",
  servers: "servers",
  noMcpCalls: "No MCP calls in this period",
  // Skills
  skillCalls: "Skill calls",
  skills: "skills",
  noSkillCalls: "No skill calls in this period",
  // Heatmap
  dailyActivity: "Daily activity",
  less: "Less",
  more: "More",
  // Footer
  estimateNote: "Est. cost via models.dev / LiteLLM · estimate",
  // Theme
  dark: "Dark",
  light: "Light",
  system: "System",
  // Screenshot
  screenshotTitle: "Save screenshot to Desktop",
  savedToDesktop: "Saved to Desktop",
  downloaded: "Downloaded",
  screenshotFailed: "Screenshot failed",
  // BarList
  nMore: (n: number) => `+${n} more`,
  showLess: "show less",
  // Loading / error
  loading: "Loading…",
  failedToLoad: "Failed to load:",
  currencySymbol: "$",
  exchangeRate: 1,
  // Footer
  refresh: "Refresh",
  launchAtLogin: "Launch at Login",
  quit: "Quit",
  // Tooltips
  noTokens: "No tokens",
  tokensLabel: (n: string) => `${n} tokens`,
  noCalls: "No calls",
};

const ZH: Dict = {
  appName: "Tokenscope",
  day: "日",
  week: "周",
  month: "月",
  totalTokens: "总 Token 数",
  estCost: "预估费用",
  cached: "缓存",
  new_: "新增",
  pctCached: "% 已缓存",
  tokensByModel: "按模型统计 Token",
  noUsageInThisPeriod: "该时段无使用记录",
  costByModel: "按模型统计费用",
  costDash: "—",
  modelsWithoutPricing: (n: number) =>
    `${n} 个模型缺少定价数据（费用未计入）：`,
  requests: "请求数",
  sessions: "会话",
  costTrend: "费用趋势",
  today24h: "今日 24h",
  thisWeek: "本周",
  thisMonth: "本月",
  mcpCalls: "MCP 调用",
  servers: "服务",
  noMcpCalls: "该时段无 MCP 调用",
  skillCalls: "技能调用",
  skills: "技能",
  noSkillCalls: "该时段无技能调用",
  dailyActivity: "每日活动",
  less: "少",
  more: "多",
  estimateNote: "费用估算来自 models.dev / LiteLLM · 仅供参考",
  currencySymbol: "¥",
  exchangeRate: 7.2,
  refresh: "刷新",
  launchAtLogin: "开机启动",
  quit: "退出",
  dark: "深色",
  light: "浅色",
  system: "系统",
  screenshotTitle: "保存截图到桌面",
  savedToDesktop: "已保存到桌面",
  downloaded: "已下载",
  screenshotFailed: "截图失败",
  nMore: (n: number) => `还有 ${n} 项`,
  showLess: "收起",
  loading: "加载中…",
  failedToLoad: "加载失败：",
  noTokens: "无 Token",
  tokensLabel: (n: string) => `${n} 个 Token`,
  noCalls: "无调用",
};

export const DICT: Record<Lang, Dict> = { en: EN, zh: ZH };

export interface I18nCtx {
  lang: Lang;
  t: Dict;
  toggleLang: () => void;
}

export const I18nContext = createContext<I18nCtx>({
  lang: "en",
  t: EN,
  toggleLang: () => {},
});

export function useT() {
  return useContext(I18nContext);
}
