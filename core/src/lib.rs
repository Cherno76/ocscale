//! OCScale aggregation core.
//!
//! Everything from raw sources (OpenCode SQLite, Codex transcripts) to the
//! serializable `Dashboard`. Shared by the Tauri app (`ocscale`) and the
//! multi-device sync server (`ocscale-server`), which ingests RawEvents from
//! many machines and aggregates them with the exact same parser + pricing.

pub mod config;
pub mod model;
pub mod parser;
pub mod pricing;
pub mod store;
pub mod store_codex;
