use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use anyhow::Result;
use chrono::{DateTime, Local, NaiveDate};

use crate::domain::{DailyUsage, ProviderSettingsSummary};

pub mod claude;
pub mod codex;
pub mod kimi;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSignature {
    pub modified_at: SystemTime,
    pub len: u64,
}

#[derive(Debug)]
struct FileParseCacheEntry<T> {
    scope_key: String,
    signature: FileSignature,
    value: T,
}

#[derive(Debug)]
pub struct FileParseCache<T> {
    entries: Mutex<HashMap<PathBuf, FileParseCacheEntry<T>>>,
}

impl<T: Clone> FileParseCache<T> {
    pub fn get_or_try_parse<F>(
        &self,
        path: &Path,
        scope_key: &str,
        signature: Option<FileSignature>,
        parse: F,
    ) -> Result<T>
    where
        F: FnOnce() -> Result<T>,
    {
        let Some(signature) = signature else {
            return parse();
        };

        if let Some(value) = self.try_get(path, scope_key, &signature) {
            return Ok(value);
        }

        let value = parse()?;
        let mut entries = self.entries.lock().expect("file parse cache lock poisoned");
        entries.insert(
            path.to_path_buf(),
            FileParseCacheEntry {
                scope_key: scope_key.to_string(),
                signature,
                value: value.clone(),
            },
        );
        Ok(value)
    }

    fn try_get(&self, path: &Path, scope_key: &str, signature: &FileSignature) -> Option<T> {
        let entries = self.entries.lock().expect("file parse cache lock poisoned");
        let entry = entries.get(path)?;
        (entry.scope_key == scope_key && entry.signature == *signature).then(|| entry.value.clone())
    }
}

impl<T> Default for FileParseCache<T> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}

pub fn load_file_signature(path: &Path) -> Option<FileSignature> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(FileSignature {
        modified_at: metadata.modified().ok()?,
        len: metadata.len(),
    })
}

pub trait UsageProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn settings_summary(&self) -> ProviderSettingsSummary;
    fn collect_daily_usage(&self, date: NaiveDate) -> anyhow::Result<DailyUsage>;
    fn collect_daily_usage_series(
        &self,
        start: NaiveDate,
        end: NaiveDate,
    ) -> anyhow::Result<Vec<DailyUsage>> {
        let mut cursor = start;
        let mut days = Vec::new();

        while cursor <= end {
            days.push(self.collect_daily_usage(cursor)?);
            let Some(next) = cursor.succ_opt() else {
                break;
            };
            cursor = next;
        }

        Ok(days)
    }
}

pub fn should_scan_file_for_date(
    modified_at: Option<SystemTime>,
    target_date: NaiveDate,
    today: NaiveDate,
) -> bool {
    if target_date < today {
        return true;
    }

    let Some(modified_at) = modified_at else {
        return true;
    };

    DateTime::<Local>::from(modified_at).date_naive() >= target_date
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::{Local, TimeZone};

    use super::{should_scan_file_for_date, FileParseCache, FileSignature};

    #[test]
    fn today_only_scans_files_modified_today_or_later() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 3, 24).unwrap();
        let yesterday = chrono::NaiveDate::from_ymd_opt(2026, 3, 23).unwrap();
        let modified_at = Local
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .unwrap()
            .into();

        assert!(!should_scan_file_for_date(Some(modified_at), today, today));
        assert!(should_scan_file_for_date(None, today, today));
        assert!(should_scan_file_for_date(
            Some(modified_at),
            yesterday,
            today
        ));
    }

    #[test]
    fn file_parse_cache_reuses_value_when_signature_and_scope_match() -> anyhow::Result<()> {
        let cache = FileParseCache::default();
        let path = PathBuf::from("/tmp/session.jsonl");
        let signature = FileSignature {
            modified_at: Local
                .with_ymd_and_hms(2026, 3, 24, 12, 0, 0)
                .single()
                .unwrap()
                .into(),
            len: 128,
        };
        let parse_calls = AtomicUsize::new(0);

        let first = cache.get_or_try_parse(&path, "2026-03-24", Some(signature.clone()), || {
            parse_calls.fetch_add(1, Ordering::Relaxed);
            Ok::<_, anyhow::Error>("first parse".to_string())
        })?;
        let second = cache.get_or_try_parse(&path, "2026-03-24", Some(signature), || {
            parse_calls.fetch_add(1, Ordering::Relaxed);
            Ok::<_, anyhow::Error>("second parse".to_string())
        })?;

        assert_eq!(first, "first parse");
        assert_eq!(second, "first parse");
        assert_eq!(parse_calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[test]
    fn file_parse_cache_invalidates_on_scope_or_signature_change() -> anyhow::Result<()> {
        let cache = FileParseCache::default();
        let path = PathBuf::from("/tmp/session.jsonl");
        let initial_signature = FileSignature {
            modified_at: Local
                .with_ymd_and_hms(2026, 3, 24, 12, 0, 0)
                .single()
                .unwrap()
                .into(),
            len: 128,
        };
        let updated_signature = FileSignature {
            modified_at: Local
                .with_ymd_and_hms(2026, 3, 24, 12, 5, 0)
                .single()
                .unwrap()
                .into(),
            len: 256,
        };
        let parse_calls = AtomicUsize::new(0);

        let first = cache.get_or_try_parse(
            &path,
            "2026-03-24:codex",
            Some(initial_signature.clone()),
            || {
                parse_calls.fetch_add(1, Ordering::Relaxed);
                Ok::<_, anyhow::Error>("initial".to_string())
            },
        )?;
        let second =
            cache.get_or_try_parse(&path, "2026-03-24:kimi", Some(initial_signature), || {
                parse_calls.fetch_add(1, Ordering::Relaxed);
                Ok::<_, anyhow::Error>("scope changed".to_string())
            })?;
        let third =
            cache.get_or_try_parse(&path, "2026-03-24:kimi", Some(updated_signature), || {
                parse_calls.fetch_add(1, Ordering::Relaxed);
                Ok::<_, anyhow::Error>("signature changed".to_string())
            })?;

        assert_eq!(first, "initial");
        assert_eq!(second, "scope changed");
        assert_eq!(third, "signature changed");
        assert_eq!(parse_calls.load(Ordering::Relaxed), 3);
        Ok(())
    }

    #[test]
    fn file_parse_cache_skips_caching_without_signature() -> anyhow::Result<()> {
        let cache = FileParseCache::default();
        let path = PathBuf::from("/tmp/session.jsonl");
        let parse_calls = AtomicUsize::new(0);

        let first = cache.get_or_try_parse(&path, "2026-03-24", None, || {
            parse_calls.fetch_add(1, Ordering::Relaxed);
            Ok::<_, anyhow::Error>("first parse".to_string())
        })?;
        let second = cache.get_or_try_parse(&path, "2026-03-24", None, || {
            parse_calls.fetch_add(1, Ordering::Relaxed);
            Ok::<_, anyhow::Error>("second parse".to_string())
        })?;

        assert_eq!(first, "first parse");
        assert_eq!(second, "second parse");
        assert_eq!(parse_calls.load(Ordering::Relaxed), 2);
        Ok(())
    }
}
