// Copyright 2025 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Configuration for the proactive event filter.
//!
//! Loaded from YAML; when absent, proactive signals are disabled entirely.
//! Do NOT derive `Default` — the absence of config means "feature off".

use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};
use tracing::warn;

/// Configuration for the proactive signal filter.
///
/// Controls quiet hours, per-signal cooldowns, global rate limiting,
/// and work-hour boundaries for time-based signals.
///
/// This struct intentionally does NOT derive `Default`. If the proactive
/// config section is missing from the YAML file, the feature is disabled.
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct ProactiveConfig {
    /// Quiet hours — suppress all proactive signals.
    ///
    /// Format: `["HH:MM", "HH:MM"]` (start, end). Handles midnight
    /// wrap-around (e.g. `["23:00", "08:00"]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_hours: Option<(String, String)>,

    /// Per-signal-kind minimum interval in seconds.
    ///
    /// Keys are signal kind names (e.g. `"session_idle"`, `"task_failed"`).
    /// Values are seconds.
    #[serde(default, deserialize_with = "deserialize_cooldowns")]
    pub cooldowns: HashMap<String, Duration>,

    /// Global rate limit — maximum signals per hour.
    pub max_hourly: u32,

    /// Work hours start time (e.g. `"09:00"`).
    pub work_hours_start: String,

    /// Work hours end time (e.g. `"18:00"`).
    pub work_hours_end: String,

    /// IANA timezone for time calculations (e.g. `"Asia/Shanghai"`).
    pub timezone: String,

    /// Idle threshold in seconds — sessions idle beyond this duration
    /// trigger a `SessionIdle` signal.
    pub idle_threshold_secs: u64,
}

impl ProactiveConfig {
    /// Parse `work_hours_start` as a `jiff::civil::Time`.
    pub fn parsed_work_start(&self) -> Option<jiff::civil::Time> {
        let result = parse_time_str(&self.work_hours_start);
        if result.is_none() {
            warn!(
                value = self.work_hours_start.as_str(),
                "proactive config: invalid work_hours_start, time events disabled"
            );
        }
        result
    }

    /// Parse `work_hours_end` as a `jiff::civil::Time`.
    pub fn parsed_work_end(&self) -> Option<jiff::civil::Time> {
        let result = parse_time_str(&self.work_hours_end);
        if result.is_none() {
            warn!(
                value = self.work_hours_end.as_str(),
                "proactive config: invalid work_hours_end, time events disabled"
            );
        }
        result
    }

    /// Parse quiet hours start as a `jiff::civil::Time`.
    pub fn parsed_quiet_start(&self) -> Option<jiff::civil::Time> {
        self.quiet_hours
            .as_ref()
            .and_then(|(s, _)| parse_time_str(s))
    }

    /// Parse quiet hours end as a `jiff::civil::Time`.
    pub fn parsed_quiet_end(&self) -> Option<jiff::civil::Time> {
        self.quiet_hours
            .as_ref()
            .and_then(|(_, e)| parse_time_str(e))
    }

    /// Parse the configured timezone as a `jiff::tz::TimeZone`.
    pub fn parsed_timezone(&self) -> Option<jiff::tz::TimeZone> {
        let result = jiff::tz::TimeZone::get(&self.timezone).ok();
        if result.is_none() {
            warn!(
                value = self.timezone.as_str(),
                "proactive config: invalid timezone, proactive features disabled"
            );
        }
        result
    }
}

/// Parse an "HH:MM" string into a `jiff::civil::Time`.
fn parse_time_str(s: &str) -> Option<jiff::civil::Time> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let hour: i8 = parts[0].parse().ok()?;
    let minute: i8 = parts[1].parse().ok()?;
    jiff::civil::Time::new(hour, minute, 0, 0).ok()
}

/// Deserialize cooldown values as seconds (u64 → Duration).
fn deserialize_cooldowns<'de, D>(deserializer: D) -> Result<HashMap<String, Duration>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: HashMap<String, u64> = HashMap::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|(k, secs)| (k, Duration::from_secs(secs)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_time_str_valid() {
        let t = parse_time_str("09:30").expect("should parse");
        assert_eq!(t.hour(), 9);
        assert_eq!(t.minute(), 30);
    }

    #[test]
    fn parse_time_str_invalid() {
        assert!(parse_time_str("invalid").is_none());
        assert!(parse_time_str("25:00").is_none());
    }

    #[test]
    fn deserialize_config_from_yaml() {
        let yaml = r#"
quiet_hours: ["23:00", "08:00"]
cooldowns:
  session_idle: 3600
  task_failed: 600
max_hourly: 5
work_hours_start: "09:00"
work_hours_end: "18:00"
timezone: "Asia/Shanghai"
idle_threshold_secs: 1800
"#;
        let config: ProactiveConfig = serde_yaml::from_str(yaml).expect("should parse");
        assert_eq!(config.max_hourly, 5);
        assert_eq!(
            config.cooldowns.get("session_idle"),
            Some(&Duration::from_secs(3600))
        );
        assert_eq!(
            config.cooldowns.get("task_failed"),
            Some(&Duration::from_secs(600))
        );
        assert!(config.parsed_work_start().is_some());
        assert!(config.parsed_quiet_start().is_some());
    }
}
