//! OCScale aggregation core.
//!
//! Everything from raw sources (OpenCode SQLite, Codex transcripts) to the
//! serializable `Dashboard`, used by the Tauri app (`ocscale`).

pub mod config;
pub mod model;
pub mod parser;
pub mod pricing;
pub mod store;
pub mod store_codex;
pub mod store_dsh;
