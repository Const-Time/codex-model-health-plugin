use crate::invalid;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use croner::{
    Cron,
    parser::{CronParser, Seconds, Year},
};
use gateway_plugin_sdk::PluginFault;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    #[default]
    Interval,
    Daily,
    Cron,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Timing {
    pub mode: Mode,
    pub daily_time: String,
    pub cron: String,
    pub timezone: String,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            mode: Mode::Interval,
            daily_time: "09:00".into(),
            cron: "0 9 * * *".into(),
            timezone: "UTC".into(),
        }
    }
}

pub(crate) enum Prepared {
    Interval(u64),
    Calendar(Box<Cron>, Tz),
}

impl Timing {
    pub fn prepare(&self, interval_minutes: u64) -> Result<Prepared, PluginFault> {
        // Bound even inactive fields: configuration is persisted and can be enabled later.
        if !(60..=10080).contains(&interval_minutes)
            || self.cron.len() > 128
            || self.timezone.len() > 64
            || self.daily_time.len() > 5
        {
            return Err(invalid("间隔需为 60–10080 分钟，定时配置过长或无效"));
        }
        let timezone: Tz = self
            .timezone
            .parse()
            .map_err(|_| invalid("请选择有效的 IANA 时区，例如 Asia/Shanghai 或 UTC"))?;
        let expression = match self.mode {
            Mode::Interval => return Ok(Prepared::Interval(interval_minutes * 60_000)),
            Mode::Daily => {
                let bytes = self.daily_time.as_bytes();
                if bytes.len() != 5
                    || bytes[2] != b':'
                    || !bytes
                        .iter()
                        .enumerate()
                        .all(|(i, b)| i == 2 || b.is_ascii_digit())
                {
                    return Err(invalid("每日时间必须为 HH:mm，例如 09:30"));
                }
                let hour: u8 = self.daily_time[..2]
                    .parse()
                    .map_err(|_| invalid("小时无效"))?;
                let minute: u8 = self.daily_time[3..]
                    .parse()
                    .map_err(|_| invalid("分钟无效"))?;
                if hour > 23 || minute > 59 {
                    return Err(invalid("每日时间必须在 00:00–23:59 之间"));
                }
                format!("{minute} {hour} * * *")
            }
            Mode::Cron => {
                if self.cron.split_whitespace().count() != 5
                    || self.cron.chars().any(char::is_control)
                {
                    return Err(invalid("Cron 需要五段：分 时 日 月 周，不包含秒或年份"));
                }
                self.cron.clone()
            }
        };
        let cron = CronParser::builder()
            .seconds(Seconds::Disallowed)
            .year(Year::Disallowed)
            .build()
            .parse(&expression)
            .map_err(|_| invalid("Cron 表达式无效，请检查分、时、日、月、周的范围"))?;
        Ok(Prepared::Calendar(Box::new(cron), timezone))
    }
}

impl Prepared {
    pub fn next_after(&self, after: u64) -> Result<u64, PluginFault> {
        match self {
            Self::Interval(duration) => after
                .checked_add(*duration)
                .ok_or_else(|| invalid("执行时间超出范围")),
            Self::Calendar(cron, timezone) => {
                let timestamp = i64::try_from(after).map_err(|_| invalid("执行时间超出范围"))?;
                let start = DateTime::<Utc>::from_timestamp_millis(timestamp)
                    .ok_or_else(|| invalid("执行时间超出范围"))?
                    .with_timezone(timezone);
                let next = cron
                    .find_next_occurrence(&start, false)
                    .map_err(|_| invalid("计划没有可执行的未来时间，请检查日期组合"))?;
                let next = u64::try_from(next.timestamp_millis())
                    .map_err(|_| invalid("执行时间超出范围"))?;
                if next <= after {
                    return Err(invalid("计划未产生未来执行时间"));
                }
                Ok(next)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ms(time: &str) -> u64 {
        DateTime::parse_from_rfc3339(time)
            .unwrap()
            .timestamp_millis() as u64
    }
    fn cron(expression: &str, timezone: &str) -> Prepared {
        Timing {
            mode: Mode::Cron,
            cron: expression.into(),
            timezone: timezone.into(),
            ..Timing::default()
        }
        .prepare(360)
        .unwrap()
    }
    #[test]
    fn daily_timezone_rollover_and_exact_boundary() {
        let timing = Timing {
            mode: Mode::Daily,
            daily_time: "09:30".into(),
            timezone: "Asia/Shanghai".into(),
            ..Timing::default()
        }
        .prepare(360)
        .unwrap();
        assert_eq!(
            timing.next_after(ms("2026-10-25T01:29:59Z")).unwrap(),
            ms("2026-10-25T01:30:00Z")
        );
        assert_eq!(
            timing.next_after(ms("2026-10-25T01:30:00Z")).unwrap(),
            ms("2026-10-26T01:30:00Z")
        );
    }
    #[test]
    fn cron_lists_steps_weekdays_and_day_or_week_semantics() {
        assert_eq!(
            cron("0 9,18 * * 1-5", "UTC")
                .next_after(ms("2026-10-23T18:00:00Z"))
                .unwrap(),
            ms("2026-10-26T09:00:00Z")
        );
        assert_eq!(
            cron("*/15 * * * *", "UTC")
                .next_after(ms("2026-10-25T09:15:00Z"))
                .unwrap(),
            ms("2026-10-25T09:30:00Z")
        );
        assert_eq!(
            cron("0 9 1 * 1", "UTC")
                .next_after(ms("2026-10-25T00:00:00Z"))
                .unwrap(),
            ms("2026-10-26T09:00:00Z")
        );
        assert_eq!(
            cron("0 0 29 2 *", "UTC")
                .next_after(ms("2026-01-01T00:00:00Z"))
                .unwrap(),
            ms("2028-02-29T00:00:00Z")
        );
    }
    #[test]
    fn dst_gap_moves_to_first_valid_time_and_does_not_repeat_fixed_time() {
        let spring = cron("30 2 * * *", "America/New_York");
        assert_eq!(
            spring.next_after(ms("2026-03-08T06:59:00Z")).unwrap(),
            ms("2026-03-08T07:00:00Z")
        );
        let autumn = cron("30 1 * * *", "America/New_York");
        assert_eq!(
            autumn.next_after(ms("2026-11-01T05:30:00Z")).unwrap(),
            ms("2026-11-02T06:30:00Z")
        );
    }
    #[test]
    fn invalid_fields_and_impossible_calendar_are_rejected() {
        for expression in ["* * * * * *", "61 * * * *", "@daily", "*/0 * * * *"] {
            assert!(
                Timing {
                    mode: Mode::Cron,
                    cron: expression.into(),
                    ..Timing::default()
                }
                .prepare(360)
                .is_err()
            );
        }
        assert!(
            cron("0 0 31 2 *", "UTC")
                .next_after(ms("2026-01-01T00:00:00Z"))
                .is_err()
        );
        assert!(
            Timing {
                timezone: "bad/timezone".into(),
                ..Timing::default()
            }
            .prepare(360)
            .is_err()
        );
        assert!(
            Timing {
                mode: Mode::Daily,
                daily_time: "25:99".into(),
                ..Timing::default()
            }
            .prepare(360)
            .is_err()
        );
    }
}
