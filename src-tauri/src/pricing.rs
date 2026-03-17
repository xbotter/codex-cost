use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::domain::PriceQuote;

const LITELLM_PRICING_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

#[derive(Debug, Clone)]
pub struct PricingStore {
    cache_path: PathBuf,
    ttl: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingCache {
    pub fetched_at: String,
    pub source_url: String,
    pub prices: HashMap<String, LiteLlmPrice>,
}

#[derive(Debug, Clone)]
pub struct PricingSnapshot {
    pub cache: PricingCache,
    pub used_stale_cache: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LiteLlmPrice {
    pub input_cost_per_token: Option<f64>,
    pub cache_read_input_token_cost: Option<f64>,
    pub output_cost_per_token: Option<f64>,
}

impl PricingStore {
    pub fn new(cache_path: PathBuf, ttl: Duration) -> Self {
        Self { cache_path, ttl }
    }

    pub fn default_cache_path() -> Result<PathBuf> {
        let base = dirs::cache_dir().context("cache directory is unavailable")?;
        Ok(base.join("codex-cost").join("pricing-cache.json"))
    }

    pub fn load(&self, force_refresh: bool) -> Result<PricingSnapshot> {
        if !force_refresh {
            if let Some(cache) = self.load_if_fresh()? {
                return Ok(PricingSnapshot {
                    cache,
                    used_stale_cache: false,
                });
            }
        }

        match self.fetch_remote() {
            Ok(cache) => {
                self.persist(&cache)?;
                Ok(PricingSnapshot {
                    cache,
                    used_stale_cache: false,
                })
            }
            Err(fetch_error) => {
                let cache = self.load_any_cache()?.with_context(|| {
                    format!("failed to refresh pricing and no cache exists: {fetch_error:#}")
                })?;
                Ok(PricingSnapshot {
                    cache,
                    used_stale_cache: true,
                })
            }
        }
    }

    fn fetch_remote(&self) -> Result<PricingCache> {
        let response = reqwest::blocking::get(LITELLM_PRICING_URL)
            .context("failed to request LiteLLM pricing")?
            .error_for_status()
            .context("LiteLLM pricing request failed")?;

        let prices = response
            .json::<HashMap<String, LiteLlmPrice>>()
            .context("failed to decode LiteLLM pricing json")?;

        Ok(PricingCache {
            fetched_at: Utc::now().to_rfc3339(),
            source_url: LITELLM_PRICING_URL.to_string(),
            prices,
        })
    }

    fn persist(&self, cache: &PricingCache) -> Result<()> {
        if let Some(parent) = self.cache_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let content =
            serde_json::to_string_pretty(cache).context("failed to serialize pricing cache")?;
        fs::write(&self.cache_path, content)
            .with_context(|| format!("failed to write {}", self.cache_path.display()))?;
        Ok(())
    }

    fn load_if_fresh(&self) -> Result<Option<PricingCache>> {
        let metadata = match fs::metadata(&self.cache_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to stat {}", self.cache_path.display()))
            }
        };

        let modified = metadata.modified().context("failed to read cache mtime")?;
        let age = SystemTime::now()
            .duration_since(modified)
            .unwrap_or_else(|_| Duration::from_secs(0));
        if age > self.ttl {
            return Ok(None);
        }

        self.load_any_cache()
    }

    fn load_any_cache(&self) -> Result<Option<PricingCache>> {
        let content = match fs::read_to_string(&self.cache_path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", self.cache_path.display()))
            }
        };

        let cache = serde_json::from_str(&content).context("failed to decode pricing cache")?;
        Ok(Some(cache))
    }
}

pub fn normalize_model_for_pricing(model: &str) -> String {
    let normalized = model
        .trim()
        .trim_start_matches("openai/")
        .to_ascii_lowercase();
    match normalized.as_str() {
        "gpt-5-codex" => "gpt-5".to_string(),
        "gpt-5-mini-codex" => "gpt-5-mini".to_string(),
        "gpt-5-nano-codex" => "gpt-5-nano".to_string(),
        other => other.to_string(),
    }
}

pub fn find_price_quote(cache: &PricingCache, model: &str) -> Option<PriceQuote> {
    let normalized = normalize_model_for_pricing(model);
    let price = cache.prices.get(&normalized)?;

    let input_per_million_usd = price.input_cost_per_token? * 1_000_000.0;
    let output_per_million_usd = price.output_cost_per_token? * 1_000_000.0;
    let cached_input_per_million_usd = price
        .cache_read_input_token_cost
        .or(price.input_cost_per_token)
        .map(|value| value * 1_000_000.0);

    Some(PriceQuote {
        input_per_million_usd,
        cached_input_per_million_usd,
        output_per_million_usd,
    })
}
