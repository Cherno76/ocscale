import { createContext, useContext } from "react";

export type Lang = "en" | "zh";

export type Dict = typeof EN;

const EN = {
  // Header
  appName: "OCScale",
  // Periods
  day: "Day",
  week: "Week",
  month: "Month",
  dayLocal: "Local",
  dayUtc: "UTC",
  // Hero
  totalTokens: "Total tokens",
  estCost: "Est. cost",
  balance: "Balance",
  setApiKey: "Set API key",
  deepseekKey: "DeepSeek API key",
  deepseekKeyPlaceholder: "sk-…",
  save: "Save",
  cancel: "Cancel",
  keySaved: "API key saved",
  keyInvalid: "Key looks invalid",
  balanceRetry: "Balance failed · Retry",
  // Multi-device sync
  syncTitle: "Multi-device sync",
  syncHint: "Push usage to the central server (multi-device merge)",
  syncCollapse: "Collapse",
  syncExpand: "Expand",
  syncServerUrl: "Server URL (https://…/api)",
  syncTokenPlaceholder: "Server token",
  syncTokenSet: "Token saved — type to replace",
  syncSave: "Save",
  syncNow: "Sync now",
  syncNever: "never",
  syncLastSync: "Last sync",
  syncDevice: "Device",
  syncPending: (n: number) => `${n} event${n === 1 ? "" : "s"} pending`,
  // Cache split
  cached: "Cached",
  new_: "New",
  pctCached: "% cache hit",
  reasoning: "Reasoning",
  // Model section
  tokensByModel: "Tokens by model",
  noUsageInThisPeriod: "No usage in this period",
  costByModel: "Cost by model",
  costDash: "—",
  model: "Model",
  project: "Project",
  modelsWithoutPricing: (n: number) =>
    `${n} model${n > 1 ? "s" : ""} without pricing data (cost not counted):`,
  // Stats
  requests: "Requests",
  sessions: "sessions",
  costTrend: "Cost trend",
  cacheHit: "Cache hit",
  avgPerReq: "Cost / request",
  today24h: "today 24h",
  thisWeek: "this week",
  thisMonth: "this month",
  // Projects
  byProject: "By project",
  costByProject: "Cost by project",
  sessionsCount: (n: number) => `${n} sessions`,
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
  // DeepSeek reports the balance in CNY; the English UI prices everything in
  // USD, so convert at the same fixed rate the ZH UI uses for cost (¥7.2/$1).
  cnyPerUsd: 7.2,
  // Footer
  refresh: "Refresh",
  moreActions: "More actions",
  launchAtLogin: "Launch at Login",
  quit: "Quit",
  trayModeHint: "Toggle menu-bar label (tokens / balance)",
  // Tooltips
  noTokens: "No tokens",
  tokensLabel: (n: string) => `${n} tokens`,
  noCalls: "No calls",
  // Tabs
  overview: "Overview",
  agents: "Agents",
  code: "Code",
  sessionsTab: "Sessions",
  // Agents tab
  tokensByAgent: "Tokens by agent",
  costByAgent: "Cost by agent",
  byAgent: "Agent",
  sourceOpenCode: "OpenCode",
  sourceCodex: "Codex",
  // Code tab
  codeActivity: "Code activity",
  linesAdded: "Added",
  linesDeleted: "Deleted",
  filesChanged: "Files",
  diffsCount: "Diffs",
  noCodeActivity: "No code activity recorded",
  // Sessions tab
  recentSessions: "Recent sessions",
  searchSessions: "Search sessions…",
  clearSearch: "Clear search",
  sortRecent: "Recent",
  sortTokens: "By tokens",
  noSessions: "No sessions in this period",
  durationLabel: "duration",
};

const ZH: Dict = {
  appName: "OCScale",
  day: "日",
  week: "周",
  month: "月",
  dayLocal: "本地日",
  dayUtc: "平台日",
  totalTokens: "总 Token 数",
  estCost: "预估费用",
  balance: "当前余额",
  setApiKey: "设置 API Key",
  deepseekKey: "DeepSeek API Key",
  deepseekKeyPlaceholder: "sk-…",
  save: "保存",
  cancel: "取消",
  keySaved: "API Key 已保存",
  keyInvalid: "Key 格式不正确",
  balanceRetry: "余额获取失败 · 重试",
  // Multi-device sync
  syncTitle: "多设备同步",
  syncHint: "把用量推送到中心服务器（多设备合并）",
  syncCollapse: "折叠",
  syncExpand: "展开",
  syncServerUrl: "服务器地址 (https://…/api)",
  syncTokenPlaceholder: "服务器 Token",
  syncTokenSet: "已保存 Token，输入以更换",
  syncSave: "保存",
  syncNow: "立即同步",
  syncNever: "从未",
  syncLastSync: "上次同步",
  syncDevice: "设备",
  syncPending: (n: number) => `${n} 条待推送`,
  cached: "缓存",
  new_: "新增",
  pctCached: "% 命中缓存",
  reasoning: "推理",
  tokensByModel: "按模型统计 Token",
  noUsageInThisPeriod: "该时段无使用记录",
  costByModel: "按模型统计费用",
  costDash: "—",
  model: "模型",
  project: "项目",
  modelsWithoutPricing: (n: number) =>
    `${n} 个模型缺少定价数据（费用未计入）：`,
  requests: "请求数",
  sessions: "会话",
  costTrend: "费用趋势",
  cacheHit: "缓存命中",
  avgPerReq: "单次请求成本",
  today24h: "今日 24h",
  thisWeek: "本周",
  thisMonth: "本月",
  // Projects
  byProject: "按项目统计TOKEN",
  costByProject: "按项目统计费用",
  sessionsCount: (n: number) => `${n} 个会话`,
  // MCP
  mcpCalls: "MCP 调用",
  servers: "服务",
  noMcpCalls: "该时段无 MCP 调用",
  skillCalls: "技能调用",
  skills: "技能",
  noSkillCalls: "该时段无技能调用",
  dailyActivity: "每日活动",
  less: "少",
  more: "多",
  currencySymbol: "¥",
  exchangeRate: 7.2,
  cnyPerUsd: 7.2,
  refresh: "刷新",
  moreActions: "更多操作",
  launchAtLogin: "开机启动",
  quit: "退出",
  trayModeHint: "切换菜单栏显示（Token / 余额）",
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
  tokensLabel: (n: string) => `${n} Tokens`,
  noCalls: "无调用",
  // Tabs
  overview: "概览",
  agents: "Agent",
  code: "代码",
  sessionsTab: "会话",
  // Agents tab
  tokensByAgent: "按 Agent 统计 Token",
  costByAgent: "按 Agent 统计费用",
  byAgent: "Agent",
  sourceOpenCode: "OpenCode",
  sourceCodex: "Codex",
  // Code tab
  codeActivity: "代码活动",
  linesAdded: "新增行",
  linesDeleted: "删除行",
  filesChanged: "文件数",
  diffsCount: "Diff 数",
  noCodeActivity: "无代码活动记录",
  // Sessions tab
  recentSessions: "最近会话",
  searchSessions: "搜索会话…",
  clearSearch: "清除搜索",
  sortRecent: "最近",
  sortTokens: "按 Token",
  noSessions: "该时段无会话",
  durationLabel: "时长",
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
