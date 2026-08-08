//! OCScale multi-device sync server (方案二: server-side aggregation).
//!
//! Each device pushes its incremental RawEvents (idempotent: `UNIQUE(source,
//! id)` + `INSERT OR IGNORE`); this server stores them in SQLite and serves a
//! merged `Dashboard` built by the exact same `ocscale-core` parser/pricing
//! the app uses, so prices, timezones and day-boundary logic stay consistent.
//!
//! ```text
//! OCSCALE_TOKEN=<shared secret> \
//! OCSCALE_ADDR=127.0.0.1:8787 \
//! OCSCALE_DB=/var/lib/ocscale/server.db \
//! cargo run -p ocscale-server
//! ```
//!
//! Expose it publicly through a TLS reverse proxy (Caddy auto-HTTPS is the
//! simplest) and point each device's app at `https://your.host/api/...`.

use axum::{
    extract::{DefaultBodyLimit, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use ocscale_core::parser::build_dashboard_from_mode;
use ocscale_core::store::RawEvent;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    started_at_ms: i64,
}

#[derive(Deserialize)]
struct IngestRequest {
    device_id: String,
    events: Vec<RawEvent>,
}

#[derive(Serialize)]
struct IngestResponse {
    accepted: usize,
    duplicates: usize,
}

#[derive(Serialize)]
struct Health {
    ok: bool,
    events: u64,
    uptime_s: u64,
}

#[derive(Serialize)]
struct ApiRoot {
    name: String,
    endpoints: Vec<String>,
}

#[derive(Deserialize)]
struct DashParams {
    #[serde(default)]
    utc_day: Option<String>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn init_db(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS events (
            source               TEXT NOT NULL,
            id                   TEXT NOT NULL,
            ts_ms                INTEGER NOT NULL,
            session              TEXT NOT NULL,
            model                TEXT NOT NULL,
            in_tok               REAL NOT NULL,
            cc                   REAL NOT NULL,
            cr                   REAL NOT NULL,
            out_tok              REAL NOT NULL,
            reasoning            REAL NOT NULL,
            mcp                  TEXT NOT NULL DEFAULT '[]',
            skills               TEXT NOT NULL DEFAULT '[]',
            msg_count            INTEGER NOT NULL DEFAULT 1,
            stored_cost          REAL,
            project_id           TEXT NOT NULL,
            project_name         TEXT NOT NULL,
            agent                TEXT NOT NULL,
            code_additions       INTEGER NOT NULL,
            code_deletions       INTEGER NOT NULL,
            code_files           INTEGER NOT NULL,
            code_diffs           INTEGER NOT NULL,
            session_duration_ms  INTEGER NOT NULL,
            session_time_created_ms INTEGER NOT NULL,
            session_title        TEXT NOT NULL,
            device_id            TEXT NOT NULL,
            received_at          INTEGER NOT NULL,
            PRIMARY KEY (source, id)
        );
        CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts_ms);
        ",
    )
}

/// Idempotent batch insert. Returns (accepted, duplicates).
fn insert_events(conn: &Connection, device_id: &str, events: &[RawEvent]) -> (usize, usize) {
    let received = now_ms();
    let mut accepted = 0usize;
    let mut duplicates = 0usize;
    let tx = conn
        .unchecked_transaction()
        .expect("failed to start events transaction");
    {
        let mut stmt = tx
            .prepare(
                "INSERT OR IGNORE INTO events (
                    source, id, ts_ms, session, model, in_tok, cc, cr, out_tok,
                    reasoning, mcp, skills, msg_count, stored_cost, project_id,
                    project_name, agent, code_additions, code_deletions,
                    code_files, code_diffs, session_duration_ms,
                    session_time_created_ms, session_title, device_id, received_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                    ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26
                )",
            )
            .expect("failed to prepare events insert");
        for e in events {
            let mcp = serde_json::to_string(&e.mcp).unwrap_or_else(|_| "[]".to_string());
            let skills = serde_json::to_string(&e.skills).unwrap_or_else(|_| "[]".to_string());
            let n = stmt
                .execute(params![
                    e.source,
                    e.id,
                    e.ts_ms,
                    e.session,
                    e.model,
                    e.in_tok,
                    e.cc,
                    e.cr,
                    e.out_tok,
                    e.reasoning,
                    mcp,
                    skills,
                    e.msg_count,
                    e.stored_cost,
                    e.project_id,
                    e.project_name,
                    e.agent,
                    e.code_additions,
                    e.code_deletions,
                    e.code_files,
                    e.code_diffs,
                    e.session_duration_ms,
                    e.session_time_created_ms,
                    e.session_title,
                    device_id,
                    received,
                ])
                .expect("failed to insert event");
            if n > 0 {
                accepted += 1;
            } else {
                duplicates += 1;
            }
        }
    }
    tx.commit().expect("failed to commit events transaction");
    (accepted, duplicates)
}

fn load_events(conn: &Connection) -> Vec<RawEvent> {
    let mut stmt = conn
        .prepare(
            "SELECT source, id, ts_ms, session, model, in_tok, cc, cr, out_tok,
                    reasoning, mcp, skills, msg_count, stored_cost, project_id,
                    project_name, agent, code_additions, code_deletions,
                    code_files, code_diffs, session_duration_ms,
                    session_time_created_ms, session_title
             FROM events",
        )
        .expect("failed to prepare events select");
    let rows = stmt
        .query_map([], |row| {
            let mcp_json: String = row.get(10)?;
            let skills_json: String = row.get(11)?;
            Ok(RawEvent {
                source: row.get(0)?,
                id: row.get(1)?,
                ts_ms: row.get(2)?,
                session: row.get(3)?,
                model: row.get(4)?,
                in_tok: row.get(5)?,
                cc: row.get(6)?,
                cr: row.get(7)?,
                out_tok: row.get(8)?,
                reasoning: row.get(9)?,
                mcp: serde_json::from_str(&mcp_json).unwrap_or_default(),
                skills: serde_json::from_str(&skills_json).unwrap_or_default(),
                msg_count: row.get(12)?,
                stored_cost: row.get(13)?,
                project_id: row.get(14)?,
                project_name: row.get(15)?,
                agent: row.get(16)?,
                code_additions: row.get(17)?,
                code_deletions: row.get(18)?,
                code_files: row.get(19)?,
                code_diffs: row.get(20)?,
                session_duration_ms: row.get(21)?,
                session_time_created_ms: row.get(22)?,
                session_title: row.get(23)?,
            })
        })
        .expect("failed to query events");
    rows.filter_map(|r| r.ok()).collect()
}

fn event_count(conn: &Connection) -> u64 {
    conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap_or(0)
}

fn token_ok(headers: &HeaderMap) -> bool {
    let expected = std::env::var("OCSCALE_TOKEN").unwrap_or_default();
    if expected.is_empty() {
        return true; // auth disabled (warned at startup)
    }
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t == expected)
        .unwrap_or(false)
}

async fn ingest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<IngestRequest>,
) -> impl IntoResponse {
    if !token_ok(&headers) {
        return (StatusCode::UNAUTHORIZED, Json(IngestResponse { accepted: 0, duplicates: 0 })).into_response();
    }
    let device = if req.device_id.trim().is_empty() {
        "unknown"
    } else {
        req.device_id.trim()
    };
    let conn = state.db.lock().unwrap();
    let (accepted, duplicates) = insert_events(&conn, device, &req.events);
    (
        StatusCode::OK,
        Json(IngestResponse { accepted, duplicates }),
    )
        .into_response()
}

async fn dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<DashParams>,
) -> impl IntoResponse {
    if !token_ok(&headers) {
        return (StatusCode::UNAUTHORIZED, Json("unauthorized")).into_response();
    }
    let utc_day = params.utc_day.as_deref() == Some("true");
    let events = {
        let conn = state.db.lock().unwrap();
        load_events(&conn)
    };
    let mut dash = build_dashboard_from_mode(&events, utc_day);

    // The per-period "servers"/"skills" metrics are *installed* counts in the
    // app (read from each machine's local config). A server aggregating many
    // devices has no single config, so approximate with the distinct servers /
    // skills seen across the merged event stream.
    let mut servers: HashSet<&str> = HashSet::new();
    let mut skills: HashSet<&str> = HashSet::new();
    for e in &events {
        servers.extend(e.mcp.iter().map(String::as_str));
        skills.extend(
            e.skills
                .iter()
                .map(|s| s.rsplit(':').next().unwrap_or(s)),
        );
    }
    let (ns, nk) = (servers.len() as u64, skills.len() as u64);
    dash.day.metrics.servers = ns;
    dash.day.metrics.skills = nk;
    dash.week.metrics.servers = ns;
    dash.week.metrics.skills = nk;
    dash.month.metrics.servers = ns;
    dash.month.metrics.skills = nk;

    Json(dash).into_response()
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.db.lock().unwrap();
    Json(Health {
        ok: true,
        events: event_count(&conn),
        uptime_s: ((now_ms() - state.started_at_ms) / 1000).max(0) as u64,
    })
}

async fn root() -> impl IntoResponse {
    Json(ApiRoot {
        name: "ocscale-server".to_string(),
        endpoints: vec![
            "POST /api/events".to_string(),
            "GET /api/dashboard?utc_day=true|false".to_string(),
            "GET /api/health".to_string(),
        ],
    })
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/api/events", post(ingest))
        .route("/api/dashboard", get(dashboard))
        .route("/api/health", get(health))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

#[tokio::main]
async fn main() {
    let token = std::env::var("OCSCALE_TOKEN").unwrap_or_default();
    if token.is_empty() {
        eprintln!("[ocscale-server] warning: OCSCALE_TOKEN is not set — authentication is DISABLED");
    } else {
        eprintln!("[ocscale-server] token authentication enabled");
    }

    let db_path = std::env::var("OCSCALE_DB").unwrap_or_else(|_| "ocscale-server.db".to_string());
    let conn = Connection::open(&db_path).expect("failed to open SQLite database");
    init_db(&conn).expect("failed to initialise SQLite schema");
    eprintln!("[ocscale-server] events database: {db_path}");

    // Warm the price table off the request path (same as the app: the fetch can
    // block ~20s on a cold/stale cache; `build_dashboard_from_mode` only ever
    // sees the memoized copy).
    std::thread::spawn(ocscale_core::pricing::Pricing::reload_shared);

    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
        started_at_ms: now_ms(),
    };
    let addr: SocketAddr = std::env::var("OCSCALE_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8787".to_string())
        .parse()
        .expect("OCSCALE_ADDR must be host:port");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind listener");
    eprintln!("[ocscale-server] listening on http://{addr}");
    axum::serve(listener, app(state)).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_event(id: &str, ts_ms: i64) -> RawEvent {
        RawEvent {
            ts_ms,
            session: "sess-a".to_string(),
            model: "deepseek-v4-flash".to_string(),
            in_tok: 100.0,
            cc: 0.0,
            cr: 50.0,
            out_tok: 25.0,
            reasoning: 0.0,
            mcp: vec!["node_repl".to_string()],
            skills: vec!["visualize".to_string()],
            id: id.to_string(),
            source: "opencode".to_string(),
            msg_count: 1,
            stored_cost: None,
            project_id: "p1".to_string(),
            project_name: "demo".to_string(),
            agent: "build".to_string(),
            code_additions: 10,
            code_deletions: 2,
            code_files: 1,
            code_diffs: 1,
            session_duration_ms: 60000,
            session_time_created_ms: ts_ms - 1000,
            session_title: "fix bug".to_string(),
        }
    }

    #[test]
    fn insert_is_idempotent_on_source_id() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let batch = vec![sample_event("msg-1", 1000), sample_event("msg-2", 2000)];
        let (a1, d1) = insert_events(&conn, "dev-a", &batch);
        assert_eq!((a1, d1), (2, 0));
        // Re-send the same events (retry after a network blip): no double count.
        let (a2, d2) = insert_events(&conn, "dev-a", &batch);
        assert_eq!((a2, d2), (0, 2));
        // A different device's duplicate of the same message id is also ignored.
        let (a3, d3) = insert_events(&conn, "dev-b", &batch);
        assert_eq!((a3, d3), (0, 2));
        assert_eq!(event_count(&conn), 2);
    }

    #[test]
    fn events_roundtrip_preserves_fields() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let e = sample_event("msg-1", 1000);
        insert_events(&conn, "dev-a", &[e.clone()]);
        let loaded = load_events(&conn);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], e);
    }
}
