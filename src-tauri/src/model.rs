// Shared data structures returned to the frontend.
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AgentStat {
    pub agent: String,
    pub tokens: f64,  // M tokens
    pub cost: f64,    // USD
    pub requests: u64,
    pub sessions: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodeMetrics {
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
    pub diffs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub id: String,
    #[serde(rename = "sessionTitle")]
    pub session_title: String,
    pub agent: String,
    #[serde(rename = "projectName")]
    pub project_name: String,
    pub tokens: f64,  // M tokens
    pub cost: f64,    // USD
    #[serde(rename = "durationSecs")]
    pub duration_secs: u64,
    #[serde(rename = "timeCreated")]
    pub time_created: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeriesPoint {
    pub label: String, // sparse axis label (many empty)
    pub full: String,  // complete label for the hover tooltip (hour / date)
    pub input: f64,    // M tokens (uncached new input)
    pub cache: f64,    // M tokens (cache creation + read)
    pub output: f64,   // M tokens
    pub reasoning: f64, // M tokens (reasoning)
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectStat {
    #[serde(rename = "projectId")]
    pub project_id: String,
    #[serde(rename = "projectName")]
    pub project_name: String,
    pub worktree: String,
    pub tokens: f64,  // M tokens
    pub cost: f64,    // USD
    pub sessions: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelStat {
    pub name: String,
    pub vendor: String,
    pub tokens: f64, // M tokens (input+output, weighted)
    pub cost: f64,   // USD estimate
    pub color: String,
    pub priced: bool, // false = no pricing data in LiteLLM (cost is unknown, not $0)
    #[serde(rename = "costSource")]
    pub cost_source: String, // "pricing", "opencode", or "none"
}

#[derive(Debug, Clone, Serialize)]
pub struct NamedCount {
    pub name: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Metrics {
    #[serde(rename = "totalTokens")]
    pub total_tokens: f64,
    #[serde(rename = "inputTokens")]
    pub input_tokens: f64,
    #[serde(rename = "cacheTokens")]
    pub cache_tokens: f64,
    #[serde(rename = "outputTokens")]
    pub output_tokens: f64,
    #[serde(rename = "reasoningTokens")]
    pub reasoning_tokens: f64,
    pub cost: f64,
    #[serde(rename = "mcpCalls")]
    pub mcp_calls: u64,
    #[serde(rename = "skillCalls")]
    pub skill_calls: u64,
    pub requests: u64,
    pub sessions: u64,
    #[serde(rename = "deltaTokens")]
    pub delta_tokens: f64,
    #[serde(rename = "deltaCost")]
    pub delta_cost: f64,
    pub servers: u64,
    pub skills: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PeriodReport {
    pub metrics: Metrics,
    pub series: Vec<SeriesPoint>,
    pub models: Vec<ModelStat>,
    pub agents: Vec<AgentStat>,
    pub mcp: Vec<NamedCount>,
    pub skills: Vec<NamedCount>,
    pub projects: Vec<ProjectStat>,
    #[serde(rename = "reqTrend")]
    pub req_trend: Vec<f64>,
    #[serde(rename = "costTrend")]
    pub cost_trend: Vec<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeatDay {
    pub date: String, // ISO yyyy-mm-dd
    pub tokens: f64,  // M tokens
    pub level: u8,    // 0..4
}

#[derive(Debug, Clone, Serialize)]
pub struct Dashboard {
    pub day: PeriodReport,
    pub week: PeriodReport,
    pub month: PeriodReport,
    pub heatmap: Vec<HeatDay>,
    #[serde(rename = "todayTokens")]
    pub today_tokens: f64, // M tokens, for the tray label
    #[serde(rename = "generatedAt")]
    pub generated_at: String,
    pub agents: Vec<AgentStat>,
    #[serde(rename = "codeMetrics")]
    pub code_metrics: CodeMetrics,
    #[serde(rename = "recentSessions")]
    pub recent_sessions: Vec<SessionInfo>,
}
