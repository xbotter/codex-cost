use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::domain::{DashboardSettings, ProviderQuotaSettings, QuotaSettings};

const SUPPORTED_PROVIDER_IDS: [&str; 3] = ["codex", "claude", "kimi"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct AppConfig {
    #[serde(
        default = "default_provider_quota_settings",
        deserialize_with = "deserialize_provider_quota_settings"
    )]
    quota: ProviderQuotaSettings,
    #[serde(default)]
    dashboard: DashboardSettings,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            quota: default_provider_quota_settings(),
            dashboard: DashboardSettings::default(),
        }
    }
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

    pub fn load_quota_settings(&self, provider_id: &str) -> Result<QuotaSettings> {
        let config = self.load_config()?;
        quota_settings_for_provider(&config.quota, provider_id)
    }

    pub fn load_provider_quota_settings(&self) -> Result<ProviderQuotaSettings> {
        let config = self.load_config()?;
        Ok(config.quota)
    }

    pub fn save_provider_quota_settings(
        &self,
        settings: &ProviderQuotaSettings,
    ) -> Result<ProviderQuotaSettings> {
        let normalized = normalize_provider_quota_settings(settings)?;
        let mut config = self.load_config()?;
        config.quota = normalized.clone();
        self.persist_config(&config)?;
        Ok(normalized)
    }

    pub fn load_dashboard_settings(&self) -> Result<DashboardSettings> {
        let config = self.load_config()?;
        Ok(normalize_dashboard_settings(&config.dashboard))
    }

    pub fn save_dashboard_settings(
        &self,
        settings: &DashboardSettings,
    ) -> Result<DashboardSettings> {
        let normalized = validate_dashboard_settings(settings)?;
        let mut config = self.load_config()?;
        config.dashboard = normalized.clone();
        self.persist_config(&config)?;
        Ok(normalized)
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

pub fn default_provider_quota_settings() -> ProviderQuotaSettings {
    SUPPORTED_PROVIDER_IDS
        .into_iter()
        .map(|provider_id| (provider_id.to_string(), QuotaSettings::default()))
        .collect()
}

pub fn normalize_provider_quota_settings(
    settings: &ProviderQuotaSettings,
) -> Result<ProviderQuotaSettings> {
    let mut normalized = default_provider_quota_settings();
    for provider_id in SUPPORTED_PROVIDER_IDS {
        if let Some(provider_settings) = settings.get(provider_id) {
            normalized.insert(
                provider_id.to_string(),
                normalize_quota_settings(provider_settings)?,
            );
        }
    }
    Ok(normalized)
}

fn quota_settings_for_provider(
    settings: &ProviderQuotaSettings,
    provider_id: &str,
) -> Result<QuotaSettings> {
    validate_provider_id(provider_id)?;
    Ok(settings
        .get(provider_id)
        .cloned()
        .unwrap_or_else(QuotaSettings::default))
}

fn validate_provider_id(provider_id: &str) -> Result<()> {
    if SUPPORTED_PROVIDER_IDS.contains(&provider_id) {
        Ok(())
    } else {
        anyhow::bail!("unsupported provider: {provider_id}");
    }
}

fn deserialize_provider_quota_settings<'de, D>(
    deserializer: D,
) -> std::result::Result<ProviderQuotaSettings, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(default_provider_quota_settings());
    };

    match value {
        Value::Object(object)
            if object.contains_key("enabled")
                || object.contains_key("mode")
                || object.contains_key("amount_usd") =>
        {
            let legacy = serde_json::from_value::<QuotaSettings>(Value::Object(object))
                .map_err(D::Error::custom)?;
            let normalized = normalize_quota_settings(&legacy).map_err(D::Error::custom)?;
            Ok(SUPPORTED_PROVIDER_IDS
                .into_iter()
                .map(|provider_id| (provider_id.to_string(), normalized.clone()))
                .collect())
        }
        other => {
            let provider_settings =
                serde_json::from_value::<ProviderQuotaSettings>(other).map_err(D::Error::custom)?;
            normalize_provider_quota_settings(&provider_settings).map_err(D::Error::custom)
        }
    }
}

fn normalize_dashboard_settings(settings: &DashboardSettings) -> DashboardSettings {
    let supported_provider_order = ["codex", "claude", "kimi"];
    let mut enabled_providers = settings
        .enabled_providers
        .iter()
        .filter_map(|provider| {
            supported_provider_order
                .contains(&provider.as_str())
                .then_some(provider.clone())
        })
        .collect::<Vec<_>>();
    enabled_providers.sort_by_key(|provider| {
        supported_provider_order
            .iter()
            .position(|supported| supported == &provider.as_str())
            .unwrap_or(supported_provider_order.len())
    });
    enabled_providers.dedup();

    if enabled_providers.is_empty() {
        enabled_providers = DashboardSettings::default().enabled_providers;
    }

    let current_provider = if enabled_providers.contains(&settings.current_provider) {
        settings.current_provider.clone()
    } else {
        enabled_providers[0].clone()
    };

    DashboardSettings {
        always_on_top: settings.always_on_top,
        current_provider,
        enabled_providers,
    }
}

fn validate_dashboard_settings(settings: &DashboardSettings) -> Result<DashboardSettings> {
    let normalized = normalize_dashboard_settings(settings);
    if normalized.enabled_providers.is_empty() {
        anyhow::bail!("at least one provider must remain enabled");
    }
    if settings.enabled_providers.is_empty() {
        anyhow::bail!("at least one provider must remain enabled");
    }
    Ok(normalized)
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

    use super::{
        default_provider_quota_settings, normalize_provider_quota_settings,
        normalize_quota_settings, SettingsStore,
    };
    use crate::domain::{DashboardSettings, ProviderQuotaSettings, QuotaMode, QuotaSettings};

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
            .load_provider_quota_settings()
            .expect("missing config should default");

        assert_eq!(settings, default_provider_quota_settings());
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
        assert_eq!(settings.current_provider, "codex");
        assert_eq!(settings.enabled_providers, vec!["codex", "claude"]);
        fs::remove_dir_all(root).expect("temp root should clean up");
    }

    #[test]
    fn load_quota_settings_tolerates_partial_config() {
        let root = test_root("partial");
        let path = root.join("settings.json");
        fs::write(&path, "{}").expect("partial config should write");
        let store = SettingsStore::new(path);

        let settings = store
            .load_provider_quota_settings()
            .expect("partial config should decode");

        assert_eq!(settings, default_provider_quota_settings());
        fs::remove_dir_all(root).expect("temp root should clean up");
    }

    #[test]
    fn load_quota_settings_falls_back_when_json_is_invalid() {
        let root = test_root("invalid-json");
        let path = root.join("settings.json");
        fs::write(&path, "{\"quota\":").expect("invalid config should write");
        let store = SettingsStore::new(path);

        let settings = store
            .load_provider_quota_settings()
            .expect("invalid config should fall back");

        assert_eq!(settings, default_provider_quota_settings());
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
        let mut provider_settings = default_provider_quota_settings();
        provider_settings.insert("codex".to_string(), settings.clone());

        let saved = store
            .save_provider_quota_settings(&provider_settings)
            .expect("settings should save");

        assert_eq!(
            saved["codex"],
            QuotaSettings {
                enabled: true,
                mode: QuotaMode::Cap,
                amount_usd: 12.35,
            }
        );

        let persisted = store
            .load_quota_settings("codex")
            .expect("saved config should reload");
        assert_eq!(persisted, saved["codex"]);

        let raw = fs::read_to_string(path).expect("config should exist");
        assert!(raw.contains("\"amount_usd\": 12.35"));
        fs::remove_dir_all(root).expect("temp root should clean up");
    }

    #[test]
    fn load_quota_settings_migrates_legacy_global_quota_to_all_providers() {
        let root = test_root("legacy-quota");
        let path = root.join("settings.json");
        fs::write(
            &path,
            r#"{
  "quota": {
    "enabled": true,
    "mode": "cap",
    "amount_usd": 42.5
  }
}"#,
        )
        .expect("legacy config should write");
        let store = SettingsStore::new(path);

        let settings = store
            .load_provider_quota_settings()
            .expect("legacy quota should migrate");

        let expected = QuotaSettings {
            enabled: true,
            mode: QuotaMode::Cap,
            amount_usd: 42.5,
        };
        assert_eq!(settings.get("codex"), Some(&expected));
        assert_eq!(settings.get("claude"), Some(&expected));
        assert_eq!(settings.get("kimi"), Some(&expected));
        fs::remove_dir_all(root).expect("temp root should clean up");
    }

    #[test]
    fn normalize_provider_quota_settings_fills_missing_supported_providers() {
        let mut settings = ProviderQuotaSettings::new();
        settings.insert(
            "claude".to_string(),
            QuotaSettings {
                enabled: true,
                mode: QuotaMode::Target,
                amount_usd: 30.0,
            },
        );

        let normalized =
            normalize_provider_quota_settings(&settings).expect("provider quotas should normalize");

        assert_eq!(normalized["codex"], QuotaSettings::default());
        assert_eq!(normalized["kimi"], QuotaSettings::default());
        assert_eq!(
            normalized["claude"],
            QuotaSettings {
                enabled: true,
                mode: QuotaMode::Target,
                amount_usd: 30.0,
            }
        );
    }

    #[test]
    fn save_dashboard_settings_persists_always_on_top() {
        let root = test_root("save-dashboard");
        let path = root.join("settings.json");
        let store = SettingsStore::new(path.clone());
        let settings = DashboardSettings {
            always_on_top: true,
            current_provider: "claude".to_string(),
            enabled_providers: vec!["codex".to_string(), "claude".to_string()],
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
        assert!(raw.contains("\"current_provider\": \"claude\""));
        assert!(raw.contains("\"enabled_providers\": ["));
        fs::remove_dir_all(root).expect("temp root should clean up");
    }

    #[test]
    fn load_dashboard_settings_restores_at_least_one_enabled_provider() {
        let root = test_root("invalid-dashboard-providers");
        let path = root.join("settings.json");
        fs::write(
            &path,
            r#"{"dashboard":{"always_on_top":false,"current_provider":"claude","enabled_providers":[]}}"#,
        )
        .expect("invalid config should write");

        let store = SettingsStore::new(path);
        let settings = store
            .load_dashboard_settings()
            .expect("invalid dashboard config should normalize");

        assert_eq!(settings.current_provider, "claude");
        assert_eq!(settings.enabled_providers, vec!["codex", "claude"]);
        fs::remove_dir_all(root).expect("temp root should clean up");
    }

    #[test]
    fn save_dashboard_settings_rejects_empty_enabled_provider_list() {
        let root = test_root("reject-empty-dashboard-providers");
        let store = SettingsStore::new(root.join("settings.json"));

        let error = store
            .save_dashboard_settings(&DashboardSettings {
                always_on_top: false,
                current_provider: "codex".to_string(),
                enabled_providers: Vec::new(),
            })
            .expect_err("empty enabled providers should fail");

        assert!(error
            .to_string()
            .contains("at least one provider must remain enabled"));
        fs::remove_dir_all(root).expect("temp root should clean up");
    }
}
