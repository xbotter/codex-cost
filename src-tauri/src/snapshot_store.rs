use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use chrono::NaiveDate;

use crate::domain::AppSnapshot;

#[derive(Debug, Clone)]
pub struct SnapshotStore {
    root: PathBuf,
}

impl SnapshotStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn default_root() -> Result<PathBuf> {
        let base = dirs::cache_dir().context("cache directory is unavailable")?;
        Ok(base.join("codex-cost").join("daily-snapshots"))
    }

    pub fn load(&self, provider_id: &str, date: NaiveDate) -> Result<Option<AppSnapshot>> {
        let path = self.path_for(provider_id, date);
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()));
            }
        };

        let snapshot = serde_json::from_str(&content)
            .with_context(|| format!("failed to decode {}", path.display()))?;
        Ok(Some(snapshot))
    }

    pub fn load_range(
        &self,
        provider_id: &str,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<BTreeMap<NaiveDate, AppSnapshot>> {
        let mut cursor = start;
        let mut snapshots = BTreeMap::new();

        while cursor <= end {
            if let Some(snapshot) = self.load(provider_id, cursor)? {
                snapshots.insert(cursor, snapshot);
            }
            let Some(next) = cursor.succ_opt() else {
                break;
            };
            cursor = next;
        }

        Ok(snapshots)
    }

    pub fn save(&self, snapshot: &AppSnapshot) -> Result<()> {
        let date = NaiveDate::parse_from_str(&snapshot.date, "%Y-%m-%d")
            .with_context(|| format!("invalid snapshot date {}", snapshot.date))?;
        let path = self.path_for(&snapshot.provider_id, date);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let content =
            serde_json::to_vec_pretty(snapshot).context("failed to serialize daily snapshot")?;
        atomic_write(&path, &content)
    }

    fn path_for(&self, provider_id: &str, date: NaiveDate) -> PathBuf {
        self.root
            .join(provider_id)
            .join(format!("{}.json", date.format("%Y-%m-%d")))
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp_path = temp_path_for(path);
    fs::write(&temp_path, bytes)
        .with_context(|| format!("failed to write {}", temp_path.display()))?;

    if path.exists() {
        fs::remove_file(path).with_context(|| format!("failed to replace {}", path.display()))?;
    }

    fs::rename(&temp_path, path)
        .with_context(|| format!("failed to move {} into place", temp_path.display()))?;
    Ok(())
}

fn temp_path_for(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("snapshot");
    path.with_file_name(format!(".{file_name}.{nanos}.{nonce}.tmp"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;

    use super::SnapshotStore;
    use crate::domain::{AppSnapshot, TokenUsage};

    #[test]
    fn save_and_load_round_trip() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "codex-cost-snapshot-store-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let store = SnapshotStore::new(root.clone());
        let snapshot = AppSnapshot {
            provider_id: "codex".to_string(),
            enabled_provider_ids: vec!["codex".to_string()],
            date: "2026-03-22".to_string(),
            title: "$1.23".to_string(),
            tooltip: "Codex today".to_string(),
            total_cost_usd: 1.23,
            total_cost_sparkline: vec![0.0; 48],
            totals: TokenUsage::default(),
            model_costs: Vec::new(),
            pricing_updated_at: None,
            used_stale_pricing: false,
            last_refreshed_at: "2026-03-22T00:00:00Z".to_string(),
            quota: None,
            dashboard_always_on_top: false,
            warning: None,
            error_message: None,
        };

        store.save(&snapshot)?;
        let loaded = store
            .load(
                "codex",
                chrono::NaiveDate::from_ymd_opt(2026, 3, 22).expect("valid date"),
            )?
            .expect("snapshot should exist");

        assert_eq!(loaded.total_cost_usd, 1.23);
        assert_eq!(loaded.provider_id, "codex");

        fs::remove_dir_all(root)?;
        Ok(())
    }
}
