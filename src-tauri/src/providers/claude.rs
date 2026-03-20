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

const DEFAULT_FALLBACK_MODEL: &str = "claude";

struct ParsedProjectJsonl {
    snapshots: Vec<UsageSnapshot>,
    skipped_line_count: u64,
}

pub struct ClaudeUsageProvider {
    root: PathBuf,
}

impl ClaudeUsageProvider {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn default_root() -> Result<PathBuf> {
        let home = dirs::home_dir().context("home directory is unavailable")?;
        Ok(home.join(".claude").join("projects"))
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

impl UsageProvider for ClaudeUsageProvider {
    fn id(&self) -> &'static str {
        "claude"
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

        for path in self.session_files() {
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;

            let parsed = parse_project_jsonl(&path, &contents, date)?;
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
}

fn parse_project_jsonl(
    path: &Path,
    contents: &str,
    target_date: NaiveDate,
) -> Result<ParsedProjectJsonl> {
    let mut snapshots = BTreeMap::<String, UsageSnapshot>::new();
    let mut skipped_line_count = 0u64;

    for (index, line) in contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        let value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => {
                skipped_line_count += 1;
                continue;
            }
        };
        if value.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }

        let timestamp = match value.get("timestamp").and_then(Value::as_str) {
            Some(timestamp) if timestamp_matches_local_date(timestamp, target_date) => timestamp,
            _ => continue,
        };

        let message = match value.get("message") {
            Some(message) => message,
            None => continue,
        };

        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }

        let usage = match message.get("usage") {
            Some(usage) => usage,
            None => continue,
        };

        let input_tokens = usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let cache_read_input_tokens = usage
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let cache_creation_input_tokens = usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let output_tokens = usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let reasoning_output_tokens = usage
            .get("reasoning_output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default();

        let parsed_usage = TokenUsage {
            input_tokens: input_tokens.saturating_add(cache_read_input_tokens),
            cached_input_tokens: cache_read_input_tokens,
            cache_creation_input_tokens,
            output_tokens,
            reasoning_output_tokens,
        };

        let model_name = message
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_FALLBACK_MODEL)
            .to_string();

        let entry_key = format!(
            "{}:{}:{}:{}",
            path.display(),
            value
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            value
                .get("agentId")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            message
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| value.get("uuid").and_then(Value::as_str))
                .unwrap_or_else(|| "")
        );
        let fallback_entry_key = format!("{}:{index}:{timestamp}", path.display());
        let key = if entry_key.ends_with(":::") {
            fallback_entry_key
        } else {
            entry_key
        };

        snapshots
            .entry(key)
            .and_modify(|snapshot| {
                snapshot.model_name = model_name.clone();
                if timestamp > snapshot.timestamp.as_str() {
                    snapshot.timestamp = timestamp.to_string();
                }
                snapshot.usage.input_tokens =
                    snapshot.usage.input_tokens.max(parsed_usage.input_tokens);
                snapshot.usage.cached_input_tokens = snapshot
                    .usage
                    .cached_input_tokens
                    .max(parsed_usage.cached_input_tokens);
                snapshot.usage.cache_creation_input_tokens = snapshot
                    .usage
                    .cache_creation_input_tokens
                    .max(parsed_usage.cache_creation_input_tokens);
                snapshot.usage.output_tokens =
                    snapshot.usage.output_tokens.max(parsed_usage.output_tokens);
                snapshot.usage.reasoning_output_tokens = snapshot
                    .usage
                    .reasoning_output_tokens
                    .max(parsed_usage.reasoning_output_tokens);
            })
            .or_insert_with(|| UsageSnapshot {
                provider_id: "claude".to_string(),
                model_name,
                timestamp: timestamp.to_string(),
                usage: parsed_usage,
            });
    }

    Ok(ParsedProjectJsonl {
        snapshots: snapshots.into_values().collect(),
        skipped_line_count,
    })
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
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use anyhow::Result;
    use chrono::{DateTime, Local, NaiveDate, Timelike};

    use super::ClaudeUsageProvider;
    use crate::providers::UsageProvider;

    #[test]
    fn collect_daily_usage_deduplicates_same_assistant_message_and_includes_subagents() -> Result<()>
    {
        let root = std::env::temp_dir().join(format!(
            "codex-cost-claude-provider-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir_all(&root)?;

        let main_file = root.join("project-a").join("session.jsonl");
        let subagent_file = root
            .join("project-a")
            .join("subagents")
            .join("agent-1.jsonl");
        fs::create_dir_all(main_file.parent().expect("main file parent"))?;
        fs::create_dir_all(subagent_file.parent().expect("subagent file parent"))?;

        fs::write(
            &main_file,
            r#"{"type":"assistant","sessionId":"main","timestamp":"2026-03-20T01:00:00.000Z","message":{"id":"msg-1","role":"assistant","model":"claude-sonnet-4-20250514","usage":{"input_tokens":1000,"cache_creation_input_tokens":250,"cache_read_input_tokens":400,"output_tokens":80}}}
{"type":"assistant","sessionId":"main","timestamp":"2026-03-20T01:00:02.000Z","message":{"id":"msg-1","role":"assistant","model":"claude-sonnet-4-20250514","usage":{"input_tokens":1000,"cache_creation_input_tokens":250,"cache_read_input_tokens":400,"output_tokens":80}}}
{"type":"assistant","sessionId":"main","timestamp":"2026-03-20T01:10:00.000Z","message":{"id":"msg-2","role":"assistant","model":"claude-sonnet-4-20250514","usage":{"input_tokens":200,"cache_creation_input_tokens":0,"cache_read_input_tokens":50,"output_tokens":20}}}
"#,
        )?;

        fs::write(
            &subagent_file,
            r#"{"type":"assistant","sessionId":"main","agentId":"sub-1","timestamp":"2026-03-20T01:20:00.000Z","message":{"id":"sub-msg-1","role":"assistant","model":"claude-sonnet-4-20250514","usage":{"input_tokens":300,"cache_creation_input_tokens":100,"cache_read_input_tokens":25,"output_tokens":10}}}
"#,
        )?;

        let provider = ClaudeUsageProvider::new(root.clone());
        let usage = provider.collect_daily_usage(NaiveDate::from_ymd_opt(2026, 3, 20).unwrap())?;

        assert_eq!(usage.provider_id, "claude");
        assert_eq!(usage.model_breakdown.len(), 1);

        let model = &usage.model_breakdown[0];
        assert_eq!(model.model_name, "claude-sonnet-4-20250514");
        assert_eq!(model.usage.input_tokens, 1_975);
        assert_eq!(model.usage.cached_input_tokens, 475);
        assert_eq!(model.usage.cache_creation_input_tokens, 350);
        assert_eq!(model.usage.output_tokens, 110);
        assert_eq!(model.usage.reasoning_output_tokens, 0);
        assert_eq!(usage.totals, model.usage);
        assert_eq!(model.usage_timeline.len(), 48);
        let expected_bucket =
            DateTime::parse_from_rfc3339("2026-03-20T01:00:00.000Z")?.with_timezone(&Local);
        let bucket_index =
            (expected_bucket.hour() as usize) * 2 + usize::from(expected_bucket.minute() >= 30);
        assert_eq!(model.usage_timeline[bucket_index].input_tokens, 1_975);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn collect_daily_usage_skips_malformed_lines_and_keeps_valid_usage() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "codex-cost-claude-provider-malformed-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir_all(&root)?;

        let session_file = root.join("project-a").join("session.jsonl");
        fs::create_dir_all(session_file.parent().expect("session file parent"))?;
        fs::write(
            &session_file,
            concat!(
                "{\"type\":\"assistant\",\"sessionId\":\"main\",\"timestamp\":\"2026-03-20T01:00:00.000Z\",\"message\":{\"id\":\"msg-1\",\"role\":\"assistant\",\"model\":\"glm-5\",\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":20,\"output_tokens\":10}}}\n",
                "{\"type\":\"assistant\",\"sessionId\":\"main\",\"timestamp\":\"2026-03-20T01:05:00.000Z\",\"message\":{\"id\":\"msg-bad\",\"role\":\"assistant\",\"model\":\"glm-5\",\"usage\":{\"input_tokens\":0,\"output_tokens\":0}},\"type\":\"assista\0\0\0\0\0\n",
                "{\"type\":\"assistant\",\"sessionId\":\"main\",\"timestamp\":\"2026-03-20T01:10:00.000Z\",\"message\":{\"id\":\"msg-2\",\"role\":\"assistant\",\"model\":\"glm-5\",\"usage\":{\"input_tokens\":40,\"cache_read_input_tokens\":10,\"output_tokens\":5}}}\n"
            ),
        )?;

        let provider = ClaudeUsageProvider::new(root.clone());
        let usage = provider.collect_daily_usage(NaiveDate::from_ymd_opt(2026, 3, 20).unwrap())?;

        assert_eq!(usage.provider_id, "claude");
        assert_eq!(usage.model_breakdown.len(), 1);
        assert_eq!(usage.model_breakdown[0].usage.input_tokens, 170);
        assert_eq!(usage.model_breakdown[0].usage.cached_input_tokens, 30);
        assert_eq!(usage.model_breakdown[0].usage.output_tokens, 15);
        assert_eq!(usage.skipped_log_lines, 1);
        assert_eq!(usage.skipped_log_files, 1);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn collect_daily_usage_still_errors_when_a_jsonl_file_cannot_be_read() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "codex-cost-claude-provider-read-error-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir_all(&root)?;

        let session_file = root.join("project-a").join("session.jsonl");
        fs::create_dir_all(session_file.parent().expect("session file parent"))?;
        fs::write(&session_file, b"{\"type\":\"assistant\"}\n")?;

        let mut permissions = fs::metadata(&session_file)?.permissions();
        permissions.set_mode(0o000);
        fs::set_permissions(&session_file, permissions)?;

        let provider = ClaudeUsageProvider::new(root.clone());
        let error = provider
            .collect_daily_usage(NaiveDate::from_ymd_opt(2026, 3, 20).unwrap())
            .expect_err("read failures should still surface as provider errors");

        let mut restore_permissions = fs::metadata(&session_file)?.permissions();
        restore_permissions.set_mode(0o644);
        fs::set_permissions(&session_file, restore_permissions)?;
        fs::remove_dir_all(root)?;

        assert!(
            format!("{error:#}").contains("failed to read"),
            "unexpected error: {error:#}"
        );

        Ok(())
    }
}
