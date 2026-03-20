use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Local;

use crate::domain::{
    AppSnapshot, CostBreakdown, DailyUsage, DashboardSettings, PriceQuote, QuotaMode,
    QuotaSettings, QuotaSnapshot, SnapshotWarning, TokenUsage,
};
use crate::pricing::{find_price_quote, normalize_model_for_pricing, PricingCache, PricingStore};
use crate::providers::{claude::ClaudeUsageProvider, codex::CodexUsageProvider, UsageProvider};
use crate::settings::SettingsStore;

#[derive(Clone)]
pub struct UsageAppService {
    providers: Vec<Arc<dyn UsageProvider>>,
    pricing_store: PricingStore,
    settings_store: SettingsStore,
}

pub fn format_token_count(value: u64) -> String {
    if value >= 1_000_000 {
        return format!("{:.1}M", value as f64 / 1_000_000.0);
    }

    if value >= 1_000 {
        return format!("{:.1}K", value as f64 / 1_000.0);
    }

    value.to_string()
}

pub fn billable_input_tokens(usage: &TokenUsage) -> u64 {
    usage
        .input_tokens
        .saturating_sub(usage.cached_input_tokens)
        .saturating_add(usage.cache_creation_input_tokens)
}

pub fn total_output_tokens(usage: &TokenUsage) -> u64 {
    usage.output_tokens + usage.reasoning_output_tokens
}

pub fn provider_display_name(provider_id: &str) -> &'static str {
    match provider_id {
        "claude" => "Claude Code",
        _ => "Codex",
    }
}

impl UsageAppService {
    pub fn new() -> Result<Self> {
        let codex_root = CodexUsageProvider::default_root()?;
        let pricing_cache = PricingStore::default_cache_path()?;
        let settings_path = SettingsStore::default_config_path()?;

        Ok(Self {
            providers: vec![
                Arc::new(CodexUsageProvider::new(codex_root)),
                Arc::new(ClaudeUsageProvider::new(
                    ClaudeUsageProvider::default_root()?
                )),
            ],
            pricing_store: PricingStore::new(pricing_cache, Duration::from_secs(60 * 60 * 24)),
            settings_store: SettingsStore::new(settings_path),
        })
    }

    pub fn refresh(&self, force_pricing_refresh: bool) -> Result<AppSnapshot> {
        let today = Local::now().date_naive();
        let quota_settings = self.settings_store.load_quota_settings()?;
        let dashboard_settings = self.settings_store.load_dashboard_settings()?;
        let current_provider_id = dashboard_settings.current_provider.as_str();
        let provider = self
            .providers
            .iter()
            .filter(|provider| {
                dashboard_settings
                    .enabled_providers
                    .iter()
                    .any(|enabled| enabled == provider.id())
            })
            .find(|provider| provider.id() == current_provider_id)
            .or_else(|| {
                self.providers.iter().find(|provider| {
                    dashboard_settings
                        .enabled_providers
                        .iter()
                        .any(|enabled| enabled == provider.id())
                })
            })
            .context("no usage providers configured")?;
        let usage = match provider.collect_daily_usage(today) {
            Ok(usage) => usage,
            Err(error) => {
                return Ok(build_error_snapshot_with_quota_for_provider(
                    format!("{error:#}"),
                    provider.id(),
                    dashboard_settings.enabled_providers.clone(),
                    &quota_settings,
                    dashboard_settings.always_on_top,
                ))
            }
        };
        let pricing = self.pricing_store.load(force_pricing_refresh)?;

        Ok(build_app_snapshot(
            usage,
            &dashboard_settings.enabled_providers,
            &quota_settings,
            dashboard_settings.always_on_top,
            &pricing.cache,
            pricing.used_stale_cache,
            Local::now(),
        ))
    }

    pub fn load_quota_settings(&self) -> Result<QuotaSettings> {
        self.settings_store.load_quota_settings()
    }

    pub fn save_quota_settings(&self, settings: &QuotaSettings) -> Result<QuotaSettings> {
        self.settings_store.save_quota_settings(settings)
    }

    pub fn load_dashboard_settings(&self) -> Result<DashboardSettings> {
        self.settings_store.load_dashboard_settings()
    }

    pub fn save_dashboard_settings(
        &self,
        settings: &DashboardSettings,
    ) -> Result<DashboardSettings> {
        self.settings_store.save_dashboard_settings(settings)
    }
}

fn build_app_snapshot(
    usage: DailyUsage,
    enabled_provider_ids: &[String],
    quota_settings: &QuotaSettings,
    dashboard_always_on_top: bool,
    pricing: &PricingCache,
    used_stale_pricing: bool,
    now: chrono::DateTime<Local>,
) -> AppSnapshot {
    let provider_id = usage.provider_id.clone();
    let date = usage.date.clone();
    let totals = usage.totals.clone();
    let skipped_log_lines = usage.skipped_log_lines;
    let skipped_log_files = usage.skipped_log_files;
    let mut total_cost_usd = 0.0;
    let mut total_cost_sparkline = vec![0.0; 48];
    let mut model_costs = Vec::new();

    for model_usage in usage.model_breakdown {
        let normalized_model_name = normalize_model_for_pricing(&model_usage.model_name);
        let Some(price_quote) = find_price_quote(pricing, &model_usage.model_name) else {
            continue;
        };

        let input_cost_usd = input_cost_for_usage(&model_usage.usage, &price_quote)
            + cache_creation_input_cost_for_usage(&model_usage.usage, &price_quote);
        let cached_input_cost_usd = cached_input_cost_for_usage(&model_usage.usage, &price_quote);
        let output_cost_usd = output_cost_for_usage(&model_usage.usage, &price_quote);
        let total = input_cost_usd + cached_input_cost_usd + output_cost_usd;
        total_cost_usd += total;
        let cost_sparkline = model_usage
            .usage_timeline
            .iter()
            .map(|usage| total_cost_for_usage(usage, &price_quote))
            .collect::<Vec<_>>();
        for (index, bucket_cost) in cost_sparkline.iter().enumerate() {
            total_cost_sparkline[index] += bucket_cost;
        }

        model_costs.push(CostBreakdown {
            model_name: model_usage.model_name,
            normalized_model_name,
            input_cost_usd,
            cached_input_cost_usd,
            output_cost_usd,
            total_cost_usd: total,
            usage: model_usage.usage,
            cost_sparkline,
        });
    }

    let title = format!("${total_cost_usd:.2}");
    let provider_label = provider_display_name(&provider_id);
    let tooltip = format!(
        "{provider_label} today: ${total_cost_usd:.2}\n↑ {}   ⚡ {}   ↓ {}",
        format_token_count(billable_input_tokens(&totals)),
        format_token_count(totals.cached_input_tokens),
        format_token_count(total_output_tokens(&totals))
    );
    let warning = build_snapshot_warning(
        &provider_id,
        &model_costs,
        skipped_log_lines,
        skipped_log_files,
    );

    AppSnapshot {
        provider_id,
        enabled_provider_ids: enabled_provider_ids.to_vec(),
        date,
        title,
        tooltip,
        total_cost_usd,
        total_cost_sparkline,
        totals,
        model_costs,
        pricing_updated_at: Some(pricing.fetched_at.clone()),
        used_stale_pricing,
        last_refreshed_at: now.to_rfc3339(),
        quota: build_quota_snapshot(quota_settings, total_cost_usd, false),
        dashboard_always_on_top,
        warning,
        error_message: None,
    }
}

fn build_snapshot_warning(
    provider_id: &str,
    model_costs: &[CostBreakdown],
    skipped_log_lines: u64,
    skipped_log_files: u64,
) -> Option<SnapshotWarning> {
    if provider_id != "claude" || !model_costs.is_empty() || skipped_log_lines == 0 {
        return None;
    }

    let _ = skipped_log_files;

    Some(SnapshotWarning {
        kind: "partial_data".to_string(),
        message: "Some Claude Code log lines were unreadable and were skipped.".to_string(),
    })
}

fn build_quota_snapshot(
    settings: &QuotaSettings,
    total_cost_usd: f64,
    is_error_state: bool,
) -> Option<QuotaSnapshot> {
    if !settings.enabled {
        return None;
    }

    let progress_ratio = if settings.amount_usd > 0.0 {
        (total_cost_usd / settings.amount_usd).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let amount_label = format_usd_label(settings.amount_usd);
    let primary_label = match settings.mode {
        QuotaMode::Target => format!("Target {amount_label}"),
        QuotaMode::Cap => format!("Cap {amount_label}"),
    };

    let status_label = if is_error_state {
        "Unavailable".to_string()
    } else {
        match settings.mode {
            QuotaMode::Target => format!("{:.0}% reached", progress_ratio * 100.0),
            QuotaMode::Cap => {
                let remaining = settings.amount_usd - total_cost_usd;
                if remaining >= 0.0 {
                    format!("${remaining:.2} left")
                } else {
                    format!("Over by ${:.2}", remaining.abs())
                }
            }
        }
    };

    Some(QuotaSnapshot {
        mode: settings.mode.clone(),
        amount_usd: settings.amount_usd,
        progress_ratio,
        primary_label,
        status_label,
        is_error_state,
    })
}

fn format_usd_label(amount: f64) -> String {
    if (amount.fract()).abs() < f64::EPSILON {
        format!("${amount:.0}")
    } else {
        format!("${amount:.2}")
    }
}

fn input_cost_for_usage(usage: &TokenUsage, price_quote: &PriceQuote) -> f64 {
    usage.input_tokens.saturating_sub(usage.cached_input_tokens) as f64 / 1_000_000.0
        * price_quote.input_per_million_usd
}

fn cache_creation_input_cost_for_usage(usage: &TokenUsage, price_quote: &PriceQuote) -> f64 {
    usage.cache_creation_input_tokens as f64 / 1_000_000.0
        * price_quote
            .cache_creation_input_per_million_usd
            .unwrap_or(price_quote.input_per_million_usd)
}

fn cached_input_cost_for_usage(usage: &TokenUsage, price_quote: &PriceQuote) -> f64 {
    usage.cached_input_tokens as f64 / 1_000_000.0
        * price_quote
            .cached_input_per_million_usd
            .unwrap_or(price_quote.input_per_million_usd)
}

fn output_cost_for_usage(usage: &TokenUsage, price_quote: &PriceQuote) -> f64 {
    total_output_tokens(usage) as f64 / 1_000_000.0 * price_quote.output_per_million_usd
}

fn total_cost_for_usage(usage: &TokenUsage, price_quote: &PriceQuote) -> f64 {
    input_cost_for_usage(usage, price_quote)
        + cache_creation_input_cost_for_usage(usage, price_quote)
        + cached_input_cost_for_usage(usage, price_quote)
        + output_cost_for_usage(usage, price_quote)
}

pub fn build_error_snapshot(message: impl Into<String>) -> AppSnapshot {
    build_error_snapshot_with_quota_for_provider(
        message,
        "codex",
        DashboardSettings::default().enabled_providers,
        &QuotaSettings::default(),
        false,
    )
}

#[allow(dead_code)]
pub fn build_error_snapshot_with_quota(
    message: impl Into<String>,
    quota_settings: &QuotaSettings,
    dashboard_always_on_top: bool,
) -> AppSnapshot {
    build_error_snapshot_with_quota_for_provider(
        message,
        "codex",
        DashboardSettings::default().enabled_providers,
        quota_settings,
        dashboard_always_on_top,
    )
}

pub fn build_error_snapshot_with_quota_for_provider(
    message: impl Into<String>,
    provider_id: &str,
    enabled_provider_ids: Vec<String>,
    quota_settings: &QuotaSettings,
    dashboard_always_on_top: bool,
) -> AppSnapshot {
    let now = Local::now();
    let message = message.into();

    AppSnapshot {
        provider_id: provider_id.to_string(),
        enabled_provider_ids,
        date: now.date_naive().to_string(),
        title: "Error".to_string(),
        tooltip: message.clone(),
        total_cost_usd: 0.0,
        total_cost_sparkline: vec![0.0; 48],
        totals: TokenUsage::default(),
        model_costs: Vec::new(),
        pricing_updated_at: None,
        used_stale_pricing: false,
        last_refreshed_at: now.to_rfc3339(),
        quota: build_quota_snapshot(quota_settings, 0.0, true),
        dashboard_always_on_top,
        warning: None,
        error_message: Some(message),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{Local, TimeZone};

    use crate::domain::{
        DailyUsage, DashboardSettings, QuotaMode, QuotaSettings, SnapshotWarning, TokenUsage,
    };
    use crate::pricing::{LiteLlmPrice, PricingCache};

    use super::{build_app_snapshot, build_error_snapshot_with_quota};

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
                    cache_creation_input_tokens: 0,
                    output_tokens: 10,
                    reasoning_output_tokens: 0,
                },
                usage_timeline: vec![TokenUsage::default(); 48],
            }],
            totals: TokenUsage {
                input_tokens: 100,
                cached_input_tokens: 40,
                cache_creation_input_tokens: 0,
                output_tokens: 10,
                reasoning_output_tokens: 0,
            },
            skipped_log_lines: 0,
            skipped_log_files: 0,
        };

        let mut prices = HashMap::new();
        prices.insert(
            "gpt-5.4".to_string(),
            LiteLlmPrice {
                input_cost_per_token: Some(2.0 / 1_000_000.0),
                cache_read_input_token_cost: Some(0.5 / 1_000_000.0),
                cache_creation_input_token_cost: None,
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
            &DashboardSettings::default().enabled_providers,
            &QuotaSettings::default(),
            false,
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
                    cache_creation_input_tokens: 0,
                    output_tokens: 100,
                    reasoning_output_tokens: 50,
                },
                usage_timeline: vec![TokenUsage::default(); 48],
            }],
            totals: TokenUsage {
                input_tokens: 0,
                cached_input_tokens: 0,
                cache_creation_input_tokens: 0,
                output_tokens: 100,
                reasoning_output_tokens: 50,
            },
            skipped_log_lines: 0,
            skipped_log_files: 0,
        };

        let mut prices = HashMap::new();
        prices.insert(
            "gpt-5.4".to_string(),
            LiteLlmPrice {
                input_cost_per_token: Some(2.0 / 1_000_000.0),
                cache_read_input_token_cost: Some(0.5 / 1_000_000.0),
                cache_creation_input_token_cost: None,
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
            &DashboardSettings::default().enabled_providers,
            &QuotaSettings::default(),
            false,
            &pricing,
            false,
            Local.with_ymd_and_hms(2026, 3, 17, 12, 0, 0).unwrap(),
        );

        let model = &snapshot.model_costs[0];
        assert!((model.output_cost_usd - 0.0012).abs() < 1e-9);
    }

    #[test]
    fn build_app_snapshot_projects_half_hour_cost_sparkline() {
        let mut usage_timeline = vec![TokenUsage::default(); 48];
        usage_timeline[0] = TokenUsage {
            input_tokens: 100,
            cached_input_tokens: 40,
            cache_creation_input_tokens: 0,
            output_tokens: 10,
            reasoning_output_tokens: 0,
        };
        usage_timeline[1] = TokenUsage {
            input_tokens: 50,
            cached_input_tokens: 10,
            cache_creation_input_tokens: 0,
            output_tokens: 5,
            reasoning_output_tokens: 5,
        };

        let usage = DailyUsage {
            provider_id: "codex".to_string(),
            date: "2026-03-17".to_string(),
            model_breakdown: vec![crate::domain::ModelUsage {
                model_name: "gpt-5.4".to_string(),
                usage: TokenUsage {
                    input_tokens: 150,
                    cached_input_tokens: 50,
                    cache_creation_input_tokens: 0,
                    output_tokens: 15,
                    reasoning_output_tokens: 5,
                },
                usage_timeline,
            }],
            totals: TokenUsage {
                input_tokens: 150,
                cached_input_tokens: 50,
                cache_creation_input_tokens: 0,
                output_tokens: 15,
                reasoning_output_tokens: 5,
            },
            skipped_log_lines: 0,
            skipped_log_files: 0,
        };

        let mut prices = HashMap::new();
        prices.insert(
            "gpt-5.4".to_string(),
            LiteLlmPrice {
                input_cost_per_token: Some(2.0 / 1_000_000.0),
                cache_read_input_token_cost: Some(0.5 / 1_000_000.0),
                cache_creation_input_token_cost: None,
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
            &DashboardSettings::default().enabled_providers,
            &QuotaSettings::default(),
            false,
            &pricing,
            false,
            Local.with_ymd_and_hms(2026, 3, 17, 12, 0, 0).unwrap(),
        );

        let model = &snapshot.model_costs[0];
        assert_eq!(model.cost_sparkline.len(), 48);
        assert!((model.cost_sparkline[0] - 0.00022).abs() < 1e-9);
        assert!((model.cost_sparkline[1] - 0.000165).abs() < 1e-9);
        assert_eq!(model.cost_sparkline[2], 0.0);
    }

    #[test]
    fn build_app_snapshot_aggregates_total_cost_sparkline() {
        let mut model_a_timeline = vec![TokenUsage::default(); 48];
        model_a_timeline[0] = TokenUsage {
            input_tokens: 100,
            cached_input_tokens: 40,
            cache_creation_input_tokens: 0,
            output_tokens: 10,
            reasoning_output_tokens: 0,
        };

        let mut model_b_timeline = vec![TokenUsage::default(); 48];
        model_b_timeline[0] = TokenUsage {
            input_tokens: 50,
            cached_input_tokens: 0,
            cache_creation_input_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 0,
        };
        model_b_timeline[1] = TokenUsage {
            input_tokens: 25,
            cached_input_tokens: 5,
            cache_creation_input_tokens: 0,
            output_tokens: 10,
            reasoning_output_tokens: 0,
        };

        let usage = DailyUsage {
            provider_id: "codex".to_string(),
            date: "2026-03-17".to_string(),
            model_breakdown: vec![
                crate::domain::ModelUsage {
                    model_name: "gpt-5.4".to_string(),
                    usage: TokenUsage {
                        input_tokens: 100,
                        cached_input_tokens: 40,
                        cache_creation_input_tokens: 0,
                        output_tokens: 10,
                        reasoning_output_tokens: 0,
                    },
                    usage_timeline: model_a_timeline,
                },
                crate::domain::ModelUsage {
                    model_name: "gpt-5-mini".to_string(),
                    usage: TokenUsage {
                        input_tokens: 75,
                        cached_input_tokens: 5,
                        cache_creation_input_tokens: 0,
                        output_tokens: 30,
                        reasoning_output_tokens: 0,
                    },
                    usage_timeline: model_b_timeline,
                },
            ],
            totals: TokenUsage {
                input_tokens: 175,
                cached_input_tokens: 45,
                cache_creation_input_tokens: 0,
                output_tokens: 40,
                reasoning_output_tokens: 0,
            },
            skipped_log_lines: 0,
            skipped_log_files: 0,
        };

        let mut prices = HashMap::new();
        prices.insert(
            "gpt-5.4".to_string(),
            LiteLlmPrice {
                input_cost_per_token: Some(2.0 / 1_000_000.0),
                cache_read_input_token_cost: Some(0.5 / 1_000_000.0),
                cache_creation_input_token_cost: None,
                output_cost_per_token: Some(8.0 / 1_000_000.0),
            },
        );
        prices.insert(
            "gpt-5-mini".to_string(),
            LiteLlmPrice {
                input_cost_per_token: Some(0.4 / 1_000_000.0),
                cache_read_input_token_cost: Some(0.1 / 1_000_000.0),
                cache_creation_input_token_cost: None,
                output_cost_per_token: Some(1.6 / 1_000_000.0),
            },
        );

        let pricing = PricingCache {
            fetched_at: "2026-03-17T00:00:00Z".to_string(),
            source_url: "test".to_string(),
            prices,
        };

        let snapshot = build_app_snapshot(
            usage,
            &DashboardSettings::default().enabled_providers,
            &QuotaSettings::default(),
            false,
            &pricing,
            false,
            Local.with_ymd_and_hms(2026, 3, 17, 12, 0, 0).unwrap(),
        );

        assert_eq!(snapshot.total_cost_sparkline.len(), 48);
        assert!((snapshot.total_cost_sparkline[0] - 0.000272).abs() < 1e-9);
        assert!((snapshot.total_cost_sparkline[1] - 0.0000245).abs() < 1e-9);
        assert_eq!(snapshot.total_cost_sparkline[2], 0.0);
    }

    #[test]
    fn build_app_snapshot_creates_target_quota_labels() {
        let usage = DailyUsage {
            provider_id: "codex".to_string(),
            date: "2026-03-17".to_string(),
            model_breakdown: vec![],
            totals: TokenUsage::default(),
            skipped_log_lines: 0,
            skipped_log_files: 0,
        };
        let pricing = PricingCache {
            fetched_at: "2026-03-17T00:00:00Z".to_string(),
            source_url: "test".to_string(),
            prices: HashMap::new(),
        };
        let quota = QuotaSettings {
            enabled: true,
            mode: QuotaMode::Target,
            amount_usd: 250.0,
        };

        let snapshot = build_app_snapshot(
            usage,
            &DashboardSettings::default().enabled_providers,
            &quota,
            false,
            &pricing,
            false,
            Local.with_ymd_and_hms(2026, 3, 17, 12, 0, 0).unwrap(),
        );

        let rendered = snapshot.quota.expect("quota should render");
        assert_eq!(rendered.primary_label, "Target $250");
        assert_eq!(rendered.status_label, "0% reached");
        assert!(!rendered.is_error_state);
    }

    #[test]
    fn build_error_snapshot_with_quota_renders_unavailable_state() {
        let quota = QuotaSettings {
            enabled: true,
            mode: QuotaMode::Cap,
            amount_usd: 250.0,
        };

        let snapshot = build_error_snapshot_with_quota("provider failed", &quota, false);

        let rendered = snapshot.quota.expect("quota should render");
        assert_eq!(rendered.primary_label, "Cap $250");
        assert_eq!(rendered.status_label, "Unavailable");
        assert!(rendered.is_error_state);
    }

    #[test]
    fn build_app_snapshot_adds_claude_warning_when_empty_after_skipping_logs() {
        let usage = DailyUsage {
            provider_id: "claude".to_string(),
            date: "2026-03-20".to_string(),
            model_breakdown: Vec::new(),
            totals: TokenUsage::default(),
            skipped_log_lines: 2,
            skipped_log_files: 1,
        };

        let snapshot = build_app_snapshot(
            usage,
            &DashboardSettings::default().enabled_providers,
            &QuotaSettings::default(),
            false,
            &PricingCache {
                fetched_at: "2026-03-20T00:00:00Z".to_string(),
                source_url: "test".to_string(),
                prices: HashMap::new(),
            },
            false,
            Local.with_ymd_and_hms(2026, 3, 20, 12, 0, 0).unwrap(),
        );

        assert_eq!(
            snapshot.warning,
            Some(SnapshotWarning {
                kind: "partial_data".to_string(),
                message: "Some Claude Code log lines were unreadable and were skipped.".to_string(),
            })
        );
        assert!(snapshot.error_message.is_none());
    }

    #[test]
    fn build_app_snapshot_hides_claude_warning_when_models_are_present() {
        let usage = DailyUsage {
            provider_id: "claude".to_string(),
            date: "2026-03-20".to_string(),
            model_breakdown: vec![crate::domain::ModelUsage {
                model_name: "glm-5".to_string(),
                usage: TokenUsage {
                    input_tokens: 100,
                    cached_input_tokens: 20,
                    cache_creation_input_tokens: 0,
                    output_tokens: 10,
                    reasoning_output_tokens: 0,
                },
                usage_timeline: vec![TokenUsage::default(); 48],
            }],
            totals: TokenUsage {
                input_tokens: 100,
                cached_input_tokens: 20,
                cache_creation_input_tokens: 0,
                output_tokens: 10,
                reasoning_output_tokens: 0,
            },
            skipped_log_lines: 3,
            skipped_log_files: 2,
        };

        let mut prices = HashMap::new();
        prices.insert(
            "zai/glm-5".to_string(),
            LiteLlmPrice {
                input_cost_per_token: Some(1.0 / 1_000_000.0),
                cache_read_input_token_cost: Some(0.1 / 1_000_000.0),
                cache_creation_input_token_cost: Some(0.0),
                output_cost_per_token: Some(4.0 / 1_000_000.0),
            },
        );

        let snapshot = build_app_snapshot(
            usage,
            &DashboardSettings::default().enabled_providers,
            &QuotaSettings::default(),
            false,
            &PricingCache {
                fetched_at: "2026-03-20T00:00:00Z".to_string(),
                source_url: "test".to_string(),
                prices,
            },
            false,
            Local.with_ymd_and_hms(2026, 3, 20, 12, 0, 0).unwrap(),
        );

        assert!(snapshot.warning.is_none());
        assert_eq!(snapshot.model_costs.len(), 1);
    }

    #[test]
    fn build_app_snapshot_never_adds_warning_for_codex() {
        let usage = DailyUsage {
            provider_id: "codex".to_string(),
            date: "2026-03-20".to_string(),
            model_breakdown: Vec::new(),
            totals: TokenUsage::default(),
            skipped_log_lines: 4,
            skipped_log_files: 1,
        };

        let snapshot = build_app_snapshot(
            usage,
            &DashboardSettings::default().enabled_providers,
            &QuotaSettings::default(),
            false,
            &PricingCache {
                fetched_at: "2026-03-20T00:00:00Z".to_string(),
                source_url: "test".to_string(),
                prices: HashMap::new(),
            },
            false,
            Local.with_ymd_and_hms(2026, 3, 20, 12, 0, 0).unwrap(),
        );

        assert!(snapshot.warning.is_none());
    }
}
