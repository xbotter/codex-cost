use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDate, Timelike, Utc};
use serde::Deserialize;
use serde_json::Value;
use walkdir::WalkDir;

use crate::domain::{DailyUsage, ModelUsage, ProviderSettingsSummary, TokenUsage, UsageSnapshot};
use crate::providers::{
    load_file_signature, should_scan_file_for_date, FileParseCache, UsageProvider,
};

const DEFAULT_FALLBACK_MODEL: &str = "kimi-for-coding";

#[derive(Debug, Deserialize)]
struct KimiConfigFile {
    default_model: Option<String>,
    #[serde(default)]
    models: BTreeMap<String, KimiModelConfig>,
}

#[derive(Debug, Deserialize)]
struct KimiModelConfig {
    model: Option<String>,
}

pub struct KimiUsageProvider {
    root: PathBuf,
    config_path: PathBuf,
    parse_cache: FileParseCache<ParsedWireJsonl>,
}

impl KimiUsageProvider {
    pub fn new(root: PathBuf, config_path: PathBuf) -> Self {
        Self {
            root,
            config_path,
            parse_cache: FileParseCache::default(),
        }
    }

    pub fn default_root() -> Result<PathBuf> {
        let home = dirs::home_dir().context("home directory is unavailable")?;
        Ok(home.join(".kimi").join("sessions"))
    }

    pub fn default_config_path() -> Result<PathBuf> {
        let home = dirs::home_dir().context("home directory is unavailable")?;
        Ok(home.join(".kimi").join("config.toml"))
    }

    fn session_files(&self) -> impl Iterator<Item = PathBuf> + '_ {
        WalkDir::new(&self.root)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
            .filter_map(|entry| {
                let path = entry.into_path();
                (path.file_name().and_then(|value| value.to_str()) == Some("wire.jsonl"))
                    .then_some(path)
            })
    }

    fn default_model_name(&self) -> String {
        let Ok(content) = fs::read_to_string(&self.config_path) else {
            return DEFAULT_FALLBACK_MODEL.to_string();
        };

        let Ok(config) = toml::from_str::<KimiConfigFile>(&content) else {
            return DEFAULT_FALLBACK_MODEL.to_string();
        };

        let Some(default_model_key) = config.default_model.as_deref() else {
            return DEFAULT_FALLBACK_MODEL.to_string();
        };

        if let Some(model) = config
            .models
            .get(default_model_key)
            .and_then(|model| model.model.as_deref())
        {
            return model.to_string();
        }

        default_model_key
            .rsplit('/')
            .next()
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_FALLBACK_MODEL)
            .to_string()
    }
}

impl UsageProvider for KimiUsageProvider {
    fn id(&self) -> &'static str {
        "kimi"
    }

    fn settings_summary(&self) -> ProviderSettingsSummary {
        let has_local_data = self.root.exists() && self.session_files().next().is_some();
        ProviderSettingsSummary {
            id: self.id().to_string(),
            display_name: "Kimi Code".to_string(),
            description: "Read local Kimi Code session logs.".to_string(),
            status_label: if has_local_data {
                "Detected local sessions".to_string()
            } else {
                "No local data found".to_string()
            },
            has_local_data,
        }
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
        let mut skipped_log_lines = 0u64;
        let mut skipped_log_files = 0u64;
        let default_model_name = self.default_model_name();
        let today = Local::now().date_naive();
        let scope_key = format!("{}:{default_model_name}", date.format("%Y-%m-%d"));

        for path in self.session_files() {
            let signature = load_file_signature(&path);
            if !should_scan_file_for_date(
                signature.as_ref().map(|value| value.modified_at),
                date,
                today,
            ) {
                continue;
            }

            let parsed = self
                .parse_cache
                .get_or_try_parse(&path, &scope_key, signature, || {
                    let contents = fs::read_to_string(&path)
                        .with_context(|| format!("failed to read {}", path.display()))?;
                    parse_wire_jsonl(&contents, date, &default_model_name)
                })?;
            skipped_log_lines += parsed.skipped_line_count;
            if parsed.skipped_line_count > 0 {
                skipped_log_files += 1;
            }

            for snapshot in parsed.snapshots {
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
            skipped_log_lines,
            skipped_log_files,
        })
    }

    fn collect_daily_usage_series(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<Vec<DailyUsage>> {
        if !self.root.exists() || start > end {
            return Ok(Vec::new());
        }

        let mut per_day_model = BTreeMap::<NaiveDate, BTreeMap<String, TokenUsage>>::new();
        let default_model_name = self.default_model_name();

        for path in self.session_files() {
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;

            let parsed = parse_wire_jsonl_range(&contents, start, end, &default_model_name)?;
            for snapshot in parsed.snapshots {
                let date = local_date_from_rfc3339(&snapshot.timestamp)
                    .context("invalid usage timestamp")?;
                per_day_model
                    .entry(date)
                    .or_default()
                    .entry(snapshot.model_name)
                    .or_default()
                    .add_assign(&snapshot.usage);
            }
        }

        Ok(per_day_model
            .into_iter()
            .map(|(date, per_model)| build_daily_usage_without_timeline(self.id(), date, per_model))
            .collect())
    }
}

#[derive(Clone)]
struct ParsedWireJsonl {
    snapshots: Vec<UsageSnapshot>,
    skipped_line_count: u64,
}

fn parse_wire_jsonl(
    contents: &str,
    target_date: NaiveDate,
    default_model_name: &str,
) -> Result<ParsedWireJsonl> {
    parse_wire_jsonl_with_filter(contents, default_model_name, |date| date == target_date)
}

fn parse_wire_jsonl_range(
    contents: &str,
    start: NaiveDate,
    end: NaiveDate,
    default_model_name: &str,
) -> Result<ParsedWireJsonl> {
    parse_wire_jsonl_with_filter(contents, default_model_name, |date| {
        date >= start && date <= end
    })
}

fn parse_wire_jsonl_with_filter<F>(
    contents: &str,
    default_model_name: &str,
    include_date: F,
) -> Result<ParsedWireJsonl>
where
    F: Fn(NaiveDate) -> bool,
{
    let mut snapshots = Vec::new();
    let mut skipped_line_count = 0u64;
    let mut current_model = default_model_name.to_string();

    for line in contents.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => {
                skipped_line_count += 1;
                continue;
            }
        };

        if let Some(model_name) = extract_model_name(&value) {
            current_model = model_name;
        }

        let message = match value.get("message") {
            Some(message) => message,
            None => continue,
        };

        if message.get("type").and_then(Value::as_str) != Some("StatusUpdate") {
            continue;
        }

        let payload = match message.get("payload") {
            Some(payload) => payload,
            None => continue,
        };

        let token_usage = match payload.get("token_usage") {
            Some(token_usage) => token_usage,
            None => continue,
        };

        let timestamp = match value.get("timestamp").and_then(Value::as_f64) {
            Some(timestamp) => timestamp,
            None => {
                skipped_line_count += 1;
                continue;
            }
        };

        let Some(local_timestamp) = epoch_to_local_datetime(timestamp) else {
            skipped_line_count += 1;
            continue;
        };

        if !include_date(local_timestamp.date_naive()) {
            continue;
        }

        snapshots.push(UsageSnapshot {
            provider_id: "kimi".to_string(),
            model_name: current_model.clone(),
            timestamp: local_timestamp.to_rfc3339(),
            usage: TokenUsage {
                input_tokens: token_usage
                    .get("input_other")
                    .and_then(Value::as_u64)
                    .unwrap_or_default()
                    .saturating_add(
                        token_usage
                            .get("input_cache_read")
                            .and_then(Value::as_u64)
                            .unwrap_or_default(),
                    ),
                cached_input_tokens: token_usage
                    .get("input_cache_read")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                cache_creation_input_tokens: token_usage
                    .get("input_cache_creation")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                output_tokens: token_usage
                    .get("output")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                reasoning_output_tokens: 0,
            },
        });
    }

    Ok(ParsedWireJsonl {
        snapshots,
        skipped_line_count,
    })
}

fn extract_model_name(value: &Value) -> Option<String> {
    value
        .pointer("/message/payload/model")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .pointer("/message/payload/model_name")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            value
                .pointer("/message/payload/current_model")
                .and_then(Value::as_str)
        })
        .map(|model| model.to_string())
}

fn epoch_to_local_datetime(timestamp: f64) -> Option<DateTime<Local>> {
    if !timestamp.is_finite() {
        return None;
    }

    let millis = (timestamp * 1000.0).round() as i64;
    DateTime::<Utc>::from_timestamp_millis(millis).map(|value| value.with_timezone(&Local))
}

fn local_date_from_rfc3339(timestamp: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|value| value.with_timezone(&Local).date_naive())
}

fn half_hour_bucket_index(timestamp: &str) -> Option<usize> {
    let local = DateTime::parse_from_rfc3339(timestamp)
        .ok()?
        .with_timezone(&Local);
    Some((local.hour() as usize) * 2 + usize::from(local.minute() >= 30))
}

fn build_daily_usage_without_timeline(
    provider_id: &str,
    date: NaiveDate,
    per_model: BTreeMap<String, TokenUsage>,
) -> DailyUsage {
    let mut totals = TokenUsage::default();
    let mut model_breakdown: Vec<_> = per_model
        .into_iter()
        .map(|(model_name, usage)| {
            totals.add_assign(&usage);
            ModelUsage {
                model_name,
                usage,
                usage_timeline: vec![TokenUsage::default(); 48],
            }
        })
        .collect();
    model_breakdown.sort_by(|left, right| {
        Reverse(left.usage.total_tokens())
            .cmp(&Reverse(right.usage.total_tokens()))
            .then_with(|| {
                Reverse(left.usage.output_tokens).cmp(&Reverse(right.usage.output_tokens))
            })
            .then_with(|| Reverse(left.usage.input_tokens).cmp(&Reverse(right.usage.input_tokens)))
            .then_with(|| left.model_name.cmp(&right.model_name))
    });

    DailyUsage {
        provider_id: provider_id.to_string(),
        date: date.to_string(),
        model_breakdown,
        totals,
        skipped_log_lines: 0,
        skipped_log_files: 0,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;
    use chrono::{Local, NaiveDate, TimeZone};

    use super::KimiUsageProvider;
    use crate::providers::UsageProvider;

    #[test]
    fn collect_daily_usage_reads_status_update_token_usage() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "codex-cost-kimi-provider-test-{}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        let session_dir = root.join("sessions").join("workspace").join("session-1");
        fs::create_dir_all(&session_dir)?;
        let config_path = root.join("config.toml");
        fs::write(
            &config_path,
            r#"
default_model = "kimi-code/kimi-for-coding"

[models."kimi-code/kimi-for-coding"]
provider = "managed:kimi-code"
model = "kimi-for-coding"
"#,
        )?;

        let timestamp = Local
            .with_ymd_and_hms(2026, 3, 21, 9, 15, 0)
            .single()
            .expect("valid timestamp")
            .timestamp() as f64;
        fs::write(
            session_dir.join("wire.jsonl"),
            format!(
                r#"{{"timestamp": {timestamp}, "message": {{"type": "StatusUpdate", "payload": {{"token_usage": {{"input_other": 1000, "input_cache_read": 250, "input_cache_creation": 50, "output": 120}} }} }} }}
{{"timestamp": {timestamp}, "message": {{"type": "StatusUpdate", "payload": {{"token_usage": {{"input_other": 400, "input_cache_read": 50, "input_cache_creation": 10, "output": 80}} }} }} }}
"#
            ),
        )?;

        let provider = KimiUsageProvider::new(root.join("sessions"), config_path);
        let usage = provider.collect_daily_usage(NaiveDate::from_ymd_opt(2026, 3, 21).unwrap())?;

        assert_eq!(usage.provider_id, "kimi");
        assert_eq!(usage.model_breakdown.len(), 1);
        assert_eq!(usage.model_breakdown[0].model_name, "kimi-for-coding");
        assert_eq!(usage.totals.input_tokens, 1700);
        assert_eq!(usage.totals.cached_input_tokens, 300);
        assert_eq!(usage.totals.cache_creation_input_tokens, 60);
        assert_eq!(usage.totals.output_tokens, 200);

        fs::remove_dir_all(root)?;
        Ok(())
    }
}
