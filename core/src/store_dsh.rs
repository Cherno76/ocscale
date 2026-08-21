//! DeepSeek Harness (DSH) data source: parse DSH session logs into `RawEvent`.
//!
//! DSH persists each session as an append-only JSONL log (optionally zstd
//! compressed) under `~/.dsh/sessions/<project>/<session-id>/session.jsonl[.zstd]`
//! (its SQLite backend stores the same event stream in a `sessions`/`events`
//! table pair). The relevant records:
//!
//! - `session` (header line)  → session id, `cwd` (project), `createdAt`,
//!   `agentPreset`
//! - `request/context`        → `provider` + `model` for the next request
//! - `assistant/message`      → one model call per step; `usage` carries the
//!   step's token accounting. `inputTokens` is already uncached input — cache
//!   hits are reported separately as `cacheReadTokens`, so (unlike Codex) no
//!   input − cached subtraction is needed.
//! - `tool/call`              → tool invocations; MCP tools carry a public name
//!   of `mcp__<server>__<tool>`
//! - `session/title`          → session title
//!
//! Fields DSH does not record (per-message cost, skills) fall back to zero /
//! the pricing module, exactly like unknown OpenCode models.
//!
//! Standalone dump: `cd src-tauri && cargo run --example dump_dsh`.
use crate::store::RawEvent;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// DSH home directory: `$DSH_HOME` when set, else `~/.dsh`.
fn dsh_home() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("DSH_HOME") {
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    dirs::home_dir().map(|h| h.join(".dsh"))
}

/// Recursively find session-log files under `~/.dsh/sessions`.
fn find_session_logs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(home) = dsh_home() else { return out };
    collect_logs(&home.join("sessions"), &mut out);
    out.sort();
    out
}

fn collect_logs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_logs(&p, out);
        } else {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "session.jsonl" || name == "session.jsonl.zstd" {
                out.push(p);
            }
        }
    }
}

/// Read a session log, transparently decompressing `.zstd` artifacts.
fn read_log(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    if path.to_string_lossy().ends_with(".zstd") {
        let decoded = zstd::stream::decode_all(bytes.as_slice()).ok()?;
        Some(String::from_utf8_lossy(&decoded).into_owned())
    } else {
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// `"mcp__node_repl__run"` → `"node_repl"`; non-MCP tool names → `None`.
fn mcp_server_from_tool_name(name: &str) -> Option<String> {
    let rest = name.strip_prefix("mcp__")?;
    let server = rest.split("__").next()?;
    if server.is_empty() { None } else { Some(server.to_string()) }
}

/// Memoized DSH events, keyed by log-file mtimes, so the 30s dashboard refresh
/// only reparses when a session log actually changed.
static DSH_CACHE: OnceLock<Mutex<Option<(Vec<(PathBuf, SystemTime)>, Arc<Vec<RawEvent>>)>>> =
    OnceLock::new();

/// Load DSH events, reusing the cache when no session log has changed.
pub fn cached_events() -> Arc<Vec<RawEvent>> {
    let lock = DSH_CACHE.get_or_init(|| Mutex::new(None));
    let mut g = lock.lock().unwrap_or_else(|e| e.into_inner());
    let files = find_session_logs();
    let stamp: Vec<(PathBuf, SystemTime)> = files
        .iter()
        .map(|p| {
            let m = fs::metadata(p)
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            (p.clone(), m)
        })
        .collect();
    if let Some((prev, events)) = g.as_ref() {
        if *prev == stamp {
            return events.clone();
        }
    }
    let events = Arc::new(load_events_from(&files));
    *g = Some((stamp, events.clone()));
    events
}

/// Parse every DSH session log into a time-sorted `RawEvent` list.
pub fn load_events() -> Vec<RawEvent> {
    load_events_from(&find_session_logs())
}

fn load_events_from(files: &[PathBuf]) -> Vec<RawEvent> {
    let mut events: Vec<RawEvent> = Vec::new();

    for path in files {
        let Some(text) = read_log(path) else { continue };

        let mut session = String::new();
        let mut cwd = String::new();
        let mut agent = String::new();
        let mut title = String::new();
        let mut model = String::new();
        let mut created_ms: i64 = 0;
        let mut first_ms: i64 = i64::MAX;
        let mut last_ms: i64 = 0;
        // MCP server names collected per (turn, step), attached to the
        // assistant/message of the same step after the whole log is read
        // (`tool/call` lines follow their `assistant/message` in the log).
        let mut mcp_by_step: HashMap<(i64, i64), Vec<String>> = HashMap::new();
        // (event, turn, step) skeletons awaiting their step's MCP servers and
        // the session-level title/duration.
        let mut skeletons: Vec<(RawEvent, i64, i64)> = Vec::new();

        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
            let typ = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
            match typ {
                "session" => {
                    session = v
                        .get("id")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    cwd = v
                        .get("cwd")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    created_ms = v.get("createdAt").and_then(|x| x.as_i64()).unwrap_or(0);
                    agent = v
                        .get("agentPreset")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                }
                "session/title" => {
                    if let Some(t) = v.pointer("/data/title").and_then(|x| x.as_str()) {
                        title = t.to_string();
                    }
                }
                "request/context" => {
                    if let Some(m) = v.pointer("/data/model").and_then(|x| x.as_str()) {
                        model = m.to_string();
                    }
                }
                "request/header" => {
                    if model.is_empty() {
                        if let Some(m) = v
                            .pointer("/data/header/config/model")
                            .and_then(|x| x.as_str())
                        {
                            model = m.to_string();
                        }
                    }
                }
                "tool/call" => {
                    if let Some(name) = v.pointer("/data/name").and_then(|x| x.as_str()) {
                        if let Some(server) = mcp_server_from_tool_name(name) {
                            let turn = v
                                .pointer("/data/turn")
                                .and_then(|x| x.as_i64())
                                .unwrap_or(0);
                            let step = v
                                .pointer("/data/step")
                                .and_then(|x| x.as_i64())
                                .unwrap_or(0);
                            mcp_by_step.entry((turn, step)).or_default().push(server);
                        }
                    }
                }
                "assistant/message" => {
                    let Some(usage) = v.pointer("/data/usage") else { continue };
                    let g = |k: &str| usage.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
                    let in_tok = g("inputTokens");
                    let cc = g("cacheWriteTokens");
                    let cr = g("cacheReadTokens");
                    let out_tok = g("outputTokens");
                    let reasoning = g("reasoningTokens");
                    if in_tok + cc + cr + out_tok + reasoning <= 0.0 {
                        continue;
                    }
                    let ts_ms = v.get("time").and_then(|x| x.as_i64()).unwrap_or(0);
                    let seq = v.get("seq").and_then(|x| x.as_i64()).unwrap_or(0);
                    let turn = v.pointer("/data/turn").and_then(|x| x.as_i64()).unwrap_or(0);
                    let step = v.pointer("/data/step").and_then(|x| x.as_i64()).unwrap_or(0);
                    first_ms = first_ms.min(ts_ms);
                    last_ms = last_ms.max(ts_ms);
                    let project_name = cwd
                        .rsplit(['/', '\\'])
                        .next()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&cwd)
                        .to_string();
                    let ev = RawEvent {
                        ts_ms,
                        session: session.clone(),
                        model: if model.is_empty() {
                            "unknown".to_string()
                        } else {
                            model.clone()
                        },
                        in_tok,
                        cc,
                        cr,
                        out_tok,
                        reasoning,
                        mcp: Vec::new(),
                        skills: Vec::new(),
                        id: format!("{session}:{seq}"),
                        source: "dsh".to_string(),
                        msg_count: 1,
                        stored_cost: None,
                        project_id: cwd.clone(),
                        project_name,
                        // Prefix DSH agents so they're distinguishable from
                        // OpenCode's and Codex's agents in the merged stats
                        // ("DSH-cordis"), matching OpenCode's "OpenCode-" scheme.
                        agent: if agent.is_empty() {
                            "DSH".to_string()
                        } else {
                            format!("DSH-{agent}")
                        },
                        code_additions: 0,
                        code_deletions: 0,
                        code_files: 0,
                        code_diffs: 0,
                        session_duration_ms: 0,
                        session_time_created_ms: created_ms,
                        session_title: String::new(),
                    };
                    skeletons.push((ev, turn, step));
                }
                _ => {}
            }
        }

        for (mut ev, turn, step) in skeletons {
            if let Some(mcp) = mcp_by_step.get(&(turn, step)) {
                ev.mcp = mcp.clone();
            }
            ev.session_duration_ms = (last_ms - first_ms).max(0);
            ev.session_title = title.clone();
            events.push(ev);
        }
    }

    events.sort_by_key(|e| e.ts_ms);
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `mcp__server__tool` names yield the server; built-ins yield nothing.
    #[test]
    fn mcp_tool_name_extraction() {
        assert_eq!(
            mcp_server_from_tool_name("mcp__node_repl__run"),
            Some("node_repl".to_string())
        );
        assert_eq!(
            mcp_server_from_tool_name("mcp__github__create_issue"),
            Some("github".to_string())
        );
        assert_eq!(mcp_server_from_tool_name("mcp__"), None);
        assert_eq!(mcp_server_from_tool_name("bash"), None);
        assert_eq!(mcp_server_from_tool_name("read"), None);
    }

    /// Regression: DSH `inputTokens` is already uncached, so it maps straight to
    /// `in_tok` (no subtraction), cache hits land in `cr`, and MCP calls attach
    /// to the matching step only.
    #[test]
    fn parses_sample_dsh_log() {
        let dir = std::env::temp_dir().join(format!("ocscale-dsh-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        let lines = [
            r#"{"type":"session","version":0,"id":"s1","createdAt":1000,"cwd":"/tmp/proj","agentPreset":"cordis"}"#,
            r#"{"type":"request/context","seq":1,"time":1001,"data":{"provider":"deepseek-official","model":"deepseek-v4-pro","contextWindow":1000000}}"#,
            r#"{"type":"assistant/message","seq":2,"time":1100,"data":{"turn":1,"step":1,"usage":{"inputTokens":1000,"outputTokens":100,"cacheReadTokens":800,"cacheWriteTokens":50,"reasoningTokens":25}}}"#,
            r#"{"type":"tool/call","seq":3,"time":1110,"data":{"turn":1,"step":1,"callId":"c1","name":"mcp__node_repl__run","arguments":"{}"}}"#,
            r#"{"type":"tool/call","seq":4,"time":1120,"data":{"turn":1,"step":1,"callId":"c2","name":"bash","arguments":"{}"}}"#,
            r#"{"type":"assistant/message","seq":5,"time":1200,"data":{"turn":1,"step":2,"usage":{"inputTokens":2000,"outputTokens":200,"cacheReadTokens":0,"cacheWriteTokens":0,"reasoningTokens":0}}}"#,
            r#"{"type":"tool/call","seq":6,"time":1210,"data":{"turn":1,"step":2,"callId":"c3","name":"read","arguments":"{}"}}"#,
            r#"{"type":"session/title","seq":7,"time":1300,"data":{"title":"fix bug"}}"#,
        ];
        std::fs::write(&path, lines.join("\n")).unwrap();
        let events = load_events_from(&[path.clone()]);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(events.len(), 2);

        // Step 1: usage maps directly; only the MCP tool is kept.
        assert_eq!(events[0].in_tok, 1000.0);
        assert_eq!(events[0].cc, 50.0);
        assert_eq!(events[0].cr, 800.0);
        assert_eq!(events[0].out_tok, 100.0);
        assert_eq!(events[0].reasoning, 25.0);
        assert_eq!(events[0].model, "deepseek-v4-pro");
        assert_eq!(events[0].project_name, "proj");
        assert_eq!(events[0].agent, "DSH-cordis");
        assert_eq!(events[0].mcp, vec!["node_repl".to_string()]);
        assert_eq!(events[0].source, "dsh");
        assert_eq!(events[0].id, "s1:2");

        // Step 2: no MCP (read is a built-in); model carried forward.
        assert_eq!(events[1].in_tok, 2000.0);
        assert_eq!(events[1].mcp, Vec::<String>::new());
        assert_eq!(events[1].model, "deepseek-v4-pro");

        // Session-level title + duration applied after the whole log is read.
        assert_eq!(events[0].session_title, "fix bug");
        assert_eq!(events[0].session_duration_ms, 100); // 1100 → 1200
    }
}
