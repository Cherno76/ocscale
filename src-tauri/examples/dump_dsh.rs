//! Dump a Dashboard JSON built from DeepSeek Harness session logs
//! (`~/.dsh/sessions/**`, or `$DSH_HOME/sessions/**`).
//!
//! Usage: `cargo run --example dump_dsh > /tmp/dsh-dashboard.json`
fn main() {
    println!("{}", ocscale_lib::dsh_dashboard_json());
}
