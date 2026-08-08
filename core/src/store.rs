// Incremental event store reading from OpenCode's SQLite database.
//
// Reads individual assistant messages from the `message` table, each with its
// own timestamp and token counts. This gives accurate hourly distribution in
// the daily chart, unlike per-session aggregates which lump all tokens into
// one time bucket.
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawEvent {
    pub ts_ms: i64,
    pub session: String,
    pub model: String,
    pub in_tok: f64,
    pub cc: f64,  // cache creation (tokens_cache_write)
    pub cr: f64,  // cache read (tokens_cache_read)
    pub out_tok: f64,
    pub reasoning: f64,    // reasoning tokens
    pub mcp: Vec<String>,    // MCP server names called in this message
    pub skills: Vec<String>, // Skill names called in this message
    pub id: String,          // message id
    pub source: String,      // always "opencode"
    /// Always 1 for per-message events (each message = 1 request).
    pub msg_count: u64,
    /// Per-message cost from OpenCode. Falls back when pricing module
    /// doesn't recognise the model.
    pub stored_cost: Option<f64>,
    pub project_id: String,
    pub project_name: String,
    // ── session-level fields (denormalized per message) ──────────
    /// Agent role that produced this session (build, explorer, etc.)
    pub agent: String,
    /// Lines added across the whole session
    pub code_additions: u64,
    /// Lines deleted across the whole session
    pub code_deletions: u64,
    /// Files changed across the whole session
    pub code_files: u64,
    /// Diff count across the whole session
    pub code_diffs: u64,
    /// Session duration in milliseconds (time_updated - time_created)
    pub session_duration_ms: i64,
    /// Session creation time in ms epoch (from session table)
    pub session_time_created_ms: i64,
    /// Session title (user-provided or auto-generated)
    pub session_title: String,
}

pub struct Store {
    pub events: Vec<RawEvent>,
}

/// Locate the OpenCode SQLite database.
fn opencode_db_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        let p = PathBuf::from(xdg).join("opencode/opencode.db");
        if p.exists() {
            return Some(p);
        }
    }
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".local/share/opencode/opencode.db");
        if p.exists() {
            return Some(p);
        }
    }
    if let Some(d) = dirs::data_dir() {
        let p = d.join("opencode/opencode.db");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Open the OpenCode DB strictly read-only. We never write to OpenCode's data,
/// and a read-write open would need to create/update the WAL/SHM sidecar files
/// in OpenCode's directory (blocked under sandboxes / read-only mounts, which
/// previously made `query_events` fail and silently return an empty dashboard).
/// If a hot WAL without a readable `-shm` blocks even a read-only open, fall
/// back to `immutable=1` (ignores journal files; safe for our read-only queries
/// on a checkpointed DB).
fn open_db_readonly(path: &PathBuf) -> rusqlite::Result<Connection> {
    if let Ok(c) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        // WAL-mode read-only access fails *lazily* on the first real read when
        // the `-shm` sidecar is missing/unreadable (SQLITE_CANTOPEN). Reading
        // sqlite_master forces an actual page read so we fall back before
        // running the real queries.
        if c.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
            .is_ok()
        {
            return Ok(c);
        }
    }
    // Last resort for restricted environments (sandboxes, read-only mounts):
    // immutable ignores journal/shm entirely. May lag uncheckpointed WAL
    // writes from an actively-running OpenCode CLI; acceptable for the 30s poll.
    Connection::open_with_flags(
        format!("file:{}?immutable=1", path.display()),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
}

/// Parse an assistant message JSON `data` column into a RawEvent.
fn parse_message(
    id: String, session_id: String, data: &str,
    project_id: String, project_name: String,
    agent: String, code_additions: u64, code_deletions: u64,
    code_files: u64, code_diffs: u64,
    session_duration_ms: i64, session_time_created_ms: i64,
    session_title: String,
) -> Option<RawEvent> {
    let v: serde_json::Value = serde_json::from_str(data).ok()?;

    // Only assistant messages carry token counts.
    if v.get("role")?.as_str()? != "assistant" {
        return None;
    }

    let tokens = v.get("tokens")?;
    let tok_in = tokens.get("input").and_then(|n| n.as_f64()).unwrap_or(0.0);
    let tok_out = tokens.get("output").and_then(|n| n.as_f64()).unwrap_or(0.0);
    let tok_reasoning = tokens.get("reasoning").and_then(|n| n.as_f64()).unwrap_or(0.0);
    if tok_in <= 0.0 && tok_out <= 0.0 {
        return None; // skip empty/errored messages
    }

    let cache = tokens.get("cache");
    let cc = cache
        .and_then(|c| c.get("write"))
        .and_then(|n| n.as_f64())
        .unwrap_or(0.0);
    let cr = cache
        .and_then(|c| c.get("read"))
        .and_then(|n| n.as_f64())
        .unwrap_or(0.0);

    let model = v
        .get("modelID")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown")
        .to_string();

    let cost = v.get("cost").and_then(|c| c.as_f64()).unwrap_or(0.0);

    // Timestamp: use time.created (ms epoch), fall back to time.completed
    let ts_ms = v
        .get("time")
        .and_then(|t| t.get("created"))
        .and_then(|n| n.as_i64())
        .or_else(|| {
            v.get("time")
                .and_then(|t| t.get("completed"))
                .and_then(|n| n.as_i64())
        })
        .unwrap_or(0);

    Some(RawEvent {
        ts_ms,
        session: session_id,
        model,
        in_tok: tok_in,
        cc,
        cr,
        out_tok: tok_out,
        reasoning: tok_reasoning,
        mcp: Vec::new(),
        skills: Vec::new(),
        id,
        source: "opencode".to_string(),
        msg_count: 1,
        stored_cost: if cost > 0.0 { Some(cost) } else { None },
        project_id,
        project_name,
        agent,
        code_additions,
        code_deletions,
        code_files,
        code_diffs,
        session_duration_ms,
        session_time_created_ms,
        session_title,
    })
}

fn query_events() -> Result<Vec<RawEvent>, rusqlite::Error> {
    let path = opencode_db_path().ok_or_else(|| {
        rusqlite::Error::InvalidPath(PathBuf::from(
            "OpenCode database not found",
        ))
    })?;
    let conn = open_db_readonly(&path)?;

    // Read every assistant message that has token data, ordered by time.
    // JOIN with session and project to get project info + session-level metadata.
    let mut stmt = conn.prepare(
        "SELECT m.id, m.session_id, m.data, s.project_id,
                COALESCE(p.name, s.directory) as project_name,
                COALESCE(s.agent, '') as agent,
                COALESCE(s.summary_additions, 0),
                COALESCE(s.summary_deletions, 0),
                COALESCE(s.summary_files, 0),
                COALESCE(s.summary_diffs, 0),
                COALESCE(s.time_created, 0),
                COALESCE(s.time_updated, 0),
                COALESCE(s.title, '')
         FROM message m
         JOIN session s ON m.session_id = s.id
         LEFT JOIN project p ON s.project_id = p.id
          WHERE json_extract(m.data, '$.role') = 'assistant'
            AND (json_extract(m.data, '$.tokens.input') > 0
                 OR json_extract(m.data, '$.tokens.output') > 0)
         ORDER BY json_extract(m.data, '$.time.created') ASC",
    )?;

    let events: Vec<RawEvent> = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let session_id: String = row.get(1)?;
            let data: String = row.get(2)?;
            let project_id: String = row.get(3)?;
            let project_name: String = row.get(4)?;
            // Prefix OpenCode agents so they're distinguishable from Codex's
            // "Codex Desktop" in the merged agent stats ("OpenCode-build").
            let agent: String = row.get(5)?;
            let agent = if agent.is_empty() {
                agent
            } else {
                format!("OpenCode-{agent}")
            };
            let code_additions: u64 = row.get(6)?;
            let code_deletions: u64 = row.get(7)?;
            let code_files: u64 = row.get(8)?;
            let code_diffs: u64 = row.get(9)?;
            let time_created: i64 = row.get(10)?;
            let time_updated: i64 = row.get(11)?;
            let session_title: String = row.get(12)?;
            // COALESCE(p.name, s.directory) falls back to full paths like
            // "/Users/…/my-project". Extract just the last segment for display.
            let project_name = if project_name.starts_with('/') {
                project_name.rsplit('/').next().unwrap_or(&project_name).to_string()
            } else {
                project_name
            };
            let session_duration_ms = if time_created > 0 && time_updated > 0 {
                time_updated - time_created
            } else {
                0
            };
            let session_time_created_ms = time_created;
            Ok((id, session_id, data, project_id, project_name,
                agent, code_additions, code_deletions, code_files, code_diffs,
                session_duration_ms, session_time_created_ms, session_title))
        })?
        .filter_map(|r| r.ok())
        .filter_map(|(id, session_id, data, project_id, project_name,
                       agent, code_additions, code_deletions, code_files, code_diffs,
                       session_duration_ms, session_time_created_ms, session_title)| {
            parse_message(
                id, session_id, &data, project_id, project_name,
                agent, code_additions, code_deletions, code_files, code_diffs,
                session_duration_ms, session_time_created_ms, session_title,
            )
        })
        .collect();

    Ok(events)
}

/// Read tool call names from the `part` table, grouped by message_id.
/// Returns a map of message_id → list of tool names (with skill: prefix for skills).
fn query_tool_calls(user_cfg: &crate::config::UserConfig) -> HashMap<String, (Vec<String>, Vec<String>)> {
    let path = match opencode_db_path() {
        Some(p) => p,
        None => return HashMap::new(),
    };
    let conn = match open_db_readonly(&path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    let mut stmt = match conn.prepare(
        "SELECT p.message_id, p.data FROM part p WHERE json_extract(p.data, '$.type') = 'tool'"
    ) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };

    // Returns (mcp_servers, skills) for each message
    let mut map: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();
    let rows = stmt.query_map([], |row| {
        let msg_id: String = row.get(0)?;
        let data: String = row.get(1)?;
        Ok((msg_id, data))
    });

    if let Ok(rows) = rows {
        for row in rows.flatten() {
            let (msg_id, data) = row;
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
                if let Some(tool_name) = v.get("tool").and_then(|t| t.as_str()) {
                    let entry = map.entry(msg_id).or_default();
                    if tool_name == "skill" {
                        // Extract the actual skill name from state.input.name
                        let skill_name = v.get("state")
                            .and_then(|s| s.get("input"))
                            .and_then(|i| i.get("name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown");
                        if user_cfg.is_user_skill(skill_name) {
                            entry.1.push(skill_name.to_string());
                        }
                    } else if !is_builtin_tool(tool_name) {
                        // Check if it's an MCP tool: format is {server}_{toolname}
                        if let Some(server) = classify_mcp_tool(tool_name, user_cfg) {
                            entry.0.push(server);
                        }
                    }
                }
            }
        }
    }
    map
}

/// Built-in OpenCode tools — not MCP or Skill, so they are ignored for
/// MCP/skill tracking.
fn is_builtin_tool(name: &str) -> bool {
    matches!(
        name,
        "read" | "write" | "edit" | "bash" | "grep" | "glob"
            | "task" | "todowrite" | "question" | "webfetch"
            | "websearch_web_search_exa" | "globb"
    )
}

/// Classify a tool name as MCP: if its prefix (before the first `_`) matches
/// a known MCP server, return the server name. Otherwise return None.
/// MCP tool names have the format `{server}_{toolname}`, e.g.
/// `firecrawl_firecrawl_search` → server "firecrawl".
fn classify_mcp_tool(name: &str, cfg: &crate::config::UserConfig) -> Option<String> {
    let underscore = name.find('_')?;
    let server = &name[..underscore];
    if cfg.is_user_mcp(server) {
        Some(server.to_string())
    } else {
        None
    }
}

impl Store {
    pub fn load() -> Self {
        let mut events = query_events().unwrap_or_default();
        let cfg = crate::config::UserConfig::load();
        let tool_map = query_tool_calls(&cfg);
        for e in &mut events {
            if let Some((mcp_servers, skills)) = tool_map.get(&e.id) {
                e.mcp = mcp_servers.clone();
                e.skills = skills.clone();
            }
        }
        Store { events }
    }

    pub fn ingest(&mut self) -> bool {
        let mut fresh = query_events().unwrap_or_default();
        let cfg = crate::config::UserConfig::load();
        let tool_map = query_tool_calls(&cfg);
        for e in &mut fresh {
            if let Some((mcp_servers, skills)) = tool_map.get(&e.id) {
                e.mcp = mcp_servers.clone();
                e.skills = skills.clone();
            }
        }
        if fresh == self.events {
            return false;
        }
        self.events = fresh;
        true
    }

    pub fn prune_before(&mut self, cutoff_ms: i64) -> bool {
        let before = self.events.len();
        self.events.retain(|e| e.ts_ms >= cutoff_ms);
        self.events.len() != before
    }

    pub fn save(&self) {}
}
