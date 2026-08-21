// Token pricing. Primary source: models.dev (bare model names, matches Claude
// CLI logs). Fallback: LiteLLM. Final backstop: a tiny built-in snapshot.
//
// Matching is layered: exact id → normalized id (strip provider path prefix +
// unify the ".'↔'p" version separator, e.g. "glm-5.1" ⇄ "glm-5p1").
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
// models, not just the few hardcoded in `ingest_builtin`. Live sources, when
// reachable, are ingested first and win.
const LITELLM_SNAPSHOT: &str = include_str!("../snapshots/litellm.json");

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

pub struct Pricing {
    exact: HashMap<String, ModelPrice>,
    norm: HashMap<String, ModelPrice>,
    /// DeepSeek models with Beijing-time peak/off-peak pricing. Looked up first
    /// in `cost()` so these official rates override any flat live-source entry.
    peak: HashMap<String, PeakPrice>,
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

/// Whether an epoch-ms timestamp lands in DeepSeek's Beijing peak window:
/// 09:00–12:00 and 14:00–18:00 (Beijing = UTC+8, no DST). Everything else is
/// off-peak. Pure arithmetic on the UTC+8 shift, so it never needs a timezone
/// database and handles pre-epoch timestamps via `div_euclid`/`rem_euclid`.
fn is_beijing_peak(ts_ms: i64) -> bool {
    let secs = ts_ms.div_euclid(1000) + 8 * 3600;
    let hour = secs.div_euclid(3600).rem_euclid(24);
    (9..12).contains(&hour) || (14..18).contains(&hour)
}

impl Pricing {
    pub fn load() -> Self {
        let mut p = Pricing {
            exact: HashMap::new(),
            norm: HashMap::new(),
            peak: HashMap::new(),
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
        // 4. built-in backstop (a handful of core models, last resort)
        p.ingest_builtin();
        p
    }

    /// Just the built-in snapshot — no disk, no network. Returned by `shared()`
    /// before the background loader has run, so the common Claude models still
    /// price during the first moments after launch.
    fn builtin_only() -> Self {
        let mut p = Pricing {
            exact: HashMap::new(),
            norm: HashMap::new(),
            peak: HashMap::new(),
        };
        p.ingest_builtin();
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
        Arc::new(Pricing::builtin_only())
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

    fn ingest_builtin(&mut self) {
        let mk = |i: f64, o: f64, cc: f64, cr: f64| ModelPrice {
            input: i,
            output: o,
            cache_create: cc,
            cache_read: cr,
        };
        let b: &[(&str, ModelPrice)] = &[
            ("claude-opus-4-7", mk(5e-6, 25e-6, 6.25e-6, 0.5e-6)),
            ("claude-opus-4-8", mk(5e-6, 25e-6, 6.25e-6, 0.5e-6)),
            ("claude-sonnet-4-5", mk(3e-6, 15e-6, 3.75e-6, 0.3e-6)),
            ("claude-sonnet-4-6", mk(3e-6, 15e-6, 3.75e-6, 0.3e-6)),
            ("claude-haiku-4-5", mk(1e-6, 5e-6, 1.25e-6, 0.1e-6)),
        ];
        for (id, price) in b {
            self.insert(id, price.clone());
        }
        // deepseek-v4-flash / deepseek-v4-pro — official DeepSeek rates with a
        // Beijing-time peak/off-peak split (CNY per 1M tokens):
        //                    off-peak                  peak
        //   flash  miss ¥1.5 / hit ¥0.05 / out ¥4.5    miss ¥3.0 / hit ¥0.10 / out ¥9.0
        //   pro    miss ¥4.5 / hit ¥0.15 / out ¥13.5   miss ¥9.0 / hit ¥0.30 / out ¥27.0
        // Cache-write tokens bill at the cache-miss input rate. The live
        // LiteLLM table has flash but prices cache writes at 0, so these rates
        // override it. Stored as USD at the zh UI's exchange rate (7.2) so the
        // panel's `cost × 7.2` shows the exact CNY figure. Peak is 09:00–12:00
        // and 14:00–18:00 Beijing time (UTC+8, no DST); see `is_beijing_peak`.
        let cny = |miss: f64, hit: f64, out: f64| mk(
            miss / 7.2 / 1e6,
            out / 7.2 / 1e6,
            miss / 7.2 / 1e6,
            hit / 7.2 / 1e6,
        );
        let mut peak = |name: &str, miss: f64, hit: f64, out: f64| {
            self.peak.insert(
                name.to_string(),
                PeakPrice {
                    peak: cny(miss * 2.0, hit * 2.0, out * 2.0),
                    offpeak: cny(miss, hit, out),
                },
            );
        };
        peak("deepseek-v4-flash", 1.5, 0.05, 4.5);
        peak("deepseek-v4-pro", 4.5, 0.15, 13.5);
    }

    fn lookup(&self, model: &str) -> Option<&ModelPrice> {
        if let Some(p) = self.exact.get(model) {
            return Some(p);
        }
        self.norm.get(&normalize_key(model))
    }

    /// Exact-or-normalized cost in USD, at the `ts_ms`-implied Beijing peak /
    /// off-peak rate for DeepSeek models with time-of-day pricing. `None` = no
    /// pricing data for this model. Reasoning tokens are priced at the output rate.
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
            .map(|pp| if is_beijing_peak(ts_ms) { &pp.peak } else { &pp.offpeak })
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

    /// Epoch ms for a given hour of day in Beijing (UTC+8). `ts_ms = 0` is
    /// 08:00 Beijing, so `ts_at_beijing_hour(h)` = (h − 8) hours shifted.
    fn ts_at_beijing_hour(hour: i64) -> i64 {
        (hour - 8) * 3600 * 1000
    }

    #[test]
    fn beijing_peak_window_boundaries() {
        assert!(!is_beijing_peak(ts_at_beijing_hour(8))); // 08:00 off-peak
        assert!(is_beijing_peak(ts_at_beijing_hour(9)));
        assert!(is_beijing_peak(ts_at_beijing_hour(11)));
        assert!(!is_beijing_peak(ts_at_beijing_hour(12))); // 12:00 off-peak
        assert!(!is_beijing_peak(ts_at_beijing_hour(13)));
        assert!(is_beijing_peak(ts_at_beijing_hour(14)));
        assert!(is_beijing_peak(ts_at_beijing_hour(17)));
        assert!(!is_beijing_peak(ts_at_beijing_hour(18))); // 18:00 off-peak
    }

    #[test]
    fn deepseek_v4_flash_prices_peak_and_offpeak() {
        let p = Pricing::builtin_only();
        // 1M miss input + 1M cache write + 1M cache hit + 1M output (USD).
        // Off-peak: miss ¥1.5 + write ¥1.5 + out ¥4.5 + hit ¥0.05.
        let c = p
            .cost("deepseek-v4-flash", 1e6, 1e6, 1e6, 1e6, 0.0, ts_at_beijing_hour(2))
            .unwrap();
        let off = (1.5 + 1.5 + 4.5 + 0.05) / 7.2;
        assert!((c - off).abs() < 1e-9, "off-peak cost={c} expected={off}");
        // Peak: miss ¥3.0 + write ¥3.0 + out ¥9.0 + hit ¥0.10.
        let c = p
            .cost("deepseek-v4-flash", 1e6, 1e6, 1e6, 1e6, 0.0, ts_at_beijing_hour(10))
            .unwrap();
        let peak = (3.0 + 3.0 + 9.0 + 0.10) / 7.2;
        assert!((c - peak).abs() < 1e-9, "peak cost={c} expected={peak}");
    }

    #[test]
    fn deepseek_v4_pro_prices_peak_and_offpeak() {
        let p = Pricing::builtin_only();
        let c = p
            .cost("deepseek-v4-pro", 1e6, 1e6, 1e6, 1e6, 0.0, ts_at_beijing_hour(3))
            .unwrap();
        let off = (4.5 + 4.5 + 13.5 + 0.15) / 7.2;
        assert!((c - off).abs() < 1e-9, "off-peak cost={c} expected={off}");
        let c = p
            .cost("deepseek-v4-pro", 1e6, 1e6, 1e6, 1e6, 0.0, ts_at_beijing_hour(15))
            .unwrap();
        let peak = (9.0 + 9.0 + 27.0 + 0.30) / 7.2;
        assert!((c - peak).abs() < 1e-9, "peak cost={c} expected={peak}");
    }
}
