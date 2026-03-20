use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDate, Timelike};
use serde_json::Value;
use walkdir::WalkDir;

use crate::domain::{DailyUsage, ModelUsage, TokenUsage, UsageSnapshot};
use crate::providers::UsageProvider;

const DEFAULT_FALLBACK_MODEL: &str = "gpt-5";

pub struct CodexUsageProvider {
    root: PathBuf,
}

impl CodexUsageProvider {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn default_root() -> Result<PathBuf> {
        let home = dirs::home_dir().context("home directory is unavailable")?;
        Ok(home.join(".codex").join("sessions"))
    }

    fn session_files(&self) -> impl Iterator<Item = PathBuf> + '_ {
        WalkDir::new(&self.root)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
            .filter_map(|entry| {
                let path = entry.into_path();
                let is_jsonl = path.extension().and_then(|value| value.to_str()) == Some("jsonl");
                is_jsonl.then_some(path)
            })
    }
}

impl UsageProvider for CodexUsageProvider {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn collect_daily_usage(&self, date: NaiveDate) -> Result<DailyUsage> {
        if !self.root.exists() {
            return Ok(DailyUsage {
                provider_id: self.id().to_string(),
                date: date.to_string(),
                model_breakdown: Vec::new(),
                totals: TokenUsage::default(),
                skipped_log_lines: 0,
                skipped_log_files: 0,
            });
        }

        let mut per_model = BTreeMap::<String, TokenUsage>::new();
        let mut per_model_timeline = BTreeMap::<String, Vec<TokenUsage>>::new();

        for path in self.session_files() {
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;

            for snapshot in parse_session_jsonl(&path, &contents, date)? {
                let bucket = half_hour_bucket_index(&snapshot.timestamp).unwrap_or(0);
                per_model
                    .entry(snapshot.model_name.clone())
                    .or_default()
                    .add_assign(&snapshot.usage);
                per_model_timeline
                    .entry(snapshot.model_name)
                    .or_insert_with(|| vec![TokenUsage::default(); 48])[bucket]
                    .add_assign(&snapshot.usage);
            }
        }

        let mut totals = TokenUsage::default();
        let mut model_breakdown: Vec<_> = per_model
            .into_iter()
            .map(|(model_name, usage)| {
                totals.add_assign(&usage);
                let usage_timeline = per_model_timeline
                    .remove(&model_name)
                    .unwrap_or_else(|| vec![TokenUsage::default(); 48]);
                ModelUsage {
                    model_name,
                    usage,
                    usage_timeline,
                }
            })
            .collect();
        model_breakdown.sort_by(|left, right| {
            Reverse(left.usage.total_tokens())
                .cmp(&Reverse(right.usage.total_tokens()))
                .then_with(|| {
                    Reverse(left.usage.output_tokens).cmp(&Reverse(right.usage.output_tokens))
                })
                .then_with(|| {
                    Reverse(left.usage.input_tokens).cmp(&Reverse(right.usage.input_tokens))
                })
                .then_with(|| left.model_name.cmp(&right.model_name))
        });

        Ok(DailyUsage {
            provider_id: self.id().to_string(),
            date: date.to_string(),
            model_breakdown,
            totals,
            skipped_log_lines: 0,
            skipped_log_files: 0,
        })
    }
}

pub fn parse_session_jsonl(
    _path: &Path,
    contents: &str,
    target_date: NaiveDate,
) -> Result<Vec<UsageSnapshot>> {
    let mut snapshots = Vec::new();
    let mut current_model = DEFAULT_FALLBACK_MODEL.to_string();
    let mut previous_total = TokenUsage::default();

    for line in contents.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line).context("invalid jsonl line")?;
        let entry_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();

        if entry_type == "session_meta" || entry_type == "turn_context" {
            current_model = extract_model_name(&value);
            continue;
        }

        if entry_type != "event_msg" {
            continue;
        }

        let payload = match value.get("payload") {
            Some(payload) => payload,
            None => continue,
        };

        if payload.get("type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }

        let timestamp = match value.get("timestamp").and_then(Value::as_str) {
            Some(timestamp) => timestamp,
            None => continue,
        };

        let total_usage = match payload
            .get("info")
            .and_then(|info| info.get("total_token_usage"))
        {
            Some(total_usage) => total_usage,
            None => continue,
        };

        let current_total = TokenUsage {
            input_tokens: total_usage
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            cached_input_tokens: total_usage
                .get("cached_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            cache_creation_input_tokens: 0,
            output_tokens: total_usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            reasoning_output_tokens: total_usage
                .get("reasoning_output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
        };

        let delta = current_total.delta_from(&previous_total);
        previous_total = current_total;

        if !timestamp_matches_local_date(timestamp, target_date) {
            continue;
        }

        snapshots.push(UsageSnapshot {
            provider_id: "codex".to_string(),
            model_name: current_model.clone(),
            timestamp: timestamp.to_string(),
            usage: delta,
        });
    }

    Ok(snapshots)
}

fn extract_model_name(value: &Value) -> String {
    value
        .get("payload")
        .and_then(|payload| payload.get("model").and_then(Value::as_str))
        .or_else(|| {
            value.get("payload").and_then(|payload| {
                payload
                    .get("collaboration_mode")
                    .and_then(|mode| mode.get("settings"))
                    .and_then(|settings| settings.get("model"))
                    .and_then(Value::as_str)
            })
        })
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| payload.get("model_name").and_then(Value::as_str))
        })
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| payload.get("current_model").and_then(Value::as_str))
        })
        .unwrap_or(DEFAULT_FALLBACK_MODEL)
        .to_string()
}

fn timestamp_matches_local_date(timestamp: &str, date: NaiveDate) -> bool {
    DateTime::parse_from_rfc3339(timestamp)
        .map(|value| value.with_timezone(&Local).date_naive() == date)
        .unwrap_or(false)
}

fn half_hour_bucket_index(timestamp: &str) -> Option<usize> {
    let local = DateTime::parse_from_rfc3339(timestamp)
        .ok()?
        .with_timezone(&Local);
    Some((local.hour() as usize) * 2 + usize::from(local.minute() >= 30))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;
    use chrono::NaiveDate;

    use super::CodexUsageProvider;
    use crate::providers::UsageProvider;

    #[test]
    fn collect_daily_usage_includes_matching_events_from_older_session_directories() -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("codex-cost-provider-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }

        let older_dir = root.join("2026").join("03").join("16");
        let today_dir = root.join("2026").join("03").join("17");
        fs::create_dir_all(&older_dir)?;
        fs::create_dir_all(&today_dir)?;

        fs::write(
            older_dir.join("older-session.jsonl"),
            r#"{"timestamp":"2026-03-16T23:55:00Z","type":"turn_context","payload":{"model":"gpt-5.4"}}
{"timestamp":"2026-03-17T01:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":120,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":5,"total_tokens":130}}}}
"#,
        )?;

        fs::write(
            today_dir.join("today-session.jsonl"),
            r#"{"timestamp":"2026-03-17T09:00:00Z","type":"turn_context","payload":{"model":"gpt-5.4"}}
{"timestamp":"2026-03-17T09:05:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":200,"cached_input_tokens":40,"output_tokens":20,"reasoning_output_tokens":10,"total_tokens":220}}}}
"#,
        )?;

        let provider = CodexUsageProvider::new(root.clone());
        let usage = provider.collect_daily_usage(NaiveDate::from_ymd_opt(2026, 3, 17).unwrap())?;

        assert_eq!(usage.model_breakdown.len(), 1);
        assert_eq!(usage.model_breakdown[0].model_name, "gpt-5.4");
        assert_eq!(usage.model_breakdown[0].usage.input_tokens, 320);
        assert_eq!(usage.model_breakdown[0].usage.cached_input_tokens, 60);
        assert_eq!(usage.model_breakdown[0].usage.output_tokens, 30);
        assert_eq!(usage.model_breakdown[0].usage.reasoning_output_tokens, 15);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn collect_daily_usage_orders_models_by_total_usage_desc() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "codex-cost-provider-sort-test-{}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }

        let today_dir = root.join("2026").join("03").join("17");
        fs::create_dir_all(&today_dir)?;

        fs::write(
            today_dir.join("session.jsonl"),
            r#"{"timestamp":"2026-03-17T09:00:00Z","type":"turn_context","payload":{"model":"gpt-5-mini"}}
{"timestamp":"2026-03-17T09:05:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"cached_input_tokens":20,"output_tokens":10,"reasoning_output_tokens":0,"total_tokens":80}}}}
{"timestamp":"2026-03-17T09:10:00Z","type":"turn_context","payload":{"model":"gpt-5.4"}}
{"timestamp":"2026-03-17T09:15:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":200,"cached_input_tokens":40,"output_tokens":20,"reasoning_output_tokens":10,"total_tokens":270}}}}
"#,
        )?;

        let provider = CodexUsageProvider::new(root.clone());
        let usage = provider.collect_daily_usage(NaiveDate::from_ymd_opt(2026, 3, 17).unwrap())?;

        assert_eq!(usage.model_breakdown.len(), 2);
        assert_eq!(usage.model_breakdown[0].model_name, "gpt-5.4");
        assert_eq!(usage.model_breakdown[1].model_name, "gpt-5-mini");

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn collect_daily_usage_groups_model_timeline_into_half_hour_buckets() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "codex-cost-provider-buckets-test-{}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }

        let today_dir = root.join("2026").join("03").join("17");
        fs::create_dir_all(&today_dir)?;

        fs::write(
            today_dir.join("session.jsonl"),
            r#"{"timestamp":"2026-03-17T00:05:00+08:00","type":"turn_context","payload":{"model":"gpt-5.4"}}
{"timestamp":"2026-03-17T00:10:00+08:00","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":5,"reasoning_output_tokens":0,"total_tokens":105}}}}
{"timestamp":"2026-03-17T00:25:00+08:00","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":160,"cached_input_tokens":40,"output_tokens":15,"reasoning_output_tokens":0,"total_tokens":175}}}}
{"timestamp":"2026-03-17T00:35:00+08:00","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":220,"cached_input_tokens":50,"output_tokens":20,"reasoning_output_tokens":5,"total_tokens":245}}}}
"#,
        )?;

        let provider = CodexUsageProvider::new(root.clone());
        let usage = provider.collect_daily_usage(NaiveDate::from_ymd_opt(2026, 3, 17).unwrap())?;
        let model = &usage.model_breakdown[0];

        assert_eq!(model.usage_timeline.len(), 48);
        assert_eq!(model.usage_timeline[0].input_tokens, 160);
        assert_eq!(model.usage_timeline[0].cached_input_tokens, 40);
        assert_eq!(model.usage_timeline[0].output_tokens, 15);
        assert_eq!(model.usage_timeline[1].input_tokens, 60);
        assert_eq!(model.usage_timeline[1].cached_input_tokens, 10);
        assert_eq!(model.usage_timeline[1].output_tokens, 5);
        assert_eq!(model.usage_timeline[1].reasoning_output_tokens, 5);

        fs::remove_dir_all(root)?;
        Ok(())
    }
}
