//! Codex feasibility prototype: parse Codex session transcripts into `RawEvent`.
//!
//! Codex stores one JSONL transcript per session under `~/.codex/sessions/YYYY/MM/DD/`
//! (recent/active) and `~/.codex/archived_sessions/` (archived). The relevant events:
//!
//! - `session_meta`             → session id, `cwd` (project), originator, source
//! - `turn_context` / `task_started` → session-level model id
//! - `event_msg`/`token_count`  → per-turn token usage (`total_token_usage` is
//!   cumulative across the session, so per-turn = current − previous)
//! - `response_item`/`function_call` → tool calls; MCP tools carry a
//!   `namespace` of `mcp__<server>` (e.g. `mcp__node_repl`)
//!
//! Not wired into the app yet: this module exists so the feasibility example
//! (`cargo run --example dump_codex`) can run Codex data through the same
//! aggregation pipeline (`parser::build_dashboard_from`). Fields Codex doesn't
//! record (per-message cost, skills, cache-write for older transcripts) fall
//! back to zero / the pricing module, exactly like unknown OpenCode models.

use crate::store::RawEvent;
use chrono::DateTime;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

fn codex_home() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".codex"))
}

/// Recursively find transcript files under `~/.codex/sessions` and
/// `~/.codex/archived_sessions`.
fn find_transcripts() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(home) = codex_home() else { return out };
    for root in ["sessions", "archived_sessions"] {
        collect_jsonl(&home.join(root), &mut out);
    }
    out.sort();
    out
}

fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_jsonl(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            out.push(p);
        }
    }
}

/// `~/.codex/session_index.jsonl`: `{"id", "thread_name", "updated_at"}` per line.
fn load_titles() -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(home) = codex_home() else { return map };
    let Ok(text) = fs::read_to_string(home.join("session_index.jsonl")) else {
        return map;
    };
    for line in text.lines() {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if let (Some(id), Some(name)) = (
                v.get("id").and_then(|x| x.as_str()),
                v.get("thread_name").and_then(|x| x.as_str()),
            ) {
                map.insert(id.to_string(), name.to_string());
            }
        }
    }
    map
}

fn ts_ms(s: &str) -> i64 {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}

#[derive(Default, Clone, Copy)]
struct Tokens {
    input: f64,
    cc: f64,
    cr: f64,
    output: f64,
    reasoning: f64,
}

fn usage_of(v: &Value) -> Tokens {
    let g = |k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
    Tokens {
        input: g("input_tokens"),
        cc: g("cache_write_input_tokens"), // absent in older transcripts → 0
        cr: g("cached_input_tokens"),
        output: g("output_tokens"),
        reasoning: g("reasoning_output_tokens"),
    }
}

/// `"mcp__node_repl"` → `"node_repl"`; non-MCP namespaces → `None`.
fn mcp_server_from_namespace(namespace: &str) -> Option<String> {
    let server = namespace.strip_prefix("mcp__")?;
    if server.is_empty() {
        None
    } else {
        Some(server.to_string())
    }
}

/// Parse every Codex transcript into a time-sorted `RawEvent` list.
pub fn load_events() -> Vec<RawEvent> {
    let titles = load_titles();
    let mut events: Vec<RawEvent> = Vec::new();
    let mut sess_first: HashMap<String, i64> = HashMap::new();
    let mut sess_last: HashMap<String, i64> = HashMap::new();

    for path in find_transcripts() {
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let mut session = String::new();
        let mut cwd = String::new();
        let mut originator = String::new();
        let mut source = String::new();
        let mut model = String::new();
        let mut created_ms: i64 = 0;
        let mut prev: Option<Tokens> = None;
        let mut pending_mcp: Vec<String> = Vec::new();

        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
            let typ = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
            let ts = v
                .get("timestamp")
                .and_then(|x| x.as_str())
                .map(ts_ms)
                .unwrap_or(0);

            match typ {
                "session_meta" => {
                    if let Some(p) = v.get("payload") {
                        session = p
                            .get("session_id")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        cwd = p.get("cwd").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        originator = p
                            .get("originator")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        source = p.get("source").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        created_ms = p
                            .get("timestamp")
                            .and_then(|x| x.as_str())
                            .map(ts_ms)
                            .unwrap_or(ts);
                    }
                }
                "turn_context" => {
                    if let Some(m) = v.pointer("/payload/model").and_then(|x| x.as_str()) {
                        model = m.to_string();
                    }
                }
                "event_msg" => {
                    let Some(p) = v.get("payload") else { continue };
                    let etype = p.get("type").and_then(|x| x.as_str()).unwrap_or("");
                    match etype {
                        "task_started" => {
                            if model.is_empty() {
                                if let Some(m) = p
                                    .pointer("/collaboration_mode/settings/model")
                                    .and_then(|x| x.as_str())
                                {
                                    model = m.to_string();
                                }
                            }
                        }
                        "token_count" => {
                            let Some(info) = p.get("info") else { continue };
                            let Some(total) = info.get("total_token_usage") else { continue };
                            let cur = usage_of(total);
                            // total_token_usage is cumulative across the session;
                            // per-turn = current − previous (clamped ≥ 0).
                            let per_turn = match prev {
                                None => cur,
                                Some(prev_t) => Tokens {
                                    input: (cur.input - prev_t.input).max(0.0),
                                    cc: (cur.cc - prev_t.cc).max(0.0),
                                    cr: (cur.cr - prev_t.cr).max(0.0),
                                    output: (cur.output - prev_t.output).max(0.0),
                                    reasoning: (cur.reasoning - prev_t.reasoning).max(0.0),
                                },
                            };
                            prev = Some(cur);
                            if per_turn.input
                                + per_turn.cc
                                + per_turn.cr
                                + per_turn.output
                                + per_turn.reasoning
                                <= 0.0
                            {
                                continue;
                            }
                            let project_name = cwd
                                .rsplit('/')
                                .next()
                                .filter(|s| !s.is_empty())
                                .unwrap_or(&cwd)
                                .to_string();
                            let sess = session.clone();
                            let ev = RawEvent {
                                ts_ms: ts,
                                session: sess.clone(),
                                model: if model.is_empty() {
                                    "unknown".to_string()
                                } else {
                                    model.clone()
                                },
                                in_tok: per_turn.input,
                                cc: per_turn.cc,
                                cr: per_turn.cr,
                                out_tok: per_turn.output,
                                reasoning: per_turn.reasoning,
                                mcp: std::mem::take(&mut pending_mcp),
                                skills: Vec::new(),
                                id: format!("{sess}:{ts}"),
                                source: "codex".to_string(),
                                msg_count: 1,
                                stored_cost: None,
                                project_id: cwd.clone(),
                                project_name,
                                agent: if originator.is_empty() {
                                    source.clone()
                                } else {
                                    originator.clone()
                                },
                                code_additions: 0,
                                code_deletions: 0,
                                code_files: 0,
                                code_diffs: 0,
                                session_duration_ms: 0,
                                session_time_created_ms: created_ms,
                                session_title: titles.get(&sess).cloned().unwrap_or_default(),
                            };
                            sess_first.entry(sess.clone()).or_insert(ev.ts_ms);
                            let last = sess_last.entry(sess.clone()).or_insert(ev.ts_ms);
                            if ev.ts_ms > *last {
                                *last = ev.ts_ms;
                            }
                            events.push(ev);
                        }
                        _ => {}
                    }
                }
                "response_item" => {
                    if v.pointer("/payload/type").and_then(|x| x.as_str()) == Some("function_call")
                    {
                        let server = v
                            .pointer("/payload/namespace")
                            .and_then(|x| x.as_str())
                            .and_then(mcp_server_from_namespace)
                            .or_else(|| {
                                // Legacy/fallback: some transcripts put the MCP
                                // prefix directly in the tool name.
                                v.pointer("/payload/name")
                                    .and_then(|x| x.as_str())
                                    .and_then(mcp_server_from_namespace)
                            });
                        if let Some(server) = server {
                            pending_mcp.push(server);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Fill session duration (last − first) and creation time.
    for ev in &mut events {
        let f = sess_first.get(&ev.session).copied().unwrap_or(0);
        let l = sess_last.get(&ev.session).copied().unwrap_or(0);
        if ev.session_time_created_ms <= 0 {
            ev.session_time_created_ms = f;
        }
        ev.session_duration_ms = (l - f).max(0);
    }

    events.sort_by_key(|e| e.ts_ms);
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_namespace_extraction() {
        assert_eq!(
            mcp_server_from_namespace("mcp__node_repl"),
            Some("node_repl".to_string())
        );
        assert_eq!(
            mcp_server_from_namespace("mcp__codex_apps__github"),
            Some("codex_apps__github".to_string())
        );
        assert_eq!(mcp_server_from_namespace("multi_agent_v1"), None);
        assert_eq!(mcp_server_from_namespace("exec_command"), None);
        assert_eq!(mcp_server_from_namespace("mcp__"), None);
    }

    /// Machine-specific: requires real `~/.codex` transcripts on this machine.
    #[test]
    #[ignore]
    fn loads_real_codex_events_with_mcp() {
        let events = load_events();
        assert!(!events.is_empty(), "no Codex transcripts found");
        let mcp_calls: usize = events.iter().map(|e| e.mcp.len()).sum();
        let sources: std::collections::HashSet<&str> =
            events.iter().map(|e| e.source.as_str()).collect();
        println!(
            "codex events={} mcp_calls={} sources={:?}",
            events.len(),
            mcp_calls,
            sources
        );
        assert!(mcp_calls > 0, "expected real mcp__ tool invocations");
    }
}
