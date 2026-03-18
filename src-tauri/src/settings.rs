use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::domain::{DashboardSettings, QuotaSettings};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
struct AppConfig {
    #[serde(default)]
    quota: QuotaSettings,
    #[serde(default)]
    dashboard: DashboardSettings,
}

#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_config_path() -> Result<PathBuf> {
        let base = dirs::config_dir().context("config directory is unavailable")?;
        Ok(base.join("codex-cost").join("settings.json"))
    }

    pub fn load_quota_settings(&self) -> Result<QuotaSettings> {
        let config = self.load_config()?;
        Ok(config.quota)
    }

    pub fn save_quota_settings(&self, settings: &QuotaSettings) -> Result<QuotaSettings> {
        let normalized = normalize_quota_settings(settings)?;
        let mut config = self.load_config()?;
        config.quota = normalized.clone();
        self.persist_config(&config)?;
        Ok(normalized)
    }

    pub fn load_dashboard_settings(&self) -> Result<DashboardSettings> {
        let config = self.load_config()?;
        Ok(config.dashboard)
    }

    pub fn save_dashboard_settings(
        &self,
        settings: &DashboardSettings,
    ) -> Result<DashboardSettings> {
        let mut config = self.load_config()?;
        config.dashboard = settings.clone();
        self.persist_config(&config)?;
        Ok(settings.clone())
    }

    fn load_config(&self) -> Result<AppConfig> {
        let content = match fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(AppConfig::default())
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", self.path.display()))
            }
        };

        match serde_json::from_str::<AppConfig>(&content) {
            Ok(config) => Ok(config),
            Err(_) => Ok(AppConfig::default()),
        }
    }

    fn persist_config(&self, config: &AppConfig) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let content =
            serde_json::to_string_pretty(config).context("failed to serialize settings config")?;
        atomic_write(&self.path, content.as_bytes())
    }
}

pub fn normalize_quota_settings(settings: &QuotaSettings) -> Result<QuotaSettings> {
    if !settings.enabled {
        return Ok(QuotaSettings {
            enabled: false,
            mode: settings.mode.clone(),
            amount_usd: 0.0,
        });
    }

    if !settings.amount_usd.is_finite() {
        anyhow::bail!("quota amount must be a finite USD value");
    }

    let rounded = ((settings.amount_usd * 100.0) + 0.5).floor() / 100.0;
    if rounded <= 0.0 {
        anyhow::bail!("quota amount must round to at least 0.01 USD");
    }

    Ok(QuotaSettings {
        enabled: true,
        mode: settings.mode.clone(),
        amount_usd: rounded,
    })
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
        .unwrap_or_default()
        .as_nanos();
    path.with_extension(format!("tmp-{nanos}-{nonce}"))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::{normalize_quota_settings, SettingsStore};
    use crate::domain::{DashboardSettings, QuotaMode, QuotaSettings};

    fn test_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "codex-cost-settings-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("temp root should exist");
        root
    }

    #[test]
    fn load_quota_settings_defaults_when_file_is_missing() {
        let root = test_root("missing");
        let store = SettingsStore::new(root.join("settings.json"));

        let settings = store
            .load_quota_settings()
            .expect("missing config should default");

        assert_eq!(settings, QuotaSettings::default());
        fs::remove_dir_all(root).expect("temp root should clean up");
    }

    #[test]
    fn load_dashboard_settings_defaults_when_file_is_missing() {
        let root = test_root("missing-dashboard");
        let store = SettingsStore::new(root.join("settings.json"));

        let settings = store
            .load_dashboard_settings()
            .expect("missing config should default");

        assert_eq!(settings, DashboardSettings::default());
        fs::remove_dir_all(root).expect("temp root should clean up");
    }

    #[test]
    fn load_quota_settings_tolerates_partial_config() {
        let root = test_root("partial");
        let path = root.join("settings.json");
        fs::write(&path, "{}").expect("partial config should write");
        let store = SettingsStore::new(path);

        let settings = store
            .load_quota_settings()
            .expect("partial config should decode");

        assert_eq!(settings, QuotaSettings::default());
        fs::remove_dir_all(root).expect("temp root should clean up");
    }

    #[test]
    fn load_quota_settings_falls_back_when_json_is_invalid() {
        let root = test_root("invalid-json");
        let path = root.join("settings.json");
        fs::write(&path, "{\"quota\":").expect("invalid config should write");
        let store = SettingsStore::new(path);

        let settings = store
            .load_quota_settings()
            .expect("invalid config should fall back");

        assert_eq!(settings, QuotaSettings::default());
        fs::remove_dir_all(root).expect("temp root should clean up");
    }

    #[test]
    fn normalize_quota_settings_rejects_non_positive_rounded_values() {
        let invalid = QuotaSettings {
            enabled: true,
            mode: QuotaMode::Target,
            amount_usd: 0.004,
        };

        assert!(normalize_quota_settings(&invalid).is_err());
    }

    #[test]
    fn save_quota_settings_rounds_half_up_and_persists_json() {
        let root = test_root("save");
        let path = root.join("settings.json");
        let store = SettingsStore::new(path.clone());
        let settings = QuotaSettings {
            enabled: true,
            mode: QuotaMode::Cap,
            amount_usd: 12.345,
        };

        let saved = store
            .save_quota_settings(&settings)
            .expect("settings should save");

        assert_eq!(
            saved,
            QuotaSettings {
                enabled: true,
                mode: QuotaMode::Cap,
                amount_usd: 12.35,
            }
        );

        let persisted = store
            .load_quota_settings()
            .expect("saved config should reload");
        assert_eq!(persisted, saved);

        let raw = fs::read_to_string(path).expect("config should exist");
        assert!(raw.contains("\"amount_usd\": 12.35"));
        fs::remove_dir_all(root).expect("temp root should clean up");
    }

    #[test]
    fn save_dashboard_settings_persists_always_on_top() {
        let root = test_root("save-dashboard");
        let path = root.join("settings.json");
        let store = SettingsStore::new(path.clone());
        let settings = DashboardSettings {
            always_on_top: true,
        };

        let saved = store
            .save_dashboard_settings(&settings)
            .expect("dashboard settings should save");

        assert_eq!(saved, settings);

        let persisted = store
            .load_dashboard_settings()
            .expect("saved config should reload");
        assert_eq!(persisted, saved);

        let raw = fs::read_to_string(path).expect("config should exist");
        assert!(raw.contains("\"always_on_top\": true"));
        fs::remove_dir_all(root).expect("temp root should clean up");
    }
}
