use chrono::NaiveDate;

use crate::domain::DailyUsage;

pub mod claude;
pub mod codex;

pub trait UsageProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn collect_daily_usage(&self, date: NaiveDate) -> anyhow::Result<DailyUsage>;
}
