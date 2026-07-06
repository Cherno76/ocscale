// Incremental event store reading from OpenCode's SQLite database.
//
// Reads individual assistant messages from the `message` table, each with its
// own timestamp and token counts. This gives accurate hourly distribution in
// the daily chart, unlike per-session aggregates which lump all tokens into
// one time bucket.
use rusqlite::Connection;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct RawEvent {
    pub ts_ms: i64,
    pub session: String,
    pub project: String, // project display name (worktree basename, or "global")
    pub model: String,
    pub in_tok: f64,
    pub cc: f64,  // cache creation (tokens_cache_write)
    pub cr: f64,  // cache read (tokens_cache_read)
    pub out_tok: f64,
    pub mcp: Vec<String>,    // (not yet populated)
    pub skills: Vec<String>, // (not yet populated)
    pub id: String,          // message id
    pub source: String,      // always "opencode"
    /// Always 1 for per-message events (each message = 1 request).
    pub msg_count: u64,
    /// Per-message cost from OpenCode. Falls back when pricing module
    /// doesn't recognise the model.
    pub stored_cost: Option<f64>,
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

/// Parse an assistant message JSON `data` column into a RawEvent.
fn parse_message(id: String, session_id: String, project: String, data: &str) -> Option<RawEvent> {
    let v: serde_json::Value = serde_json::from_str(data).ok()?;

    // Only assistant messages carry token counts.
    if v.get("role")?.as_str()? != "assistant" {
        return None;
    }

    let tokens = v.get("tokens")?;
    let tok_in = tokens.get("input").and_then(|n| n.as_f64()).unwrap_or(0.0);
    let tok_out = tokens.get("output").and_then(|n| n.as_f64()).unwrap_or(0.0);
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
        project,
        model,
        in_tok: tok_in,
        cc,
        cr,
        out_tok: tok_out,
        mcp: Vec::new(),
        skills: Vec::new(),
        id,
        source: "opencode".to_string(),
        msg_count: 1,
        stored_cost: if cost > 0.0 { Some(cost) } else { None },
    })
}

fn query_events() -> Result<Vec<RawEvent>, rusqlite::Error> {
    let path = opencode_db_path().ok_or_else(|| {
        rusqlite::Error::InvalidPath(PathBuf::from(
            "OpenCode database not found",
        ))
    })?;
    let conn = Connection::open(&path)?;

    // Read every assistant message that has token data, ordered by time.
    // Match each message against known projects by checking if the CWD is
    // inside (or equal to) a project's worktree. For monorepo root messages
    // that don't match any specific project, the CWD basename is used as the
    // project name (e.g. "TUI-Project"). Those are genuinely root-level
    // messages — switching to Month view will show deeper project breakdown.
    let mut stmt = conn.prepare(
        "SELECT m.id, m.session_id, m.data,
                COALESCE(
                  (SELECT p.worktree FROM project p
                   WHERE json_extract(m.data, '$.path.cwd') LIKE p.worktree || '%'
                    AND p.worktree != '/'
                   ORDER BY LENGTH(p.worktree) DESC LIMIT 1),
                  json_extract(m.data, '$.path.cwd'),
                  s.directory,
                  '/'
                ) as project_path
         FROM message m
         JOIN session s ON s.id = m.session_id
         WHERE json_extract(m.data, '$.role') = 'assistant'
           AND json_extract(m.data, '$.tokens.input') > 0
         ORDER BY json_extract(m.data, '$.time.created') ASC",
    )?;

    let events: Vec<RawEvent> = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let session_id: String = row.get(1)?;
            let data: String = row.get(2)?;
            let project_path: String = row.get(3)?;
            // Extract basename: "/Users/cherno/TUI-Project/tokenscope" → "tokenscope"
            let project = project_path
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("global")
                .to_string();
            Ok((id, session_id, project, data))
        })?
        .filter_map(|r| r.ok())
        .filter_map(|(id, session_id, project, data)| {
            parse_message(id, session_id, project, &data)
        })
        .collect();

    Ok(events)
}

impl Store {
    pub fn load() -> Self {
        let events = query_events().unwrap_or_default();
        Store { events }
    }

    pub fn ingest(&mut self) -> bool {
        let fresh = query_events().unwrap_or_default();
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
