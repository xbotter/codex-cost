use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Local;

use crate::domain::{AppSnapshot, CostBreakdown, DailyUsage, TokenUsage};
use crate::pricing::{find_price_quote, normalize_model_for_pricing, PricingCache, PricingStore};
use crate::providers::{codex::CodexUsageProvider, UsageProvider};

#[derive(Clone)]
pub struct UsageAppService {
    providers: Vec<Arc<dyn UsageProvider>>,
    pricing_store: PricingStore,
}

pub fn format_token_count(value: u64) -> String {
    if value >= 1_000_000 {
        let decimals = if value >= 10_000_000 { 0 } else { 1 };
        return format!("{:.*}M", decimals, value as f64 / 1_000_000.0);
    }

    if value >= 1_000 {
        let decimals = if value >= 10_000 { 0 } else { 1 };
        return format!("{:.*}K", decimals, value as f64 / 1_000.0);
    }

    value.to_string()
}

pub fn billable_input_tokens(usage: &TokenUsage) -> u64 {
    usage.input_tokens.saturating_sub(usage.cached_input_tokens)
}

pub fn total_output_tokens(usage: &TokenUsage) -> u64 {
    usage.output_tokens + usage.reasoning_output_tokens
}

impl UsageAppService {
    pub fn new() -> Result<Self> {
        let codex_root = CodexUsageProvider::default_root()?;
        let pricing_cache = PricingStore::default_cache_path()?;

        Ok(Self {
            providers: vec![Arc::new(CodexUsageProvider::new(codex_root))],
            pricing_store: PricingStore::new(pricing_cache, Duration::from_secs(60 * 60 * 24)),
        })
    }

    pub fn refresh(&self, force_pricing_refresh: bool) -> Result<AppSnapshot> {
        let today = Local::now().date_naive();
        let provider = self
            .providers
            .first()
            .context("no usage providers configured")?;
        let usage = provider.collect_daily_usage(today)?;
        let pricing = self.pricing_store.load(force_pricing_refresh)?;

        Ok(build_app_snapshot(
            usage,
            &pricing.cache,
            pricing.used_stale_cache,
            Local::now(),
        ))
    }
}

fn build_app_snapshot(
    usage: DailyUsage,
    pricing: &PricingCache,
    used_stale_pricing: bool,
    now: chrono::DateTime<Local>,
) -> AppSnapshot {
    let mut total_cost_usd = 0.0;
    let mut model_costs = Vec::new();

    for model_usage in usage.model_breakdown {
        let normalized_model_name = normalize_model_for_pricing(&model_usage.model_name);
        let Some(price_quote) = find_price_quote(pricing, &model_usage.model_name) else {
            continue;
        };

        let billable_input_tokens = billable_input_tokens(&model_usage.usage);
        let input_cost_usd =
            billable_input_tokens as f64 / 1_000_000.0 * price_quote.input_per_million_usd;
        let cached_input_cost_usd = model_usage.usage.cached_input_tokens as f64 / 1_000_000.0
            * price_quote
                .cached_input_per_million_usd
                .unwrap_or(price_quote.input_per_million_usd);
        let output_cost_usd = total_output_tokens(&model_usage.usage) as f64 / 1_000_000.0
            * price_quote.output_per_million_usd;
        let total = input_cost_usd + cached_input_cost_usd + output_cost_usd;
        total_cost_usd += total;

        model_costs.push(CostBreakdown {
            model_name: model_usage.model_name,
            normalized_model_name,
            input_cost_usd,
            cached_input_cost_usd,
            output_cost_usd,
            total_cost_usd: total,
            usage: model_usage.usage,
        });
    }

    let title = format!("Codex ${total_cost_usd:.2}");
    let tooltip = format!(
        "Codex today: ${total_cost_usd:.2}\n↑ {}   ⚡ {}   ↓ {}",
        format_token_count(billable_input_tokens(&usage.totals)),
        format_token_count(usage.totals.cached_input_tokens),
        format_token_count(total_output_tokens(&usage.totals))
    );

    AppSnapshot {
        provider_id: usage.provider_id,
        date: usage.date,
        title,
        tooltip,
        total_cost_usd,
        totals: usage.totals,
        model_costs,
        pricing_updated_at: Some(pricing.fetched_at.clone()),
        used_stale_pricing,
        last_refreshed_at: now.to_rfc3339(),
        error_message: None,
    }
}

pub fn build_error_snapshot(message: impl Into<String>) -> AppSnapshot {
    let now = Local::now();
    let message = message.into();

    AppSnapshot {
        provider_id: "codex".to_string(),
        date: now.date_naive().to_string(),
        title: "Codex error".to_string(),
        tooltip: message.clone(),
        total_cost_usd: 0.0,
        totals: TokenUsage::default(),
        model_costs: Vec::new(),
        pricing_updated_at: None,
        used_stale_pricing: false,
        last_refreshed_at: now.to_rfc3339(),
        error_message: Some(message),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{Local, TimeZone};

    use crate::domain::{DailyUsage, TokenUsage};
    use crate::pricing::{LiteLlmPrice, PricingCache};

    use super::build_app_snapshot;

    #[test]
    fn build_app_snapshot_prices_cached_input_only_once() {
        let usage = DailyUsage {
            provider_id: "codex".to_string(),
            date: "2026-03-17".to_string(),
            model_breakdown: vec![crate::domain::ModelUsage {
                model_name: "gpt-5.4".to_string(),
                usage: TokenUsage {
                    input_tokens: 100,
                    cached_input_tokens: 40,
                    output_tokens: 10,
                    reasoning_output_tokens: 0,
                },
            }],
            totals: TokenUsage {
                input_tokens: 100,
                cached_input_tokens: 40,
                output_tokens: 10,
                reasoning_output_tokens: 0,
            },
        };

        let mut prices = HashMap::new();
        prices.insert(
            "gpt-5.4".to_string(),
            LiteLlmPrice {
                input_cost_per_token: Some(2.0 / 1_000_000.0),
                cache_read_input_token_cost: Some(0.5 / 1_000_000.0),
                output_cost_per_token: Some(8.0 / 1_000_000.0),
            },
        );

        let pricing = PricingCache {
            fetched_at: "2026-03-17T00:00:00Z".to_string(),
            source_url: "test".to_string(),
            prices,
        };

        let snapshot = build_app_snapshot(
            usage,
            &pricing,
            false,
            Local.with_ymd_and_hms(2026, 3, 17, 12, 0, 0).unwrap(),
        );

        let model = &snapshot.model_costs[0];
        assert!((model.input_cost_usd - 0.00012).abs() < 1e-9);
        assert!((model.cached_input_cost_usd - 0.00002).abs() < 1e-9);
        assert!((model.output_cost_usd - 0.00008).abs() < 1e-9);
        assert!((model.total_cost_usd - 0.00022).abs() < 1e-9);
    }

    #[test]
    fn build_app_snapshot_counts_reasoning_tokens_as_output_cost() {
        let usage = DailyUsage {
            provider_id: "codex".to_string(),
            date: "2026-03-17".to_string(),
            model_breakdown: vec![crate::domain::ModelUsage {
                model_name: "gpt-5.4".to_string(),
                usage: TokenUsage {
                    input_tokens: 0,
                    cached_input_tokens: 0,
                    output_tokens: 100,
                    reasoning_output_tokens: 50,
                },
            }],
            totals: TokenUsage {
                input_tokens: 0,
                cached_input_tokens: 0,
                output_tokens: 100,
                reasoning_output_tokens: 50,
            },
        };

        let mut prices = HashMap::new();
        prices.insert(
            "gpt-5.4".to_string(),
            LiteLlmPrice {
                input_cost_per_token: Some(2.0 / 1_000_000.0),
                cache_read_input_token_cost: Some(0.5 / 1_000_000.0),
                output_cost_per_token: Some(8.0 / 1_000_000.0),
            },
        );

        let pricing = PricingCache {
            fetched_at: "2026-03-17T00:00:00Z".to_string(),
            source_url: "test".to_string(),
            prices,
        };

        let snapshot = build_app_snapshot(
            usage,
            &pricing,
            false,
            Local.with_ymd_and_hms(2026, 3, 17, 12, 0, 0).unwrap(),
        );

        let model = &snapshot.model_costs[0];
        assert!((model.output_cost_usd - 0.0012).abs() < 1e-9);
    }
}
