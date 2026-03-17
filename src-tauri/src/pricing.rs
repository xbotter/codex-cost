use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use chrono::Utc;
use reqwest::{blocking::ClientBuilder, Proxy, Url};
use serde::{Deserialize, Serialize};

use crate::domain::PriceQuote;
#[path = "../proxy_config.rs"]
mod proxy_config;

const LITELLM_PRICING_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const BUNDLED_PRICING_SOURCE_URL: &str = "bundled://codex-cost/default-pricing";
const BUNDLED_PRICING_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/bundled-pricing.json"));

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
        self.load_with_fetcher(force_refresh, || self.fetch_remote())
    }

    fn load_with_fetcher<F>(&self, force_refresh: bool, fetcher: F) -> Result<PricingSnapshot>
    where
        F: FnOnce() -> Result<PricingCache>,
    {
        if !force_refresh {
            if let Some(cache) = self.load_if_fresh()? {
                return Ok(PricingSnapshot {
                    used_stale_cache: is_bundled_pricing_cache(&cache),
                    cache,
                });
            }
        }

        match fetcher() {
            Ok(cache) => {
                self.persist(&cache)?;
                Ok(PricingSnapshot {
                    cache,
                    used_stale_cache: false,
                })
            }
            Err(fetch_error) => {
                let cache = match self.load_any_cache()? {
                    Some(cache) => cache,
                    None => {
                        let cache = bundled_pricing_cache().with_context(|| {
                            format!(
                                "failed to refresh pricing and no cache exists: {fetch_error:#}"
                            )
                        })?;
                        self.persist(&cache)?;
                        cache
                    }
                };
                Ok(PricingSnapshot {
                    cache,
                    used_stale_cache: true,
                })
            }
        }
    }

    fn fetch_remote(&self) -> Result<PricingCache> {
        let client = pricing_client_builder()?
            .timeout(Duration::from_secs(15))
            .build()
            .context("failed to build LiteLLM pricing client")?;
        let response = client
            .get(LITELLM_PRICING_URL)
            .send()
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

fn pricing_client_builder() -> Result<ClientBuilder> {
    let mut builder = reqwest::blocking::Client::builder();
    if proxy_config::explicit_proxy_env_is_set() {
        return Ok(builder);
    }

    let Some(config) = proxy_config::load_system_proxy_config() else {
        return Ok(builder);
    };

    let target_url = Url::parse(LITELLM_PRICING_URL).context("invalid LiteLLM pricing URL")?;
    let Some(host) = target_url.host_str() else {
        return Ok(builder);
    };

    if proxy_config::should_bypass_proxy(host, &config.exceptions) {
        return Ok(builder);
    }

    if let Some(proxy_url) = proxy_url_for_target(&target_url, &config) {
        builder =
            builder.proxy(Proxy::all(&proxy_url).context("failed to configure pricing proxy")?);
    }

    Ok(builder)
}

fn bundled_pricing_cache() -> Result<PricingCache> {
    serde_json::from_str::<PricingCache>(BUNDLED_PRICING_JSON)
        .context("failed to decode bundled pricing fallback")
}

fn is_bundled_pricing_cache(cache: &PricingCache) -> bool {
    cache.source_url == BUNDLED_PRICING_SOURCE_URL
}

fn proxy_url_for_target(
    target_url: &Url,
    config: &proxy_config::SystemProxyConfig,
) -> Option<String> {
    let endpoint = match target_url.scheme() {
        "https" => config
            .https
            .as_ref()
            .or(config.http.as_ref())
            .or(config.socks.as_ref()),
        "http" => config
            .http
            .as_ref()
            .or(config.https.as_ref())
            .or(config.socks.as_ref()),
        _ => config
            .socks
            .as_ref()
            .or(config.https.as_ref())
            .or(config.http.as_ref()),
    }?;

    let scheme = if target_url.scheme() == "https" && config.https.as_ref() == Some(endpoint) {
        "http"
    } else if target_url.scheme() == "http" && config.http.as_ref() == Some(endpoint) {
        "http"
    } else if config.socks.as_ref() == Some(endpoint) {
        "socks5h"
    } else {
        "http"
    };

    Some(format!("{scheme}://{}:{}", endpoint.host, endpoint.port))
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use anyhow::{anyhow, Result};

    use super::{PricingStore, BUNDLED_PRICING_SOURCE_URL, LITELLM_PRICING_URL};

    #[test]
    fn load_uses_bundled_pricing_when_fetch_fails_and_no_cache_exists() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("codex-cost-pricing-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(&root)?;

        let store = PricingStore::new(root.join("pricing-cache.json"), Duration::from_secs(3600));
        let snapshot = store.load_with_fetcher(false, || Err(anyhow!("network timeout")))?;

        assert!(snapshot.used_stale_cache);
        assert!(snapshot.cache.prices.contains_key("gpt-5"));
        assert!(
            snapshot.cache.source_url == BUNDLED_PRICING_SOURCE_URL
                || snapshot.cache.source_url == LITELLM_PRICING_URL
        );
        assert!(store.cache_path.exists());

        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn load_marks_bundled_cache_as_stale_without_refetching() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "codex-cost-pricing-test-fresh-{}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(&root)?;

        let store = PricingStore::new(root.join("pricing-cache.json"), Duration::from_secs(3600));
        let first = store.load_with_fetcher(false, || Err(anyhow!("network timeout")))?;
        assert!(first.used_stale_cache);

        let second = store.load_with_fetcher(false, || panic!("should not refetch"))?;
        assert_eq!(
            second.used_stale_cache,
            second.cache.source_url == BUNDLED_PRICING_SOURCE_URL
        );
        assert!(
            second.cache.source_url == BUNDLED_PRICING_SOURCE_URL
                || second.cache.source_url == LITELLM_PRICING_URL
        );

        fs::remove_dir_all(&root)?;
        Ok(())
    }
}
