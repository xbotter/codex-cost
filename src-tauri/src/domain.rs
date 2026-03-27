use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaMode {
    Target,
    Cap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaSettings {
    pub enabled: bool,
    pub mode: QuotaMode,
    pub amount_usd: f64,
}

pub type ProviderQuotaSettings = BTreeMap<String, QuotaSettings>;

impl Default for QuotaSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: QuotaMode::Target,
            amount_usd: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardSettings {
    pub always_on_top: bool,
    #[serde(default = "default_provider_id")]
    pub current_provider: String,
    #[serde(default = "default_enabled_provider_ids")]
    pub enabled_providers: Vec<String>,
}

fn default_provider_id() -> String {
    "codex".to_string()
}

fn default_enabled_provider_ids() -> Vec<String> {
    vec!["codex".to_string(), "claude".to_string()]
}

impl Default for DashboardSettings {
    fn default() -> Self {
        Self {
            always_on_top: false,
            current_provider: default_provider_id(),
            enabled_providers: default_enabled_provider_ids(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaSnapshot {
    pub mode: QuotaMode,
    pub amount_usd: f64,
    pub progress_ratio: f64,
    pub primary_label: String,
    pub status_label: String,
    pub is_error_state: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotWarning {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSettingsSummary {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub status_label: String,
    pub has_local_data: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
}

impl TokenUsage {
    pub fn delta_from(&self, previous: &Self) -> Self {
        let reset_detected = self.input_tokens < previous.input_tokens
            || self.cached_input_tokens < previous.cached_input_tokens
            || self.cache_creation_input_tokens < previous.cache_creation_input_tokens
            || self.output_tokens < previous.output_tokens
            || self.reasoning_output_tokens < previous.reasoning_output_tokens;

        if reset_detected {
            return self.clone();
        }

        Self {
            input_tokens: self.input_tokens - previous.input_tokens,
            cached_input_tokens: self.cached_input_tokens - previous.cached_input_tokens,
            cache_creation_input_tokens: self.cache_creation_input_tokens
                - previous.cache_creation_input_tokens,
            output_tokens: self.output_tokens - previous.output_tokens,
            reasoning_output_tokens: self.reasoning_output_tokens
                - previous.reasoning_output_tokens,
        }
    }

    pub fn add_assign(&mut self, other: &Self) {
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.cache_creation_input_tokens += other.cache_creation_input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_output_tokens += other.reasoning_output_tokens;
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            + self.cached_input_tokens
            + self.cache_creation_input_tokens
            + self.output_tokens
            + self.reasoning_output_tokens
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub provider_id: String,
    pub model_name: String,
    pub timestamp: String,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeatmapDay {
    pub date: String,
    pub total_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageHeatmap {
    pub provider_id: String,
    pub today: String,
    pub days: Vec<HeatmapDay>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyUsage {
    pub provider_id: String,
    pub date: String,
    pub model_breakdown: Vec<ModelUsage>,
    pub totals: TokenUsage,
    #[serde(default)]
    pub skipped_log_lines: u64,
    #[serde(default)]
    pub skipped_log_files: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelUsage {
    pub model_name: String,
    pub usage: TokenUsage,
    pub usage_timeline: Vec<TokenUsage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PriceQuote {
    pub input_per_million_usd: f64,
    pub cached_input_per_million_usd: Option<f64>,
    pub cache_creation_input_per_million_usd: Option<f64>,
    pub output_per_million_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub model_name: String,
    pub normalized_model_name: String,
    pub input_cost_usd: f64,
    pub cached_input_cost_usd: f64,
    pub output_cost_usd: f64,
    pub total_cost_usd: f64,
    pub usage: TokenUsage,
    pub cost_sparkline: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSnapshot {
    pub provider_id: String,
    pub enabled_provider_ids: Vec<String>,
    pub date: String,
    pub title: String,
    pub tooltip: String,
    pub total_cost_usd: f64,
    pub total_cost_sparkline: Vec<f64>,
    pub totals: TokenUsage,
    pub model_costs: Vec<CostBreakdown>,
    pub pricing_updated_at: Option<String>,
    pub used_stale_pricing: bool,
    pub last_refreshed_at: String,
    pub quota: Option<QuotaSnapshot>,
    pub dashboard_always_on_top: bool,
    #[serde(default)]
    pub warning: Option<SnapshotWarning>,
    pub error_message: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use chrono::{Local, TimeZone};

    use super::{
        DashboardSettings, QuotaMode, QuotaSettings, QuotaSnapshot, TokenUsage, UsageSnapshot,
    };
    use crate::pricing::normalize_model_for_pricing;
    use crate::providers::codex::parse_session_jsonl;

    #[test]
    fn quota_settings_default_to_disabled_target_mode() {
        let settings = QuotaSettings::default();

        assert!(!settings.enabled);
        assert_eq!(settings.mode, QuotaMode::Target);
        assert_eq!(settings.amount_usd, 0.0);
    }

    #[test]
    fn dashboard_settings_default_to_not_always_on_top() {
        let settings = DashboardSettings::default();

        assert!(!settings.always_on_top);
        assert_eq!(settings.current_provider, "codex");
        assert_eq!(settings.enabled_providers, vec!["codex", "claude"]);
    }

    #[test]
    fn quota_settings_serde_round_trip_supports_target_and_cap_modes() {
        let target = QuotaSettings {
            enabled: true,
            mode: QuotaMode::Target,
            amount_usd: 250.0,
        };
        let cap = QuotaSettings {
            enabled: true,
            mode: QuotaMode::Cap,
            amount_usd: 125.5,
        };

        let target_json = serde_json::to_string(&target).expect("target should serialize");
        let cap_json = serde_json::to_string(&cap).expect("cap should serialize");

        assert_eq!(
            serde_json::from_str::<QuotaSettings>(&target_json).expect("target should deserialize"),
            target
        );
        assert_eq!(
            serde_json::from_str::<QuotaSettings>(&cap_json).expect("cap should deserialize"),
            cap
        );
    }

    #[test]
    fn quota_snapshot_serde_round_trip_preserves_rendered_fields() {
        let snapshot = QuotaSnapshot {
            mode: QuotaMode::Cap,
            amount_usd: 250.0,
            progress_ratio: 0.737,
            primary_label: "Cap $250".to_string(),
            status_label: "$65.78 left".to_string(),
            is_error_state: false,
        };

        let json = serde_json::to_string(&snapshot).expect("quota snapshot should serialize");
        let decoded = serde_json::from_str::<QuotaSnapshot>(&json)
            .expect("quota snapshot should deserialize");

        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn parse_session_jsonl_aggregates_token_count_deltas_for_today() {
        let first_timestamp = Local
            .with_ymd_and_hms(2026, 3, 17, 10, 7, 26)
            .single()
            .unwrap()
            .to_rfc3339();
        let second_timestamp = Local
            .with_ymd_and_hms(2026, 3, 17, 10, 10, 26)
            .single()
            .unwrap()
            .to_rfc3339();
        let previous_day_timestamp = Local
            .with_ymd_and_hms(2026, 3, 16, 23, 59, 59)
            .single()
            .unwrap()
            .to_rfc3339();
        let jsonl = format!(
            r#"{{"timestamp":"2026-03-17T02:05:57.058Z","type":"session_meta","payload":{{"id":"abc","model_provider":"openai"}}}}
{{"timestamp":"{first_timestamp}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":100,"cached_input_tokens":20,"output_tokens":5,"reasoning_output_tokens":1,"total_tokens":105}}}}}}}}
{{"timestamp":"{second_timestamp}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":150,"cached_input_tokens":30,"output_tokens":9,"reasoning_output_tokens":2,"total_tokens":159}}}}}}}}
{{"timestamp":"{previous_day_timestamp}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":999,"cached_input_tokens":999,"output_tokens":999,"reasoning_output_tokens":999,"total_tokens":999}}}}}}}}"#
        );

        let snapshots = parse_session_jsonl(
            Path::new("C:/Users/test/.codex/sessions/2026/03/17/test.jsonl"),
            &jsonl,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 17).unwrap(),
        )
        .expect("session should parse");

        assert_eq!(
            snapshots,
            vec![
                UsageSnapshot {
                    provider_id: "codex".to_string(),
                    model_name: "gpt-5".to_string(),
                    timestamp: first_timestamp,
                    usage: TokenUsage {
                        input_tokens: 100,
                        cached_input_tokens: 20,
                        cache_creation_input_tokens: 0,
                        output_tokens: 5,
                        reasoning_output_tokens: 1,
                    },
                },
                UsageSnapshot {
                    provider_id: "codex".to_string(),
                    model_name: "gpt-5".to_string(),
                    timestamp: second_timestamp,
                    usage: TokenUsage {
                        input_tokens: 50,
                        cached_input_tokens: 10,
                        cache_creation_input_tokens: 0,
                        output_tokens: 4,
                        reasoning_output_tokens: 1,
                    },
                },
            ]
        );
    }

    #[test]
    fn parse_session_jsonl_uses_latest_snapshot_when_counters_reset() {
        let jsonl = r#"{"timestamp":"2026-03-17T02:05:57.058Z","type":"session_meta","payload":{"id":"abc","model_provider":"openai"}}
{"timestamp":"2026-03-17T02:07:26.915Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":5,"reasoning_output_tokens":1,"total_tokens":105}}}}
{"timestamp":"2026-03-17T03:10:26.915Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":15,"cached_input_tokens":3,"output_tokens":2,"reasoning_output_tokens":0,"total_tokens":17}}}}"#;

        let snapshots = parse_session_jsonl(
            Path::new("C:/Users/test/.codex/sessions/2026/03/17/test.jsonl"),
            jsonl,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 17).unwrap(),
        )
        .expect("session should parse");

        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[1].usage.input_tokens, 15);
        assert_eq!(snapshots[1].usage.cached_input_tokens, 3);
        assert_eq!(snapshots[1].usage.cache_creation_input_tokens, 0);
        assert_eq!(snapshots[1].usage.output_tokens, 2);
    }

    #[test]
    fn parse_session_jsonl_uses_turn_context_model_for_following_token_counts() {
        let jsonl = r#"{"timestamp":"2026-03-17T02:05:57.058Z","type":"session_meta","payload":{"id":"abc","model_provider":"openai"}}
{"timestamp":"2026-03-17T02:06:00.000Z","type":"turn_context","payload":{"model":"gpt-5.4","collaboration_mode":{"settings":{"model":"gpt-5.4"}}}}
{"timestamp":"2026-03-17T02:07:26.915Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":5,"reasoning_output_tokens":1,"total_tokens":105}}}}"#;

        let snapshots = parse_session_jsonl(
            Path::new("C:/Users/test/.codex/sessions/2026/03/17/test.jsonl"),
            jsonl,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 17).unwrap(),
        )
        .expect("session should parse");

        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].model_name, "gpt-5.4");
    }

    #[test]
    fn parse_session_jsonl_uses_previous_day_totals_as_baseline_for_first_event_of_day() {
        let previous_day_timestamp = Local
            .with_ymd_and_hms(2026, 3, 16, 23, 55, 0)
            .single()
            .unwrap()
            .to_rfc3339();
        let target_day_timestamp = Local
            .with_ymd_and_hms(2026, 3, 17, 0, 10, 0)
            .single()
            .unwrap()
            .to_rfc3339();
        let jsonl = format!(
            r#"{{"timestamp":"2026-03-16T23:50:00.000Z","type":"turn_context","payload":{{"model":"gpt-5.4"}}}}
{{"timestamp":"{previous_day_timestamp}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":100,"cached_input_tokens":20,"output_tokens":5,"reasoning_output_tokens":1,"total_tokens":105}}}}}}}}
{{"timestamp":"{target_day_timestamp}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":150,"cached_input_tokens":30,"output_tokens":9,"reasoning_output_tokens":2,"total_tokens":159}}}}}}}}"#
        );

        let snapshots = parse_session_jsonl(
            Path::new("C:/Users/test/.codex/sessions/2026/03/16/test.jsonl"),
            &jsonl,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 17).unwrap(),
        )
        .expect("session should parse");

        assert_eq!(snapshots.len(), 1);
        assert_eq!(
            snapshots[0].usage,
            TokenUsage {
                input_tokens: 50,
                cached_input_tokens: 10,
                cache_creation_input_tokens: 0,
                output_tokens: 4,
                reasoning_output_tokens: 1,
            }
        );
    }

    #[test]
    fn parse_session_jsonl_skips_malformed_lines_without_failing_the_whole_session() {
        let jsonl = r#"{"timestamp":"2026-03-17T02:05:57.058Z","type":"turn_context","payload":{"model":"gpt-5.4"}}
not-json
{"timestamp":"2026-03-17T02:07:26.915Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":5,"reasoning_output_tokens":1,"total_tokens":105}}}}"#;

        let snapshots = parse_session_jsonl(
            Path::new("C:/Users/test/.codex/sessions/2026/03/17/test.jsonl"),
            jsonl,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 17).unwrap(),
        )
        .expect("session should parse");

        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].model_name, "gpt-5.4");
        assert_eq!(
            snapshots[0].usage,
            TokenUsage {
                input_tokens: 100,
                cached_input_tokens: 20,
                cache_creation_input_tokens: 0,
                output_tokens: 5,
                reasoning_output_tokens: 1,
            }
        );
    }

    #[test]
    fn normalize_model_for_pricing_maps_codex_aliases_to_canonical_openai_models() {
        assert_eq!(normalize_model_for_pricing("gpt-5-codex"), "gpt-5");
        assert_eq!(normalize_model_for_pricing("gpt-5"), "gpt-5");
        assert_eq!(
            normalize_model_for_pricing("gpt-5-mini-codex"),
            "gpt-5-mini"
        );
    }
}
