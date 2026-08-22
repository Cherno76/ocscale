// Token pricing. Primary source: models.dev (bare model names, matches Claude
// CLI logs). Fallback: LiteLLM. Final layer: an external, user-editable
// `pricing.json` (see `read_external_pricing`) that carries the official
// DeepSeek peak/off-peak rates plus a small per-token-USD backstop. Nothing is
// hardcoded here anymore — price changes only require editing that file.
//
// Matching is layered: exact id → normalized id (strip provider path prefix +
// unify the ".'↔'p" version separator, e.g. "glm-5.1" ⇄ "glm-5p1").
use chrono::NaiveDate;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, SystemTime};

// Process-wide memoized price table. Loaded once off the main thread (see
// reload_shared) and refreshed every 24h, so build_dashboard — which holds
// BUILD_LOCK — only ever does a cheap Arc clone, never JSON parsing or network.
static PRICING: OnceLock<RwLock<Arc<Pricing>>> = OnceLock::new();

const MODELSDEV_URL: &str = "https://models.dev/api.json";
const LITELLM_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60); // 24h
// Bundled LiteLLM price table snapshot — offline fallback so a first launch
// with no network (and no prior cache) still prices the common third-party
// models. Live sources, when reachable, are ingested first and win.
const LITELLM_SNAPSHOT: &str = include_str!("../snapshots/litellm.json");
// Bundled copy of the user-editable pricing file. Copied to
// `data_dir/ocscale/pricing.json` on first run, and used as the offline
// fallback before the background loader has run (and whenever the external
// file is missing or malformed).
const PRICING_SNAPSHOT: &str = include_str!("../snapshots/pricing.json");
// Fixed CNY→USD conversion used by the UI (¥7.2/$1); the DeepSeek rates in
// `pricing.json` are stored as CNY per 1M tokens and converted here so the
// panel's `cost × 7.2` shows the exact CNY figure.
const CNY_PER_USD: f64 = 7.2;

#[derive(Clone, Default)]
pub struct ModelPrice {
    pub input: f64,        // per-token USD
    pub output: f64,       // per-token USD
    pub cache_create: f64, // per-token USD
    pub cache_read: f64,   // per-token USD
}

impl ModelPrice {
    fn is_zero(&self) -> bool {
        self.input == 0.0 && self.output == 0.0 && self.cache_create == 0.0 && self.cache_read == 0.0
    }
}

/// A DeepSeek model's time-of-day split: one rate for the Beijing peak window
/// (09:00–12:00 and 14:00–18:00) and one for the rest of the day.
#[derive(Clone)]
struct PeakPrice {
    peak: ModelPrice,
    offpeak: ModelPrice,
}

/// Beijing-time peak/off-peak schedule parsed from the external pricing file.
/// Pure arithmetic on the UTC+8 shift, so it never needs a timezone database
/// and handles pre-epoch timestamps via `div_euclid`/`rem_euclid`.
#[derive(Clone)]
struct PeakSchedule {
    tz_offset_secs: i64,
    windows: Vec<(i64, i64)>,
    /// When true, weekends are billed at the off-peak rate all day.
    weekend_offpeak: bool,
    /// Epoch day (Beijing-shifted) on/after which the weekend rule applies.
    /// `None` = apply the rule from the beginning of time.
    weekend_offpeak_from_day: Option<i64>,
}

impl Default for PeakSchedule {
    fn default() -> Self {
        PeakSchedule {
            tz_offset_secs: 8 * 3600,
            windows: vec![(9, 12), (14, 18)],
            weekend_offpeak: false,
            weekend_offpeak_from_day: None,
        }
    }
}

impl PeakSchedule {
    /// Whether an epoch-ms timestamp lands in a Beijing peak window, taking the
    /// weekend off-peak rule into account (weekends are off-peak all day once
    /// `weekend_offpeak_from_day` is reached). Beijing = UTC+8, no DST.
    fn is_peak(&self, ts_ms: i64) -> bool {
        let secs = ts_ms.div_euclid(1000) + self.tz_offset_secs;
        let day = secs.div_euclid(86400);
        let hour = secs.rem_euclid(86400) / 3600;
        if self.weekend_offpeak {
            // Epoch day 0 (1970-01-01) was a Thursday; num_days_from_sunday
            // says Thu=4, so weekday (0=Sun..6=Sat) = (day + 4) mod 7.
            let weekday = (day + 4).rem_euclid(7);
            let rule_active = self.weekend_offpeak_from_day.map_or(true, |from| day >= from);
            if rule_active && (weekday == 0 || weekday == 6) {
                return false;
            }
        }
        self.windows.iter().any(|&(s, e)| (s..e).contains(&hour))
    }
}

/// Epoch day number (in Beijing-shifted seconds) of a `YYYY-MM-DD` date, i.e.
/// the same `day` value `PeakSchedule::is_peak` derives from a timestamp.
fn beijing_day_of_date(s: &str) -> Option<i64> {
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    let utc_ts = d.and_hms_opt(0, 0, 0)?.and_utc().timestamp();
    Some((utc_ts + 8 * 3600).div_euclid(86400))
}

/// External pricing file schema (`core/snapshots/pricing.json` / the
/// user-editable copy in `data_dir/ocscale/pricing.json`).
#[derive(Deserialize)]
struct PricingFile {
    #[serde(default)]
    timezone_offset_hours: i64,
    #[serde(default)]
    peak_windows: Vec<[i64; 2]>,
    #[serde(default)]
    weekend_offpeak: bool,
    #[serde(default)]
    weekend_offpeak_from: Option<String>,
    /// DeepSeek-style models: CNY per 1M tokens, split by Beijing peak window.
    #[serde(default)]
    models: HashMap<String, WindowPrices>,
    /// Flat per-token-USD backstop models (no peak/off-peak split).
    #[serde(default)]
    flat_usd: HashMap<String, FlatUsd>,
}

#[derive(Deserialize, Clone)]
struct WindowPrices {
    #[serde(default)]
    offpeak: CnyWindow,
    /// Omitted → same as `offpeak` (no time-of-day split).
    #[serde(default)]
    peak: Option<CnyWindow>,
}

#[derive(Deserialize, Clone, Default)]
struct CnyWindow {
    /// CNY per 1M tokens, cache-miss input (cache-write bills at this rate).
    #[serde(default)]
    input_miss: f64,
    /// CNY per 1M tokens, cache-hit input.
    #[serde(default)]
    input_hit: f64,
    /// CNY per 1M tokens, output (reasoning bills at this rate too).
    #[serde(default)]
    output: f64,
}

#[derive(Deserialize, Default)]
struct FlatUsd {
    #[serde(default)]
    input: f64,
    #[serde(default)]
    output: f64,
    #[serde(default)]
    cache_create: f64,
    #[serde(default)]
    cache_read: f64,
}

pub struct Pricing {
    exact: HashMap<String, ModelPrice>,
    norm: HashMap<String, ModelPrice>,
    /// DeepSeek models with Beijing-time peak/off-peak pricing. Looked up first
    /// in `cost()` so these official rates override any flat live-source entry.
    peak: HashMap<String, PeakPrice>,
    schedule: PeakSchedule,
}

/// Strip provider path prefix (after last '/') and unify version separators
/// so "z-ai/glm-5.1", "glm-5p1" and "glm-5.1" all collapse to one key.
fn normalize_key(s: &str) -> String {
    let base = s.rsplit('/').next().unwrap_or(s);
    base.to_lowercase().replace('.', "p")
}

fn bare(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

fn cache_dir() -> Option<PathBuf> {
    let dir = dirs::cache_dir()?.join("ocscale");
    let _ = fs::create_dir_all(&dir);
    Some(dir)
}

/// User-editable pricing file: `data_dir/ocscale/pricing.json` (same location
/// as the other persisted prefs). On first run (file absent) the bundled
/// default is copied out so it's easy to find and edit — future price changes
/// only require editing this file, not rebuilding the app. A present-but-
/// unparseable file is left in place (so the user can repair it) and the
/// bundled default is returned instead.
fn read_external_pricing() -> String {
    let Some(path) = dirs::data_dir().map(|d| d.join("ocscale").join("pricing.json")) else {
        return PRICING_SNAPSHOT.to_string();
    };
    if path.exists() {
        if let Ok(text) = fs::read_to_string(&path) {
            if serde_json::from_str::<PricingFile>(&text).is_ok() {
                return text;
            }
        }
    } else if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
        let _ = fs::write(&path, PRICING_SNAPSHOT);
    }
    PRICING_SNAPSHOT.to_string()
}

/// A models.dev payload: at least one provider with a non-empty `models` map.
fn valid_modelsdev(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| {
            v.as_object().map(|root| {
                root.values().any(|p| {
                    p.get("models")
                        .and_then(|m| m.as_object())
                        .map(|m| !m.is_empty())
                        .unwrap_or(false)
                })
            })
        })
        .unwrap_or(false)
}

/// A LiteLLM payload: at least one entry carrying a per-token cost field.
fn valid_litellm(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| {
            v.as_object().map(|root| {
                root.values().filter_map(|m| m.as_object()).any(|m| {
                    m.contains_key("input_cost_per_token")
                        || m.contains_key("output_cost_per_token")
                })
            })
        })
        .unwrap_or(false)
}

/// Read a fresh (<24h) cache for `name`, else fetch `url` & cache it, else fall
/// back to any stale cache. Returns the raw JSON text. `valid` gates what gets
/// written to the cache: a 200 carrying a JSON error envelope (CDN/proxy/rate
/// limit) would otherwise poison the cache for 24h with zero usable prices, so
/// we only persist a body that actually parses as a price table — and keep the
/// previous good cache otherwise.
fn fetch_cached(name: &str, url: &str, valid: impl Fn(&str) -> bool) -> Option<String> {
    let path = cache_dir()?.join(format!("{name}.json"));
    if let Ok(meta) = fs::metadata(&path) {
        let fresh = meta
            .modified()
            .ok()
            .and_then(|m| SystemTime::now().duration_since(m).ok())
            .map(|age| age < MAX_AGE)
            .unwrap_or(false);
        if fresh {
            if let Ok(t) = fs::read_to_string(&path) {
                return Some(t);
            }
        }
    }
    // fetch fresh — only overwrite the cache if the body validates as a table
    if let Ok(resp) = ureq::get(url).timeout(Duration::from_secs(10)).call() {
        if let Ok(text) = resp.into_string() {
            if valid(&text) {
                let _ = fs::write(&path, &text);
                return Some(text);
            }
        }
    }
    // stale cache as last resort
    fs::read_to_string(&path).ok()
}

impl Pricing {
    pub fn load() -> Self {
        let mut p = Pricing {
            exact: HashMap::new(),
            norm: HashMap::new(),
            peak: HashMap::new(),
            schedule: PeakSchedule::default(),
        };
        // 1. models.dev — primary (inserted first, so it wins on conflict)
        if let Some(text) = fetch_cached("modelsdev", MODELSDEV_URL, valid_modelsdev) {
            p.ingest_modelsdev(&text);
        }
        // 2. LiteLLM — fills gaps models.dev doesn't cover
        if let Some(text) = fetch_cached("litellm", LITELLM_URL, valid_litellm) {
            p.ingest_litellm(&text);
        }
        // 3. bundled LiteLLM snapshot — offline fallback for anything the live
        //    sources didn't supply (only fills gaps; live prices already won).
        p.ingest_litellm(LITELLM_SNAPSHOT);
        // 4. external pricing file (`data_dir/ocscale/pricing.json`) — DeepSeek
        //    official rates are checked first in `cost()`, so they always win;
        //    its `flat_usd` entries fill the remaining gaps. Falls back to the
        //    bundled default if the external file is missing or malformed.
        if !p.ingest_external(&read_external_pricing()) {
            let _ = p.ingest_external(PRICING_SNAPSHOT);
        }
        p
    }

    /// Just the bundled external-pricing snapshot — no disk, no network.
    /// Returned by `shared()` before the background loader has run, so the
    /// DeepSeek/Claude models still price during the first moments after
    /// launch.
    fn snapshot_only() -> Self {
        let mut p = Pricing {
            exact: HashMap::new(),
            norm: HashMap::new(),
            peak: HashMap::new(),
            schedule: PeakSchedule::default(),
        };
        p.ingest_external(PRICING_SNAPSHOT);
        p
    }

    /// The process-wide memoized price table (cheap Arc clone). Never blocks on
    /// disk/network — until `reload_shared` has populated the cell it returns the
    /// built-in snapshot, so callers holding BUILD_LOCK are never stalled.
    pub fn shared() -> Arc<Pricing> {
        if let Some(lock) = PRICING.get() {
            if let Ok(g) = lock.read() {
                return g.clone();
            }
        }
        Arc::new(Pricing::snapshot_only())
    }

    /// Load the full table (cache read + network on cold/stale cache) and swap it
    /// into the shared cell. MUST run on a background thread — never the main
    /// thread or a BUILD_LOCK holder — since the fetch can block up to ~20s.
    pub fn reload_shared() {
        let p = Arc::new(Pricing::load());
        match PRICING.get() {
            Some(lock) => {
                if let Ok(mut g) = lock.write() {
                    *g = p;
                }
            }
            None => {
                let _ = PRICING.set(RwLock::new(p));
            }
        }
    }

    fn insert(&mut self, id: &str, price: ModelPrice) {
        if price.is_zero() {
            return;
        }
        self.exact.entry(id.to_string()).or_insert_with(|| price.clone());
        self.exact.entry(bare(id).to_string()).or_insert_with(|| price.clone());
        self.norm.entry(normalize_key(id)).or_insert(price);
    }

    // models.dev: { provider: { models: { id: { cost: {input,output,cache_read,cache_write} } } } }
    // cost is per-1M tokens → divide by 1e6 for per-token.
    fn ingest_modelsdev(&mut self, text: &str) {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else { return };
        let Some(root) = json.as_object() else { return };
        // gather (id, price); bare ids (no '/') first so official-vendor prices win
        let mut entries: Vec<(String, ModelPrice)> = Vec::new();
        for prov in root.values() {
            let Some(models) = prov.get("models").and_then(|m| m.as_object()) else { continue };
            for (id, m) in models {
                let Some(c) = m.get("cost").and_then(|c| c.as_object()) else { continue };
                let g = |k: &str| c.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
                let price = ModelPrice {
                    input: g("input") / 1e6,
                    output: g("output") / 1e6,
                    cache_create: g("cache_write") / 1e6,
                    cache_read: g("cache_read") / 1e6,
                };
                entries.push((id.clone(), price));
            }
        }
        entries.sort_by_key(|(id, _)| id.contains('/')); // false(0)=bare first
        for (id, price) in entries {
            self.insert(&id, price);
        }
    }

    // LiteLLM: { key: { input_cost_per_token, output_cost_per_token, ... } } — already per-token.
    fn ingest_litellm(&mut self, text: &str) {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else { return };
        let Some(root) = json.as_object() else { return };
        let mut entries: Vec<(String, ModelPrice)> = Vec::new();
        for (id, m) in root {
            let Some(o) = m.as_object() else { continue };
            let g = |k: &str| o.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let price = ModelPrice {
                input: g("input_cost_per_token"),
                output: g("output_cost_per_token"),
                cache_create: g("cache_creation_input_token_cost"),
                cache_read: g("cache_read_input_token_cost"),
            };
            entries.push((id.clone(), price));
        }
        entries.sort_by_key(|(id, _)| id.contains('/'));
        for (id, price) in entries {
            self.insert(&id, price);
        }
    }

    /// Ingest the external pricing file (see `PricingFile`). Returns false if
    /// the text isn't valid — callers then fall back to the bundled snapshot.
    /// DeepSeek entries populate the `peak` map (looked up first in `cost()`),
    /// so the official time-of-day rates override any flat live-source entry;
    /// cache-write tokens bill at the cache-miss input rate. `flat_usd`
    /// entries are ordinary gap-filling backstops.
    fn ingest_external(&mut self, text: &str) -> bool {
        let Ok(file) = serde_json::from_str::<PricingFile>(text) else {
            return false;
        };
        self.schedule = PeakSchedule {
            tz_offset_secs: file.timezone_offset_hours * 3600,
            windows: file.peak_windows.iter().map(|w| (w[0], w[1])).collect(),
            weekend_offpeak: file.weekend_offpeak,
            weekend_offpeak_from_day: file.weekend_offpeak_from.as_deref().and_then(beijing_day_of_date),
        };
        let cny_to_usd = |w: &CnyWindow| ModelPrice {
            input: w.input_miss / CNY_PER_USD / 1e6,
            output: w.output / CNY_PER_USD / 1e6,
            cache_create: w.input_miss / CNY_PER_USD / 1e6,
            cache_read: w.input_hit / CNY_PER_USD / 1e6,
        };
        for (id, w) in file.models {
            let offpeak = cny_to_usd(&w.offpeak);
            let peak = cny_to_usd(w.peak.as_ref().unwrap_or(&w.offpeak));
            if !offpeak.is_zero() || !peak.is_zero() {
                self.peak.insert(id, PeakPrice { peak, offpeak });
            }
        }
        for (id, f) in file.flat_usd {
            let price = ModelPrice {
                input: f.input,
                output: f.output,
                cache_create: f.cache_create,
                cache_read: f.cache_read,
            };
            if !price.is_zero() {
                self.insert(&id, price);
            }
        }
        true
    }

    fn lookup(&self, model: &str) -> Option<&ModelPrice> {
        if let Some(p) = self.exact.get(model) {
            return Some(p);
        }
        self.norm.get(&normalize_key(model))
    }

    /// Exact-or-normalized cost in USD, at the `ts_ms`-implied Beijing peak /
    /// off-peak rate for DeepSeek models with time-of-day pricing (weekends
    /// off-peak once the external file's `weekend_offpeak_from` date is
    /// reached). `None` = no pricing data for this model. Reasoning tokens are
    /// priced at the output rate.
    pub fn cost(
        &self,
        model: &str,
        input: f64,
        output: f64,
        cache_create: f64,
        cache_read: f64,
        reasoning: f64,
        ts_ms: i64,
    ) -> Option<f64> {
        let p = self
            .peak
            .get(model)
            .or_else(|| self.peak.get(&normalize_key(model)))
            .map(|pp| if self.schedule.is_peak(ts_ms) { &pp.peak } else { &pp.offpeak })
            .or_else(|| self.lookup(model))?;
        Some(
            input * p.input
                + output * p.output
                + cache_create * p.cache_create
                + cache_read * p.cache_read
                + reasoning * p.output,
        )
    }

    #[allow(dead_code)]
    pub fn known(&self, model: &str) -> bool {
        self.lookup(model).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Epoch ms for a given hour of day in Beijing (UTC+8), on the epoch day
    /// (Thursday 1970-01-01). `ts_ms = 0` is 08:00 Beijing, so
    /// `ts_at_beijing_hour(h)` = (h − 8) hours shifted.
    fn ts_at_beijing_hour(hour: i64) -> i64 {
        (hour - 8) * 3600 * 1000
    }

    /// Epoch ms for a given Beijing date (YYYY-MM-DD) and hour of day.
    fn ts_at_beijing_date_time(date: &str, hour: i64) -> i64 {
        let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
        let dt = d.and_hms_opt(hour as u32, 0, 0).unwrap().and_utc();
        (dt.timestamp() - 8 * 3600) * 1000
    }

    /// The schedule from the bundled `pricing.json`: Beijing +8, peak windows
    /// 09–12 & 14–18, weekend off-peak from 2026-08-23.
    fn test_schedule() -> PeakSchedule {
        let p = Pricing::snapshot_only();
        p.schedule.clone()
    }

    #[test]
    fn beijing_peak_window_boundaries() {
        let sched = test_schedule();
        // Epoch day is a Thursday, before the weekend rule's start date.
        assert!(!sched.is_peak(ts_at_beijing_hour(8))); // 08:00 off-peak
        assert!(sched.is_peak(ts_at_beijing_hour(9)));
        assert!(sched.is_peak(ts_at_beijing_hour(11)));
        assert!(!sched.is_peak(ts_at_beijing_hour(12))); // 12:00 off-peak
        assert!(!sched.is_peak(ts_at_beijing_hour(13)));
        assert!(sched.is_peak(ts_at_beijing_hour(14)));
        assert!(sched.is_peak(ts_at_beijing_hour(17)));
        assert!(!sched.is_peak(ts_at_beijing_hour(18))); // 18:00 off-peak
    }

    #[test]
    fn beijing_weekend_offpeak_from_2026_08_23() {
        let sched = test_schedule();
        // 2026-08-22 is Saturday — the weekend rule starts 08-23, so peak
        // windows still apply that day.
        assert!(sched.is_peak(ts_at_beijing_date_time("2026-08-22", 10)));
        assert!(!sched.is_peak(ts_at_beijing_date_time("2026-08-22", 13)));
        // 2026-08-23 is Sunday — weekend off-peak all day.
        assert!(!sched.is_peak(ts_at_beijing_date_time("2026-08-23", 10)));
        // 2026-08-24 is Monday — peak windows apply again.
        assert!(sched.is_peak(ts_at_beijing_date_time("2026-08-24", 10)));
        // 2026-08-29 is Saturday — weekend off-peak again.
        assert!(!sched.is_peak(ts_at_beijing_date_time("2026-08-29", 10)));
    }

    #[test]
    fn deepseek_v4_flash_prices_peak_and_offpeak() {
        let p = Pricing::snapshot_only();
        // 1M miss input + 1M cache write + 1M cache hit + 1M output (USD).
        // Off-peak: miss ¥1.5 + write ¥1.5 + out ¥4.5 + hit ¥0.05.
        let off = (1.5 + 1.5 + 4.5 + 0.05) / 7.2;
        let peak = (3.0 + 3.0 + 9.0 + 0.10) / 7.2;
        // Monday 02:00 off-peak, Monday 10:00 peak.
        let c = p
            .cost("deepseek-v4-flash", 1e6, 1e6, 1e6, 1e6, 0.0, ts_at_beijing_date_time("2026-08-24", 2))
            .unwrap();
        assert!((c - off).abs() < 1e-9, "off-peak cost={c} expected={off}");
        // Peak: miss ¥3.0 + write ¥3.0 + out ¥9.0 + hit ¥0.10.
        let c = p
            .cost("deepseek-v4-flash", 1e6, 1e6, 1e6, 1e6, 0.0, ts_at_beijing_date_time("2026-08-24", 10))
            .unwrap();
        assert!((c - peak).abs() < 1e-9, "peak cost={c} expected={peak}");
        // Sunday 10:00 — weekend off-peak rule applies.
        let c = p
            .cost("deepseek-v4-flash", 1e6, 1e6, 1e6, 1e6, 0.0, ts_at_beijing_date_time("2026-08-23", 10))
            .unwrap();
        assert!((c - off).abs() < 1e-9, "weekend cost={c} expected={off}");
    }

    #[test]
    fn deepseek_v4_pro_prices_peak_and_offpeak() {
        let p = Pricing::snapshot_only();
        let c = p
            .cost("deepseek-v4-pro", 1e6, 1e6, 1e6, 1e6, 0.0, ts_at_beijing_date_time("2026-08-24", 3))
            .unwrap();
        let off = (4.5 + 4.5 + 13.5 + 0.15) / 7.2;
        assert!((c - off).abs() < 1e-9, "off-peak cost={c} expected={off}");
        let c = p
            .cost("deepseek-v4-pro", 1e6, 1e6, 1e6, 1e6, 0.0, ts_at_beijing_date_time("2026-08-24", 15))
            .unwrap();
        let peak = (9.0 + 9.0 + 27.0 + 0.30) / 7.2;
        assert!((c - peak).abs() < 1e-9, "peak cost={c} expected={peak}");
    }

    #[test]
    fn deepseek_v4_flash_vision_exp_priced_from_file() {
        let p = Pricing::snapshot_only();
        let c = p
            .cost("deepseek-v4-flash-vision-exp", 1e6, 1e6, 1e6, 1e6, 0.0, ts_at_beijing_date_time("2026-08-24", 2))
            .unwrap();
        let off = (1.5 + 1.5 + 4.5 + 0.05) / 7.2;
        assert!((c - off).abs() < 1e-9, "off-peak cost={c} expected={off}");
    }

    #[test]
    fn flat_usd_backstop_from_external_file() {
        let p = Pricing::snapshot_only();
        // 1M input + 1M output at the bundled per-token USD rates.
        let c = p
            .cost("claude-opus-4-7", 1e6, 1e6, 0.0, 0.0, 0.0, ts_at_beijing_date_time("2026-08-24", 10))
            .unwrap();
        let expected = 1e6 * (5e-6 + 25e-6);
        assert!((c - expected).abs() < 1e-9, "cost={c} expected={expected}");
    }

    #[test]
    fn malformed_external_file_is_rejected() {
        let mut p = Pricing {
            exact: HashMap::new(),
            norm: HashMap::new(),
            peak: HashMap::new(),
            schedule: PeakSchedule::default(),
        };
        assert!(!p.ingest_external("{ not json"));
        assert!(p.peak.is_empty());
        // A file without `models` still yields a valid (empty) table.
        assert!(p.ingest_external("{\"flat_usd\":{}}"));
    }
}
