use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
}

impl TokenUsage {
    pub fn delta_from(&self, previous: &Self) -> Self {
        let reset_detected = self.input_tokens < previous.input_tokens
            || self.cached_input_tokens < previous.cached_input_tokens
            || self.output_tokens < previous.output_tokens
            || self.reasoning_output_tokens < previous.reasoning_output_tokens;

        if reset_detected {
            return self.clone();
        }

        Self {
            input_tokens: self.input_tokens - previous.input_tokens,
            cached_input_tokens: self.cached_input_tokens - previous.cached_input_tokens,
            output_tokens: self.output_tokens - previous.output_tokens,
            reasoning_output_tokens: self.reasoning_output_tokens
                - previous.reasoning_output_tokens,
        }
    }

    pub fn add_assign(&mut self, other: &Self) {
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_output_tokens += other.reasoning_output_tokens;
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
pub struct DailyUsage {
    pub provider_id: String,
    pub date: String,
    pub model_breakdown: Vec<ModelUsage>,
    pub totals: TokenUsage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelUsage {
    pub model_name: String,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PriceQuote {
    pub input_per_million_usd: f64,
    pub cached_input_per_million_usd: Option<f64>,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSnapshot {
    pub provider_id: String,
    pub date: String,
    pub title: String,
    pub tooltip: String,
    pub total_cost_usd: f64,
    pub totals: TokenUsage,
    pub model_costs: Vec<CostBreakdown>,
    pub pricing_updated_at: Option<String>,
    pub used_stale_pricing: bool,
    pub last_refreshed_at: String,
    pub error_message: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{TokenUsage, UsageSnapshot};
    use crate::pricing::normalize_model_for_pricing;
    use crate::providers::codex::parse_session_jsonl;

    #[test]
    fn parse_session_jsonl_aggregates_token_count_deltas_for_today() {
        let jsonl = r#"{"timestamp":"2026-03-17T02:05:57.058Z","type":"session_meta","payload":{"id":"abc","model_provider":"openai"}}
{"timestamp":"2026-03-17T02:07:26.915Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":5,"reasoning_output_tokens":1,"total_tokens":105}}}}
{"timestamp":"2026-03-17T02:10:26.915Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150,"cached_input_tokens":30,"output_tokens":9,"reasoning_output_tokens":2,"total_tokens":159}}}}
{"timestamp":"2026-03-16T23:59:59.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":999,"cached_input_tokens":999,"output_tokens":999,"reasoning_output_tokens":999,"total_tokens":999}}}}"#;

        let snapshots = parse_session_jsonl(
            Path::new("C:/Users/test/.codex/sessions/2026/03/17/test.jsonl"),
            jsonl,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 17).unwrap(),
        )
        .expect("session should parse");

        assert_eq!(
            snapshots,
            vec![
                UsageSnapshot {
                    provider_id: "codex".to_string(),
                    model_name: "gpt-5".to_string(),
                    timestamp: "2026-03-17T02:07:26.915Z".to_string(),
                    usage: TokenUsage {
                        input_tokens: 100,
                        cached_input_tokens: 20,
                        output_tokens: 5,
                        reasoning_output_tokens: 1,
                    },
                },
                UsageSnapshot {
                    provider_id: "codex".to_string(),
                    model_name: "gpt-5".to_string(),
                    timestamp: "2026-03-17T02:10:26.915Z".to_string(),
                    usage: TokenUsage {
                        input_tokens: 50,
                        cached_input_tokens: 10,
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
        let jsonl = r#"{"timestamp":"2026-03-16T23:50:00.000Z","type":"turn_context","payload":{"model":"gpt-5.4"}}
{"timestamp":"2026-03-16T23:55:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":5,"reasoning_output_tokens":1,"total_tokens":105}}}}
{"timestamp":"2026-03-17T00:10:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150,"cached_input_tokens":30,"output_tokens":9,"reasoning_output_tokens":2,"total_tokens":159}}}}"#;

        let snapshots = parse_session_jsonl(
            Path::new("C:/Users/test/.codex/sessions/2026/03/16/test.jsonl"),
            jsonl,
            chrono::NaiveDate::from_ymd_opt(2026, 3, 17).unwrap(),
        )
        .expect("session should parse");

        assert_eq!(snapshots.len(), 1);
        assert_eq!(
            snapshots[0].usage,
            TokenUsage {
                input_tokens: 50,
                cached_input_tokens: 10,
                output_tokens: 4,
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
