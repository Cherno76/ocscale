// Incremental event store reading from OpenCode's SQLite database.
//
// Replaces the original JSONL-watcher for ~/.claude/projects with a direct
// SQLite query against OpenCode's session table. The RawEvent struct stays
// identical so parser.rs, pricing.rs and the entire frontend work unchanged.
//
// OpenCode already stores per-session token and cost data, so we just poll
// the DB every 30s — no filesystem watcher, no incremental ingest, no cache.
use rusqlite::Connection;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct RawEvent {
    pub ts_ms: i64,
    pub session: String,
    pub model: String, // normalized model id (e.g. "deepseek-v4-flash")
    pub in_tok: f64,
    pub cc: f64,  // cache creation (tokens_cache_write)
    pub cr: f64,  // cache read (tokens_cache_read)
    pub out_tok: f64,
    pub mcp: Vec<String>,    // (not yet populated — future: event table)
    pub skills: Vec<String>, // (not yet populated — future: event table)
    pub id: String,          // session id (also used as dedup key)
    pub source: String,      // always "opencode"
    /// Cost pre-calculated by OpenCode. Falls back when the pricing module
    /// doesn't recognise the model (e.g. custom DeepSeek variants).
    pub stored_cost: Option<f64>,
}

pub struct Store {
    pub events: Vec<RawEvent>,
}

/// Locate the OpenCode SQLite database. Tries the XDG-style path first
/// (Linux/portable), then the macOS Application Support path.
fn opencode_db_path() -> Option<PathBuf> {
    // XDG data home (Linux, or macOS when configured by OpenCode)
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        let p = PathBuf::from(xdg).join("opencode/opencode.db");
        if p.exists() {
            return Some(p);
        }
    }
    // Default XDG: ~/.local/share/opencode/opencode.db (macOS too)
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".local/share/opencode/opencode.db");
        if p.exists() {
            return Some(p);
        }
    }
    // macOS fallback: ~/Library/Application Support/opencode/opencode.db
    if let Some(d) = dirs::data_dir() {
        let p = d.join("opencode/opencode.db");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Parse the `model` JSON column from the session table. It's a JSON object
/// like `{"id":"deepseek-v4-flash","providerID":"deepseek","variant":"low"}`.
/// Returns just the `id` field, or the raw string if parsing fails.
fn parse_model(raw: &str) -> String {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str().map(String::from)))
        .unwrap_or_else(|| raw.to_string())
}

fn query_events() -> Result<Vec<RawEvent>, rusqlite::Error> {
    let path = opencode_db_path().ok_or_else(|| {
        rusqlite::Error::InvalidPath(
            PathBuf::from(
                "OpenCode database not found — have you launched OpenCode yet?",
            ),
        )
    })?;
    let conn = Connection::open(&path)?;

    // Build a version-independent query: some schema versions had cost/token
    // columns added later, but they should all be present in current DBs.
    let mut stmt = conn.prepare(
        "SELECT id, time_updated, model,
                tokens_input, tokens_output,
                tokens_cache_read, tokens_cache_write,
                cost
         FROM session
         WHERE (tokens_input > 0 OR tokens_output > 0)
           AND model IS NOT NULL AND model != ''
         ORDER BY time_updated ASC",
    )?;

    let events = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let ts_ms: i64 = row.get(1)?;
        let model_raw: String = row.get(2)?;
        let tok_in: i64 = row.get(3)?;
        let tok_out: i64 = row.get(4)?;
        let cache_read: i64 = row.get(5)?;
        let cache_write: i64 = row.get(6)?;
        let cost: f64 = row.get(7)?;

        Ok(RawEvent {
            ts_ms,
            session: id.clone(),
            model: parse_model(&model_raw),
            in_tok: tok_in as f64,
            cc: cache_write as f64,
            cr: cache_read as f64,
            out_tok: tok_out as f64,
            mcp: Vec::new(),
            skills: Vec::new(),
            id,
            source: "opencode".to_string(),
            stored_cost: if cost > 0.0 { Some(cost) } else { None },
        })
    })?;

    events.collect()
}

impl Store {
    /// Full reload from the OpenCode database. This is the only public
    /// load path — no incremental manifest, no disk cache.
    pub fn load() -> Self {
        let events = query_events().unwrap_or_default();
        Store { events }
    }

    /// Re-query the database and return true if events changed (so the
    /// caller knows to re-aggregate). Cheap enough to call every 30s.
    pub fn ingest(&mut self) -> bool {
        let fresh = query_events().unwrap_or_default();
        if fresh == self.events {
            return false;
        }
        self.events = fresh;
        true
    }

    /// Drop events older than `cutoff_ms` to bound the aggregation window.
    pub fn prune_before(&mut self, cutoff_ms: i64) -> bool {
        let before = self.events.len();
        self.events.retain(|e| e.ts_ms >= cutoff_ms);
        self.events.len() != before
    }

    /// No-op: the DB is always current, no cache to persist.
    pub fn save(&self) {}
}
