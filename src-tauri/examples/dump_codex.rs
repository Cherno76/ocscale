//! Feasibility prototype: dump a Dashboard JSON built from Codex session
//! transcripts (`~/.codex/sessions/**` + `~/.codex/archived_sessions/`).
//!
//! Usage: `cargo run --example dump_codex > /tmp/codex-dashboard.json`
fn main() {
    println!("{}", ocscale_lib::codex_dashboard_json());
}
