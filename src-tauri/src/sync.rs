//! Multi-device sync (方案二: server-side aggregation).
//!
//! This device **only pushes**: on every background refresh it extracts the
//! RawEvents newer than a persisted watermark and POSTs them to the configured
//! `ocscale-server` (`POST /api/events`). The server stores them idempotently
//! (`UNIQUE(source, id)` + `INSERT OR IGNORE`) and serves the merged Dashboard.
//! Retries are safe — a re-sent event is simply ignored server-side.
//!
//! Config lives in `data_dir/ocscale/sync.json` (0600, contains the token) and
//! a stable per-device id in `data_dir/ocscale/device_id`; the push watermark
//! and last result live in `data_dir/ocscale/sync_state.json`. Nothing is ever
//! logged or printed, including the token.

use ocscale_core::store::RawEvent;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub enabled: bool,
    pub url: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncState {
    #[serde(rename = "watermarkMs")]
    watermark_ms: i64,
    #[serde(rename = "lastSyncMs")]
    last_sync_ms: Option<i64>,
    #[serde(rename = "lastError")]
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncStatus {
    pub enabled: bool,
    pub url: String,
    #[serde(rename = "hasToken")]
    pub has_token: bool,
    #[serde(rename = "deviceId")]
    pub device_id: String,
    #[serde(rename = "lastSyncMs")]
    pub last_sync_ms: Option<i64>,
    #[serde(rename = "lastError")]
    pub last_error: Option<String>,
    #[serde(rename = "pendingEvents")]
    pub pending_events: u64,
}

// Serializes pushes: the 30s refresh loop and a manual "sync now" must never
// overlap (each has its own watermark read/write).
static SYNC_LOCK: Mutex<()> = Mutex::new(());

fn data_dir() -> Option<PathBuf> {
    let dir = dirs::data_dir()?.join("ocscale");
    let _ = fs::create_dir_all(&dir);
    Some(dir)
}

fn config_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("sync.json"))
}

fn device_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("device_id"))
}

fn state_path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("sync_state.json"))
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            enabled: false,
            url: String::new(),
            token: String::new(),
        }
    }
}

fn load_config() -> SyncConfig {
    let Some(p) = config_path() else {
        return SyncConfig::default();
    };
    fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn load_state() -> SyncState {
    let Some(p) = state_path() else {
        return SyncState {
            watermark_ms: 0,
            last_sync_ms: None,
            last_error: None,
        };
    };
    fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(SyncState {
            watermark_ms: 0,
            last_sync_ms: None,
            last_error: None,
        })
}

fn save_state(state: &SyncState) {
    let Some(p) = state_path() else { return };
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = fs::write(p, json);
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Stable per-install device id (persisted; never derived from anything
/// user-identifiable). Lets the server tell devices apart for dedup/debug.
fn device_id() -> String {
    if let Some(p) = device_path() {
        if let Ok(s) = fs::read_to_string(&p) {
            let s = s.trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
        // 32 random hex chars.
        let mut id = String::with_capacity(32);
        use std::time::Instant;
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
            ^ Instant::now().elapsed().as_nanos();
        let mut x = seed;
        for _ in 0..32 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            id.push("0123456789abcdef".as_bytes()[((x >> 60) & 0xf) as usize] as char);
        }
        let _ = fs::write(&p, &id);
        return id;
    }
    "unknown".to_string()
}

/// Push events newer than the watermark to the server. Idempotent by design;
/// the watermark only advances on success, so a dropped network request retries
/// on the next refresh. Runs off the async runtime (blocking HTTP + BUILD_LOCK).
pub fn sync_now() {
    let _guard = SYNC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cfg = load_config();
    let url = cfg.url.trim().trim_end_matches('/').to_string();
    if !cfg.enabled || url.is_empty() || cfg.token.trim().is_empty() {
        return;
    }

    let mut state = load_state();
    let events = ocscale_core::parser::raw_events();
    let pending: Vec<RawEvent> = events
        .into_iter()
        .filter(|e| e.ts_ms > state.watermark_ms)
        .collect();
    if pending.is_empty() {
        return;
    }

    let body = serde_json::json!({
        "device_id": device_id(),
        "events": pending,
    });
    let result = ureq::post(&format!("{url}/api/events"))
        .timeout(Duration::from_secs(20))
        .set("Authorization", &format!("Bearer {}", cfg.token.trim()))
        .send_json(body);

    match result {
        Ok(resp) if resp.status() == 200 => {
            state.watermark_ms = pending
                .iter()
                .map(|e| e.ts_ms)
                .max()
                .unwrap_or(state.watermark_ms);
            state.last_sync_ms = Some(now_ms());
            state.last_error = None;
            save_state(&state);
        }
        Ok(resp) => {
            state.last_error = Some(format!("server returned HTTP {}", resp.status()));
            save_state(&state);
        }
        Err(e) => {
            state.last_error = Some(format!("{e}"));
            save_state(&state);
        }
    }
}

/// Status for the settings UI. `pending_events` reflects the last refresh's
/// view and is only counted while sync is enabled.
pub fn status() -> SyncStatus {
    let cfg = load_config();
    let state = load_state();
    let pending = if cfg.enabled && !cfg.url.trim().is_empty() && !cfg.token.trim().is_empty() {
        ocscale_core::parser::raw_events()
            .into_iter()
            .filter(|e| e.ts_ms > state.watermark_ms)
            .count() as u64
    } else {
        0
    };
    SyncStatus {
        enabled: cfg.enabled,
        url: cfg.url,
        has_token: !cfg.token.trim().is_empty(),
        device_id: device_id(),
        last_sync_ms: state.last_sync_ms,
        last_error: state.last_error,
        pending_events: pending,
    }
}

/// Persist the sync settings (token stored 0600, like the DeepSeek key).
pub fn save_config(url: String, token: String, enabled: bool) -> Result<(), String> {
    let cfg = SyncConfig {
        url: url.trim().trim_end_matches('/').to_string(),
        token: token.trim().to_string(),
        enabled,
    };
    let p = config_path().ok_or_else(|| "no data directory".to_string())?;
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    fs::write(&p, json).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrips_through_json() {
        let cfg = SyncConfig {
            enabled: true,
            url: "https://example.com".to_string(),
            token: "secret".to_string(),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: SyncConfig = serde_json::from_str(&json).unwrap();
        assert!(back.enabled);
        assert_eq!(back.url, "https://example.com");
        assert_eq!(back.token, "secret");
    }

    #[test]
    fn default_config_is_disabled() {
        let cfg = SyncConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.url.is_empty());
        assert!(cfg.token.is_empty());
    }
}
