// Query OpenCode's SQLite session database, classify tool calls (user-installed
// MCP / Skill only), and aggregate into Day / Week / Month reports + a daily
// heatmap. The RawEvent data source is swapped from JSONL → SQLite via store.rs;
// nothing else in this file needs to change.
use crate::config::UserConfig;
use crate::model::*;
use crate::pricing::Pricing;
use crate::store::{RawEvent, Store};
use chrono::{DateTime, Datelike, Duration, Local, Timelike, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

// Serializes dashboard builds so the background refresh thread and the command
// handler never touch the incremental cache files concurrently.
static BUILD_LOCK: Mutex<()> = Mutex::new(());

// One assistant API response, with config + pricing applied (derived per request
// from a RawEvent, since user config / prices / time windows can all change).
// One assistant API response, with config + pricing applied (derived per request
// from a RawEvent, since user config / prices / time windows can all change).
#[derive(Clone)]
struct Event {
    ts: DateTime<Local>,
    session: String,
    model: String,
    input: f64,  // raw tokens, uncached new input only
    cache: f64,  // raw tokens, cache creation + read
    output: f64, // raw tokens
    reasoning: f64, // reasoning tokens
    cost: f64,   // USD (differentiated by token type), 0 if unknown model
    priced: bool, // whether a price was found for this model
    cost_source: String, // "pricing", "opencode", or "none"
    mcp: Vec<String>,   // user-installed server names called in this msg
    skills: Vec<String>, // user-installed skill names called in this msg
    project_id: String,
    project_name: String,
    // ── session-level metadata ────────────────────────────────────
    agent: String,
    code_additions: u64,
    code_deletions: u64,
    code_files: u64,
    code_diffs: u64,
    session_duration_ms: i64,
    session_time_created_ms: i64,
    session_title: String,
}

// Top-5 models keep the blue/slate scheme; everything beyond is uniform gray.
const PALETTE: &[&str] = &["#1e40af", "#2563eb", "#3b82f6", "#60a5fa", "#4b5a52"];
const OVERFLOW_GRAY: &str = "#79817b";

/// Strip a trailing "-YYYYMMDD" date suffix so dated releases merge into
/// their base model (e.g. "claude-haiku-4-5-20251001" → "claude-haiku-4-5").
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn normalize_model(name: &str) -> String {
    if let Some(idx) = name.rfind('-') {
        let suffix = &name[idx + 1..];
        if suffix.len() == 8 && suffix.chars().all(|c| c.is_ascii_digit()) {
            return name[..idx].to_string();
        }
    }
    name.to_string()
}

fn vendor_of(model: &str) -> &'static str {
    let m = model.to_lowercase();
    if m.contains("claude") {
        "Anthropic"
    } else if m.contains("gpt") || m.contains("o1") || m.contains("o3") {
        "OpenAI"
    } else if m.contains("gemini") {
        "Google"
    } else if m.contains("llama") {
        "Local"
    } else if m.contains("glm") {
        "Zhipu"
    } else if m.contains("deepseek") {
        "DeepSeek"
    } else {
        "Other"
    }
}

pub fn build_dashboard() -> Dashboard {
    build_dashboard_with_mode(false)
}

/// Dashboard with the day-boundary mode applied: `utc_day` switches the Day
/// report and the tray's "today" to the UTC boundary (matches DeepSeek's
/// platform usage dashboard); week/month/heatmap stay on the local calendar.
pub fn build_dashboard_with_mode(utc_day: bool) -> Dashboard {
    let _guard = BUILD_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // 1. OpenCode: incremental ingest, prune to the heatmap window, persist only
    //    when something actually changed — so an idle tick doesn't rewrite the
    //    entire cache every 30s.
    let mut store = Store::load();
    let mut dirty = store.ingest();
    // Reports/heatmap span ~26 weeks (+ prev month); 210 days leaves margin.
    let cutoff = (Local::now() - Duration::days(210)).timestamp_millis();
    if store.prune_before(cutoff) {
        dirty = true;
    }
    if dirty {
        store.save();
    }

    // 2. Merge Codex transcripts (memoized by file mtime) for the combined view.
    let mut events = store.events;
    events.extend(crate::store_codex::cached_events().iter().cloned());
    events.retain(|e| e.ts_ms >= cutoff);
    events.sort_by_key(|e| e.ts_ms);

    build_dashboard_from_mode(&events, utc_day)
}

/// Shared aggregation core: turns any `RawEvent` stream (OpenCode SQLite, or the
/// Codex feasibility prototype in `store_codex`) into a Dashboard. Holds no lock
/// and performs no store IO — callers own serialization and data loading.
pub fn build_dashboard_from(events: &[RawEvent]) -> Dashboard {
    build_dashboard_from_mode(events, false)
}

/// Shared aggregation core: turns any `RawEvent` stream (OpenCode SQLite, or the
/// Codex feasibility prototype in `store_codex`) into a Dashboard. Holds no lock
/// and performs no store IO — callers own serialization and data loading.
pub fn build_dashboard_from_mode(events: &[RawEvent], utc_day: bool) -> Dashboard {
    // 2. Aggregate: apply current config + prices, slice by current time.
    let cfg = UserConfig::load();
    // Memoized price table (cheap clone); loaded/refreshed off-thread elsewhere
    // so neither parsing nor the network runs while we hold BUILD_LOCK.
    let pricing = Pricing::shared();
    let events: Vec<Event> = events
        .iter()
        .map(|r| compute_event(r, &cfg, &pricing))
        .collect();

    let now = Local::now();
    let today = now.date_naive();

    let mut day = report_day(&events, now, utc_day);
    let mut week = report_week(&events, now);
    let mut month = report_month(&events, now);
    let heatmap = build_heatmap(&events, today);

    // "servers"/"skills" = how many the user has *installed* (global, constant
    // across periods), not how many were called in the window.
    let installed_servers = cfg.mcp_servers.len() as u64;
    let installed_skills = cfg.skills.len() as u64;
    for r in [&mut day, &mut week, &mut month] {
        r.metrics.servers = installed_servers;
        r.metrics.skills = installed_skills;
    }

    // today's displayed tokens (M) for the tray
    let today_tokens: f64 = events
        .iter()
        .filter(|e| {
            if utc_day {
                e.ts.with_timezone(&Utc).date_naive() == now.with_timezone(&Utc).date_naive()
            } else {
                e.ts.date_naive() == today
            }
        })
        .map(|e| (e.input + e.cache + e.output + e.reasoning) / 1e6)
        .sum();

    // 4. Cross-cutting aggregations (not tied to day/week/month period).
    //    Agents, code metrics, and session list are computed from all events
    //    in memory (~210-day window) so all tabs show consistent data.
    let mut full = Agg::default();
    for e in &events {
        full.add(e);
    }
    let agents = full.agents();
    let code_metrics = full.code_metrics();
    let recent_sessions = full.recent_sessions(50);

    Dashboard {
        day,
        week,
        month,
        heatmap,
        today_tokens,
        generated_at: now.to_rfc3339(),
        agents,
        code_metrics,
        recent_sessions,
    }
}

/// Derive a computed Event from a stored RawEvent, applying the *current* user
/// config (MCP/Skill whitelist) and prices. Tries the pricing module first;
/// if the model isn't recognised (e.g. a custom DeepSeek variant), falls back
/// to the pre-calculated cost stored by OpenCode in the session table.
fn compute_event(r: &RawEvent, cfg: &UserConfig, pricing: &Pricing) -> Event {
    let ts = DateTime::from_timestamp_millis(r.ts_ms)
        .unwrap_or_default()
        .with_timezone(&Local);
    let model = normalize_model(&r.model);
    // price lookup uses the raw (possibly dated) id, then the normalized one,
    // then finally falls back to OpenCode's own cost calculation.
    // Track which source produced the cost value.
    let (cost_opt, cost_source) = if let Some(c) = pricing
        .cost(&r.model, r.in_tok, r.out_tok, r.cc, r.cr, r.reasoning)
    {
        (Some(c), "pricing")
    } else if let Some(c) = pricing.cost(&model, r.in_tok, r.out_tok, r.cc, r.cr, r.reasoning) {
        (Some(c), "pricing")
    } else if let Some(c) = r.stored_cost {
        (Some(c), "opencode")
    } else {
        (None, "none")
    };
    let mcp = if r.source == "codex" {
        // Codex feasibility prototype: `mcp__`-prefixed tool names are already
        // user-configured servers by definition, so skip the OpenCode whitelist.
        r.mcp.clone()
    } else {
        r.mcp
            .iter()
            .filter(|s| cfg.is_user_mcp(s))
            .cloned()
            .collect()
    };
    let skills = r
        .skills
        .iter()
        .filter(|s| cfg.is_user_skill(s))
        .map(|s| s.rsplit(':').next().unwrap_or(s).to_string())
        .collect();
    Event {
        ts,
        session: r.session.clone(),
        model,
        input: r.in_tok,
        cache: r.cc + r.cr,
        output: r.out_tok,
        reasoning: r.reasoning,
        cost: cost_opt.unwrap_or(0.0),
        priced: cost_opt.is_some(),
        cost_source: cost_source.to_string(),
        mcp,
        skills,
        project_id: r.project_id.clone(),
        project_name: r.project_name.clone(),
        agent: r.agent.clone(),
        code_additions: r.code_additions,
        code_deletions: r.code_deletions,
        code_files: r.code_files,
        code_diffs: r.code_diffs,
        session_duration_ms: r.session_duration_ms,
        session_time_created_ms: r.session_time_created_ms,
        session_title: r.session_title.clone(),
    }
}

// ── aggregation helpers ────────────────────────────────────────────
struct SessionAccum {
    id: String,
    title: String,
    agent: String,
    project_name: String,
    tokens: f64,
    cost: f64,
    dur_ms: i64,
    time_created_ms: i64,
    last_active_ms: i64,
}

#[derive(Default)]
struct Agg {
    input: f64,
    cache: f64,
    output: f64,
    reasoning: f64,
    cost: f64,
    requests: u64,
    sessions: HashSet<String>,
    mcp_calls: u64,
    skill_calls: u64,
    model_tok: HashMap<String, f64>,
    model_cost: HashMap<String, f64>,
    model_priced: HashMap<String, bool>,
    model_cost_source: HashMap<String, String>,
    mcp_counts: HashMap<String, u64>,
    skill_counts: HashMap<String, u64>,
    // ── agent-level aggregation ──────────────────────────────────
    agent_tokens: HashMap<String, f64>,
    agent_cost: HashMap<String, f64>,
    agent_requests: HashMap<String, u64>,
    agent_sessions: HashMap<String, HashSet<String>>,
    // ── code metrics (summed across unique sessions in period) ──
    code_additions: u64,
    code_deletions: u64,
    code_files: u64,
    code_diffs: u64,
    code_sessions: HashSet<String>, // track which sessions we've already counted
    // ── session info for listing (accumulated across all messages) ──
    session_map: HashMap<String, SessionAccum>,
}

impl Agg {
    fn add(&mut self, e: &Event) {
        self.input += e.input;
        self.cache += e.cache;
        self.output += e.output;
        self.reasoning += e.reasoning;
        self.cost += e.cost;
        // ── session: only count once per unique session ───────────
        let is_new_session = !e.session.is_empty() && self.sessions.insert(e.session.clone());
        // ── agent tracking (per-message, like model stats) ────────
        if !e.agent.is_empty() && !e.model.is_empty() {
            *self.agent_tokens.entry(e.agent.clone()).or_default() += e.input + e.cache + e.output + e.reasoning;
            *self.agent_cost.entry(e.agent.clone()).or_default() += e.cost;
            *self.agent_requests.entry(e.agent.clone()).or_default() += 1;
            self.agent_sessions.entry(e.agent.clone()).or_default().insert(e.session.clone());
        }
        // ── code metrics: sum once per unique session ─────────────
        if is_new_session && e.code_additions > 0 || e.code_files > 0 {
            if self.code_sessions.insert(e.session.clone()) {
                self.code_additions += e.code_additions;
                self.code_deletions += e.code_deletions;
                self.code_files += e.code_files;
                self.code_diffs += e.code_diffs;
            }
        }
        // ── session info: accumulate per-session totals ────────────
        if !e.session.is_empty() {
            let msg_total = e.input + e.cache + e.output + e.reasoning;
            let msg_time = e.ts.timestamp_millis();
            let sess = self.session_map.entry(e.session.clone()).or_insert_with(|| {
                SessionAccum {
                    id: e.session.clone(),
                    title: e.session_title.clone(),
                    agent: e.agent.clone(),
                    project_name: e.project_name.clone(),
                    tokens: 0.0,
                    cost: 0.0,
                    dur_ms: e.session_duration_ms,
                    time_created_ms: if e.session_time_created_ms > 0 {
                        e.session_time_created_ms
                    } else {
                        msg_time
                    },
                    last_active_ms: 0,
                }
            });
            sess.tokens += msg_total;
            sess.cost += e.cost;
            if msg_time > sess.last_active_ms {
                sess.last_active_ms = msg_time;
            }
        }
        // Slash-command skill events carry no model (empty) — they're not LLM
        // requests, so they must not inflate request counts or the model split.
        if !e.model.is_empty() {
            self.requests += 1; // each RawEvent is one assistant message
            // model totals keep all token types so shares sum to Total tokens
            *self.model_tok.entry(e.model.clone()).or_default() += e.input + e.cache + e.output + e.reasoning;
            *self.model_cost.entry(e.model.clone()).or_default() += e.cost;
            // a model is "priced" if any of its messages had a known price
            *self.model_priced.entry(e.model.clone()).or_default() |= e.priced;
            // track cost_source per model: "pricing" > "opencode" > "none"
            let cs = self.model_cost_source.entry(e.model.clone()).or_insert_with(|| "none".to_string());
            if e.cost_source == "pricing" {
                *cs = "pricing".to_string();
            } else if e.cost_source == "opencode" && cs.as_str() != "pricing" {
                *cs = "opencode".to_string();
            }
        }
        for s in &e.mcp {
            self.mcp_calls += 1;
            *self.mcp_counts.entry(s.clone()).or_default() += 1;
        }
        for s in &e.skills {
            self.skill_calls += 1;
            *self.skill_counts.entry(s.clone()).or_default() += 1;
        }
    }

    fn models(&self) -> Vec<ModelStat> {
        let mut v: Vec<(String, f64, f64)> = self
            .model_tok
            .iter()
            .map(|(k, t)| (k.clone(), *t, *self.model_cost.get(k).unwrap_or(&0.0)))
            .collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        v.into_iter()
            .enumerate()
            .map(|(i, (name, tok, cost))| {
                let priced = *self.model_priced.get(&name).unwrap_or(&false);
                let cost_source = self.model_cost_source.get(&name)
                    .cloned()
                    .unwrap_or_else(|| "none".to_string());
                ModelStat {
                    vendor: vendor_of(&name).to_string(),
                    tokens: (tok / 1e6 * 100.0).round() / 100.0,
                    cost: (cost * 100.0).round() / 100.0,
                    color: if i < PALETTE.len() { PALETTE[i] } else { OVERFLOW_GRAY }.to_string(),
                    priced,
                    cost_source,
                    name,
                }
            })
            .collect()
    }

    fn named(counts: &HashMap<String, u64>) -> Vec<NamedCount> {
        let mut v: Vec<NamedCount> = counts
            .iter()
            .map(|(k, c)| NamedCount {
                name: k.clone(),
                count: *c,
            })
            .collect();
        v.sort_by(|a, b| b.count.cmp(&a.count));
        v
    }

    fn metrics(&self, delta_tokens: f64, delta_cost: f64) -> Metrics {
        Metrics {
            total_tokens: ((self.input + self.cache + self.output + self.reasoning) / 1e6 * 100.0).round() / 100.0,
            input_tokens: (self.input / 1e6 * 100.0).round() / 100.0,
            cache_tokens: (self.cache / 1e6 * 100.0).round() / 100.0,
            output_tokens: (self.output / 1e6 * 100.0).round() / 100.0,
            reasoning_tokens: (self.reasoning / 1e6 * 100.0).round() / 100.0,
            cost: (self.cost * 100.0).round() / 100.0,
            mcp_calls: self.mcp_calls,
            skill_calls: self.skill_calls,
            requests: self.requests,
            sessions: self.sessions.len() as u64,
            delta_tokens,
            delta_cost,
            servers: self.mcp_counts.len() as u64,
            skills: self.skill_counts.len() as u64,
        }
    }

    fn agents(&self) -> Vec<AgentStat> {
        let mut v: Vec<AgentStat> = self
            .agent_tokens
            .iter()
            .map(|(agent, tokens)| AgentStat {
                agent: agent.clone(),
                tokens: (*tokens / 1e6 * 100.0).round() / 100.0,
                cost: (*self.agent_cost.get(agent).unwrap_or(&0.0) * 100.0).round() / 100.0,
                requests: *self.agent_requests.get(agent).unwrap_or(&0),
                sessions: self.agent_sessions.get(agent).map(|s| s.len() as u64).unwrap_or(0),
            })
            .collect();
        v.sort_by(|a, b| b.tokens.partial_cmp(&a.tokens).unwrap_or(std::cmp::Ordering::Equal));
        v
    }

    fn code_metrics(&self) -> CodeMetrics {
        CodeMetrics {
            additions: self.code_additions,
            deletions: self.code_deletions,
            files: self.code_files,
            diffs: self.code_diffs,
        }
    }

    fn recent_sessions(&self, limit: usize) -> Vec<SessionInfo> {
        let mut v: Vec<&SessionAccum> = self.session_map.values().collect();
        // Sort by most recently active first
        v.sort_by(|a, b| b.last_active_ms.cmp(&a.last_active_ms));
        v.truncate(limit);
        v.into_iter()
            .map(|s| {
                let ts = DateTime::from_timestamp_millis(s.time_created_ms)
                    .unwrap_or_default()
                    .with_timezone(&Local);
                SessionInfo {
                    id: s.id.clone(),
                    session_title: s.title.clone(),
                    agent: s.agent.clone(),
                    project_name: s.project_name.clone(),
                    tokens: (s.tokens / 1e6 * 100.0).round() / 100.0,
                    cost: (s.cost * 100.0).round() / 100.0,
                    duration_secs: (s.dur_ms.max(0) / 1000) as u64,
                    time_created: ts.to_rfc3339(),
                }
            })
            .collect()
    }
}

/// Percentage change of `cur` vs `prev`, e.g. +20.0 for a 20% increase,
/// rounded to 2 decimals. Returns 0 when there's no baseline to compare.
fn pct_delta(cur: f64, prev: f64) -> f64 {
    if prev <= 0.0 {
        return 0.0;
    }
    ((cur - prev) / prev * 10000.0).round() / 100.0
}

/// Aggregate project statistics from a slice of events (for a specific time period).
fn aggregate_projects(events: &[&Event]) -> Vec<ProjectStat> {
    let mut proj_tokens: HashMap<String, f64> = HashMap::new();
    let mut proj_cost: HashMap<String, f64> = HashMap::new();
    let mut proj_sessions: HashMap<String, HashSet<String>> = HashMap::new();
    let mut proj_name: HashMap<String, String> = HashMap::new();
    let mut proj_pid: HashMap<String, String> = HashMap::new();

    for e in events {
        // Merged display (OpenCode + Codex) groups by project *name*, so the
        // same repo appears as one row regardless of which source's id it
        // carries (OpenCode project ids vs Codex cwd paths).
        let key = if e.project_name.is_empty() {
            if e.project_id.is_empty() {
                "global".to_string()
            } else {
                e.project_id.clone()
            }
        } else {
            e.project_name.clone()
        };
        let pname = if e.project_name.is_empty() {
            "Global".to_string()
        } else {
            e.project_name.clone()
        };
        proj_pid.entry(key.clone()).or_insert_with(|| {
            if e.project_id.is_empty() {
                "global".to_string()
            } else {
                e.project_id.clone()
            }
        });
        *proj_tokens.entry(key.clone()).or_default() += (e.input + e.cache + e.output + e.reasoning) / 1e6;
        *proj_cost.entry(key.clone()).or_default() += e.cost;
        proj_sessions
            .entry(key.clone())
            .or_default()
            .insert(e.session.clone());
        proj_name.entry(key.clone()).or_insert(pname);
    }

    let mut projects: Vec<ProjectStat> = proj_tokens
        .iter()
        .map(|(key, tokens)| ProjectStat {
            project_id: proj_pid.get(key).cloned().unwrap_or_else(|| key.clone()),
            project_name: proj_name.get(key).cloned().unwrap_or_default(),
            worktree: String::new(),
            tokens: (tokens * 100.0).round() / 100.0,
            cost: (proj_cost.get(key).copied().unwrap_or(0.0) * 100.0).round() / 100.0,
            sessions: proj_sessions
                .get(key)
                .map(|s| s.len() as u64)
                .unwrap_or(0),
        })
        .collect();
    projects.sort_by(|a, b| b.tokens.partial_cmp(&a.tokens).unwrap_or(std::cmp::Ordering::Equal));
    projects
}

// ── Day report: today, 24 hourly buckets ───────────────────────────
fn report_day(events: &[Event], now: DateTime<Local>, utc_day: bool) -> PeriodReport {
    // "Platform day" (UTC) mirrors DeepSeek's usage dashboard day boundary; the
    // default local day is the user's calendar day.
    let today = if utc_day {
        now.with_timezone(&Utc).date_naive()
    } else {
        now.date_naive()
    };
    let yesterday = today - Duration::days(1);
    let mut agg = Agg::default();
    let mut prev = Agg::default();
    let mut buckets = vec![(0.0f64, 0.0f64, 0.0f64, 0.0f64); 24]; // (input, cache, output, reasoning) M
    let mut req_b = vec![0.0f64; 24];
    let mut cost_b = vec![0.0f64; 24];
    let mut period_events: Vec<&Event> = Vec::new();

    for e in events {
        let d = if utc_day {
            e.ts.with_timezone(&Utc).date_naive()
        } else {
            e.ts.date_naive()
        };
        if d == today {
            period_events.push(e);
            agg.add(e);
            let h = if utc_day {
                e.ts.with_timezone(&Utc).hour() as usize
            } else {
                e.ts.hour() as usize
            };
            buckets[h].0 += e.input / 1e6;
            buckets[h].1 += e.cache / 1e6;
            buckets[h].2 += e.output / 1e6;
            buckets[h].3 += e.reasoning / 1e6;
            // Match Agg::add exactly: only the request COUNT excludes model-less
            // (slash-command) events; total cost accumulates unconditionally
            // (those events carry cost 0, so this is identical today).
            if !e.model.is_empty() {
                req_b[h] += 1.0;
            }
            cost_b[h] += e.cost;
        } else if d == yesterday {
            prev.add(e);
        }
    }

    let series = (0..24)
        .map(|h| SeriesPoint {
            // axis ticks every 4h, skipping the 00/24 endpoints
            label: if h % 4 == 0 && h != 0 {
                format!("{:02}", h)
            } else {
                String::new()
            },
            full: format!("{:02}:00", h),
            input: buckets[h].0,
            cache: buckets[h].1,
            output: buckets[h].2,
            reasoning: buckets[h].3,
        })
        .collect();

    let projects = aggregate_projects(&period_events);
    debug_assert!(
        // Per-row 2-decimal rounding drifts a few cents; compare the unrounded
        // total against the rounded project sum with a tolerant bound.
        (agg.cost - projects.iter().map(|p| p.cost).sum::<f64>()).abs() < 0.05,
        "model cost sum != project cost sum"
    );
    PeriodReport {
        metrics: agg.metrics(
            pct_delta(
                agg.input + agg.cache + agg.output + agg.reasoning,
                prev.input + prev.cache + prev.output + prev.reasoning,
            ),
            pct_delta(agg.cost, prev.cost),
        ),
        series,
        models: agg.models(),
        agents: agg.agents(),

        mcp: Agg::named(&agg.mcp_counts),
        skills: Agg::named(&agg.skill_counts),
        projects,
        req_trend: req_b,
        cost_trend: cost_b,
    }
}

// ── Week report: current calendar week (Mon-Sun) vs previous week ────
fn report_week(events: &[Event], now: DateTime<Local>) -> PeriodReport {
    let today = now.date_naive();
    // Monday of the current week (Mon=0 … Sun=6).
    let start = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let next_start = start + Duration::days(7);
    let prev_start = start - Duration::days(7);

    let mut agg = Agg::default();
    let mut prev = Agg::default();
    let mut buckets = vec![(0.0f64, 0.0f64, 0.0f64, 0.0f64); 7];
    let mut req_b = vec![0.0f64; 7];
    let mut cost_b = vec![0.0f64; 7];
    let mut period_events: Vec<&Event> = Vec::new();

    for e in events {
        let d = e.ts.date_naive();
        if d >= start && d < next_start {
            period_events.push(e);
            agg.add(e);
            let idx = (d - start).num_days() as usize;
            if idx < buckets.len() {
                buckets[idx].0 += e.input / 1e6;
                buckets[idx].1 += e.cache / 1e6;
                buckets[idx].2 += e.output / 1e6;
                buckets[idx].3 += e.reasoning / 1e6;
                // Match Agg::add: only the request COUNT excludes model-less
                // events; cost accumulates unconditionally (their cost is 0).
                if !e.model.is_empty() {
                    req_b[idx] += 1.0;
                }
                cost_b[idx] += e.cost;
            }
        } else if d >= prev_start && d < start {
            prev.add(e);
        }
    }

    let weekday = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let series = (0..7usize)
        .map(|i| {
            let date = start + Duration::days(i as i64);
            let wd = weekday[i];
            SeriesPoint {
                label: wd.to_string(),
                full: format!("{} {} {}", wd, MONTHS[(date.month() - 1) as usize], date.day()),
                input: buckets[i].0,
                cache: buckets[i].1,
                output: buckets[i].2,
                reasoning: buckets[i].3,
            }
        })
        .collect();

    let projects = aggregate_projects(&period_events);
    debug_assert!(
        (agg.cost - projects.iter().map(|p| p.cost).sum::<f64>()).abs() < 0.05,
        "model cost sum != project cost sum"
    );
    PeriodReport {
        metrics: agg.metrics(
            pct_delta(
                agg.input + agg.cache + agg.output + agg.reasoning,
                prev.input + prev.cache + prev.output + prev.reasoning,
            ),
            pct_delta(agg.cost, prev.cost),
        ),
        series,
        models: agg.models(),
        agents: agg.agents(),

        mcp: Agg::named(&agg.mcp_counts),
        skills: Agg::named(&agg.skill_counts),
        projects,
        req_trend: req_b,
        cost_trend: cost_b,
    }
}

// ── Month report: current calendar month vs previous calendar month ──
fn report_month(events: &[Event], now: DateTime<Local>) -> PeriodReport {
    use chrono::NaiveDate;
    let today = now.date_naive();
    let (y, m) = (today.year(), today.month());
    let cur_first = NaiveDate::from_ymd_opt(y, m, 1).unwrap();
    let next_first = if m == 12 {
        NaiveDate::from_ymd_opt(y + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(y, m + 1, 1).unwrap()
    };
    let (py, pm) = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
    let prev_first = NaiveDate::from_ymd_opt(py, pm, 1).unwrap();
    let days_in_month = (next_first - cur_first).num_days() as usize;

    let mut agg = Agg::default();
    let mut prev = Agg::default();
    let mut buckets = vec![(0.0f64, 0.0f64, 0.0f64, 0.0f64); days_in_month];
    let mut req_b = vec![0.0f64; days_in_month];
    let mut cost_b = vec![0.0f64; days_in_month];
    let mut period_events: Vec<&Event> = Vec::new();

    for e in events {
        let d = e.ts.date_naive();
        if d >= cur_first && d < next_first {
            period_events.push(e);
            agg.add(e);
            let idx = (d - cur_first).num_days() as usize;
            if idx < buckets.len() {
                buckets[idx].0 += e.input / 1e6;
                buckets[idx].1 += e.cache / 1e6;
                buckets[idx].2 += e.output / 1e6;
                buckets[idx].3 += e.reasoning / 1e6;
                // Match Agg::add: only the request COUNT excludes model-less
                // events; cost accumulates unconditionally (their cost is 0).
                if !e.model.is_empty() {
                    req_b[idx] += 1.0;
                }
                cost_b[idx] += e.cost;
            }
        } else if d >= prev_first && d < cur_first {
            prev.add(e);
        }
    }

    let series = (0..days_in_month)
        .map(|i| {
            let dn = (i + 1) as u32;
            let label = if i == 0 || dn % 5 == 0 {
                dn.to_string()
            } else {
                String::new()
            };
            SeriesPoint {
                label,
                full: format!("{} {}", MONTHS[(m - 1) as usize], dn),
                input: buckets[i].0,
                cache: buckets[i].1,
                output: buckets[i].2,
                reasoning: buckets[i].3,
            }
        })
        .collect();

    let projects = aggregate_projects(&period_events);
    debug_assert!(
        (agg.cost - projects.iter().map(|p| p.cost).sum::<f64>()).abs() < 0.05,
        "model cost sum != project cost sum"
    );
    PeriodReport {
        metrics: agg.metrics(
            pct_delta(
                agg.input + agg.cache + agg.output + agg.reasoning,
                prev.input + prev.cache + prev.output + prev.reasoning,
            ),
            pct_delta(agg.cost, prev.cost),
        ),
        series,
        models: agg.models(),
        agents: agg.agents(),

        mcp: Agg::named(&agg.mcp_counts),
        skills: Agg::named(&agg.skill_counts),
        projects,
        req_trend: req_b,
        cost_trend: cost_b,
    }
}

// ── Heatmap: last ~26 weeks daily totals ────────────────────────────
fn build_heatmap(events: &[Event], today: chrono::NaiveDate) -> Vec<HeatDay> {
    let start = today - Duration::days(25 * 7 + today.weekday().num_days_from_sunday() as i64);
    let mut by_day: HashMap<chrono::NaiveDate, f64> = HashMap::new();
    for e in events {
        let d = e.ts.date_naive();
        if d >= start && d <= today {
            *by_day.entry(d).or_default() += (e.input + e.cache + e.output + e.reasoning) / 1e6;
        }
    }
    let mut days = Vec::new();
    let mut d = start;
    let mut maxv = 0.0f64;
    while d <= today {
        let t = *by_day.get(&d).unwrap_or(&0.0);
        maxv = maxv.max(t);
        days.push((d, t));
        d += Duration::days(1);
    }
    days.into_iter()
        .map(|(date, tokens)| {
            let f = if maxv > 0.0 { tokens / maxv } else { 0.0 };
            let level = if tokens == 0.0 {
                0
            } else if f < 0.25 {
                1
            } else if f < 0.5 {
                2
            } else if f < 0.75 {
                3
            } else {
                4
            };
            HeatDay {
                date: date.format("%Y-%m-%d").to_string(),
                tokens: (tokens * 100.0).round() / 100.0,
                level,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn event_at(ts: &str, input: f64) -> Event {
        let ts = DateTime::parse_from_rfc3339(ts)
            .unwrap()
            .with_timezone(&Local);
        Event {
            ts,
            session: "s".to_string(),
            model: "m".to_string(),
            input,
            cache: 0.0,
            output: 0.0,
            reasoning: 0.0,
            cost: 0.0,
            priced: true,
            cost_source: "pricing".to_string(),
            mcp: Vec::new(),
            skills: Vec::new(),
            project_id: "p".to_string(),
            project_name: "p".to_string(),
            agent: "a".to_string(),
            code_additions: 0,
            code_deletions: 0,
            code_files: 0,
            code_diffs: 0,
            session_duration_ms: 0,
            session_time_created_ms: 0,
            session_title: String::new(),
        }
    }

    /// 2026-08-02T17:30:00Z == 2026-08-03 01:30 local (+08): inside the local
    /// day but outside the UTC ("platform") day.
    #[test]
    fn platform_day_boundary_excludes_early_local_morning() {
        let ev = event_at("2026-08-02T17:30:00Z", 2_000_000.0);
        let now = Local.with_ymd_and_hms(2026, 8, 3, 9, 30, 0).unwrap();

        let local = report_day(&[ev.clone()], now, false);
        assert_eq!(local.metrics.total_tokens, 2.0);
        assert_eq!(local.series[1].input, 2.0); // local hour 01

        let utc = report_day(&[ev.clone()], now, true);
        assert_eq!(utc.metrics.total_tokens, 0.0); // UTC date is Aug 2 → not today
        assert!(utc.series.iter().all(|p| p.input == 0.0));
    }
}
