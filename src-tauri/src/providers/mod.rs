use chrono::NaiveDate;

use crate::domain::{DailyUsage, ProviderSettingsSummary};

pub mod claude;
pub mod codex;
pub mod kimi;

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
