//! DeepSeek account balance — GET https://api.deepseek.com/user/balance.
//!
//! Read-only, cached for 5 minutes (the panel refreshes every 30s; the balance
//! API shouldn't be hammered). API key resolution order:
//!   1. `DEEPSEEK_API_KEY` env var
//!   2. `~/.codex/config.toml` → `[model_providers.deepseek]` →
//!      `experimental_bearer_token` / `api_key` / `env_key`
//!   3. `~/.config/opencode/opencode.json` → `provider.deepseek.options.apiKey`
//!   4. A key the user entered in the OCScale panel, stored in the app data
//!      dir (`deepseek_key`, 0600). Auto-detected keys are never persisted;
//!      only an explicit user-entered key is saved (for sharing the app with
//!      people who don't use Codex/OpenCode configs). The key is never logged
//!      or printed.

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
    if let Ok(k) = std::env::var("DEEPSEEK_API_KEY") {
        if !k.is_empty() {
            return Some(k);
        }
    }
    key_from_codex_config()
        .or_else(key_from_opencode_config)
        .or_else(stored_key)
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

/// `~/.codex/config.toml` → `[model_providers.deepseek]` →
/// `experimental_bearer_token` / `api_key` / `env_key` (env_key is resolved
/// to the value of that environment variable).
fn key_from_codex_config() -> Option<String> {
    let path = dirs::home_dir()?.join(".codex/config.toml");
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_deepseek = false;
    let mut result: Option<String> = None;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            in_deepseek = t == "[model_providers.deepseek]";
            continue;
        }
        if !in_deepseek {
            continue;
        }
        if let Some(v) = parse_key_line(t, "experimental_bearer_token")
            .or_else(|| parse_key_line(t, "api_key"))
        {
            result = Some(v);
            break;
        }
        if let Some(env) = parse_key_line(t, "env_key") {
            result = std::env::var(env).ok().filter(|s| !s.is_empty());
            break;
        }
    }
    result
}

/// `~/.config/opencode/opencode.json` → `provider.deepseek.options.apiKey`.
fn key_from_opencode_config() -> Option<String> {
    let cfg_dir = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        std::path::PathBuf::from(xdg).join("opencode")
    } else {
        dirs::home_dir()?.join(".config/opencode")
    };
    let text = std::fs::read_to_string(cfg_dir.join("opencode.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.pointer("/provider/deepseek/options/apiKey")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

/// Parse `key = "value"` / `key = 'value'` (also tolerates a trailing inline
/// comment before the closing quote is not present — tokens contain no spaces).
fn parse_key_line(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let v = rest.strip_prefix('"').or_else(|| rest.strip_prefix('\''))?;
    let v = v.strip_suffix('"').or_else(|| v.strip_suffix('\''))?;
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
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
    fn parses_key_lines() {
        assert_eq!(
            parse_key_line("experimental_bearer_token = \"sk-abc123\"", "experimental_bearer_token"),
            Some("sk-abc123".to_string())
        );
        assert_eq!(
            parse_key_line("env_key = 'DEEPSEEK_API_KEY'", "env_key"),
            Some("DEEPSEEK_API_KEY".to_string())
        );
        assert_eq!(parse_key_line("base_url = \"https://x\"", "experimental_bearer_token"), None);
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
