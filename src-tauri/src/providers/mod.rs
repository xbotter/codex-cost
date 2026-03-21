use chrono::NaiveDate;

use crate::domain::{DailyUsage, ProviderSettingsSummary};

pub mod claude;
pub mod codex;
pub mod kimi;

pub trait UsageProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn settings_summary(&self) -> ProviderSettingsSummary;
    fn collect_daily_usage(&self, date: NaiveDate) -> anyhow::Result<DailyUsage>;
}
