//! DeepSeek account balance — GET https://api.deepseek.com/user/balance.
//!
//! Read-only, cached for 5 minutes (the panel refreshes every 30s; the balance
//! API shouldn't be hammered).
//!
//! The API key comes **only** from the key the user enters in the panel
//! (`set_deepseek_key`), stored in the app data dir (`deepseek_key`, 0600).
//! There is deliberately no discovery from environment variables or other
//! apps' configs — when OCScale is shared, each person sets their own key in
//! the UI (`get_deepseek_key_status` reports "missing" to show the prompt).
//! The key is never logged or printed.

use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const BALANCE_URL: &str = "https://api.deepseek.com/user/balance";
const CACHE_SECS: u64 = 5 * 60;

/// True for plausible DeepSeek API keys (`sk-` + at least 8 more chars).
pub fn validate_key(key: &str) -> bool {
    let k = key.trim();
    k.len() >= 10 && k.starts_with("sk-")
}

#[derive(Debug, Clone, Serialize)]
pub struct BalanceInfo {
    pub currency: String,
    #[serde(rename = "totalBalance")]
    pub total_balance: f64,
    #[serde(rename = "grantedBalance")]
    pub granted_balance: f64,
    #[serde(rename = "toppedUpBalance")]
    pub topped_up_balance: f64,
}

#[derive(Deserialize)]
struct RawBalance {
    is_available: bool,
    balance_infos: Vec<RawBalanceInfo>,
}

#[derive(Deserialize)]
struct RawBalanceInfo {
    currency: String,
    total_balance: String,
    granted_balance: String,
    topped_up_balance: String,
}

static CACHE: OnceLock<Mutex<Option<(Instant, Option<BalanceInfo>)>>> = OnceLock::new();

/// Cached balance; `None` when unavailable (no key / network / API error).
pub fn balance() -> Option<BalanceInfo> {
    let lock = CACHE.get_or_init(|| Mutex::new(None));
    let mut g = lock.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, cached)) = g.as_ref() {
        if at.elapsed() < Duration::from_secs(CACHE_SECS) {
            return cached.clone();
        }
    }
    let result = api_key().and_then(|k| fetch_balance(&k));
    *g = Some((Instant::now(), result.clone()));
    result
}

fn fetch_balance(key: &str) -> Option<BalanceInfo> {
    let raw = ureq::get(BALANCE_URL)
        .set("Authorization", &format!("Bearer {key}"))
        .timeout(Duration::from_secs(10))
        .call()
        .ok()?
        .into_json::<RawBalance>()
        .ok()?;
    if !raw.is_available {
        return None;
    }
    let b = raw.balance_infos.into_iter().next()?;
    Some(BalanceInfo {
        currency: b.currency,
        total_balance: b.total_balance.parse().ok()?,
        granted_balance: b.granted_balance.parse().ok()?,
        topped_up_balance: b.topped_up_balance.parse().ok()?,
    })
}

fn api_key() -> Option<String> {
    stored_key()
}

/// Whether any key source is configured (so the UI knows when to prompt).
pub fn key_configured() -> bool {
    api_key().is_some()
}

fn stored_key_path() -> Option<std::path::PathBuf> {
    let dir = dirs::data_dir()?.join("ocscale");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("deepseek_key"))
}

/// The user-entered key, if any (stored by `save_stored_key`).
fn stored_key() -> Option<String> {
    let p = stored_key_path()?;
    let s = std::fs::read_to_string(p).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Persist a user-entered key in the app data dir with 0600 permissions.
pub fn save_stored_key(key: &str) -> Result<(), String> {
    let k = key.trim();
    if !validate_key(k) {
        return Err("key must look like sk-…".to_string());
    }
    let p = stored_key_path().ok_or_else(|| "no data directory".to_string())?;
    std::fs::write(&p, k).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Remove the user-entered key.
pub fn clear_stored_key() {
    if let Some(p) = stored_key_path() {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sample_balance_response() {
        let json = r#"{
          "is_available": true,
          "balance_infos": [
            {
              "currency": "CNY",
              "total_balance": "110.00",
              "granted_balance": "10.00",
              "topped_up_balance": "100.00"
            }
          ]
        }"#;
        let raw: RawBalance = serde_json::from_str(json).unwrap();
        assert!(raw.is_available);
        let b = &raw.balance_infos[0];
        assert_eq!(b.currency, "CNY");
        assert_eq!(b.total_balance, "110.00");
        assert_eq!(b.topped_up_balance, "100.00");
    }

    #[test]
    fn key_validation() {
        assert!(validate_key("sk-0123456789abcdef"));
        assert!(validate_key("  sk-abcdefghij  "));
        assert!(!validate_key("sk-ab"));      // too short
        assert!(!validate_key("not-a-key"));  // no sk- prefix
        assert!(!validate_key(""));
        assert!(!validate_key("sk-123456")); // 9 chars total
    }
}
