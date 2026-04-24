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

//! Shared duration parsing for data-feed time-range queries.
//!
//! Both the in-process `query-feed` tool and the admin REST API accept
//! human-friendly duration strings like `"30m"` or `"7d"` to build a "since"
//! [`jiff::Timestamp`]. This module provides a single canonical parser so the
//! two surfaces cannot drift on unit coverage or error wording.

use jiff::{SignedDuration, Timestamp};

/// Parse a human-friendly duration string (e.g. `"30s"`, `"15m"`, `"1h"`,
/// `"7d"`) and return the timestamp that many units ago from now.
///
/// Supported units: `s` (seconds), `m` (minutes), `h` (hours), `d` (days).
///
/// Days are treated as fixed 86_400-second intervals — this avoids
/// `jiff::Timestamp`'s prohibition on calendar units and matches the
/// pragmatic "N days ago" meaning callers want for querying event windows.
pub fn parse_duration_ago(s: &str) -> anyhow::Result<Timestamp> {
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("empty duration string");
    }

    let (num_str, unit) = s.split_at(s.len() - 1);
    let n: i64 = num_str
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid number in duration: {s}"))?;

    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        _ => anyhow::bail!("unsupported duration unit '{unit}', expected s/m/h/d"),
    };

    let now = Timestamp::now();
    let past = now.checked_sub(SignedDuration::from_secs(secs))?;
    Ok(past)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_seconds() {
        let ts = parse_duration_ago("45s").unwrap();
        let now = Timestamp::now();
        let diff = now.duration_since(ts);
        assert!(
            diff.as_secs() >= 43 && diff.as_secs() <= 47,
            "expected ~45s, got {}s",
            diff.as_secs()
        );
    }

    #[test]
    fn parse_duration_minutes() {
        let ts = parse_duration_ago("30m").unwrap();
        let now = Timestamp::now();
        let diff = now.duration_since(ts);
        let expected = 30 * 60;
        assert!(
            diff.as_secs() >= expected - 2 && diff.as_secs() <= expected + 2,
            "expected ~{}s, got {}s",
            expected,
            diff.as_secs()
        );
    }

    #[test]
    fn parse_duration_hours() {
        let ts = parse_duration_ago("1h").unwrap();
        let now = Timestamp::now();
        let diff = now.duration_since(ts);
        assert!(
            diff.as_secs() >= 3598 && diff.as_secs() <= 3602,
            "expected ~3600s, got {}s",
            diff.as_secs()
        );
    }

    #[test]
    fn parse_duration_days() {
        let ts = parse_duration_ago("7d").unwrap();
        let now = Timestamp::now();
        let diff = now.duration_since(ts);
        let expected = 7 * 86400;
        assert!(
            diff.as_secs() >= expected - 2 && diff.as_secs() <= expected + 2,
            "expected ~{}s, got {}s",
            expected,
            diff.as_secs()
        );
    }

    #[test]
    fn parse_duration_empty() {
        let err = parse_duration_ago("").unwrap_err();
        assert!(err.to_string().contains("empty duration"), "got: {err}");
    }

    #[test]
    fn parse_duration_invalid_unit() {
        let err = parse_duration_ago("5x").unwrap_err();
        assert!(
            err.to_string().contains("unsupported duration unit"),
            "got: {err}"
        );
    }

    #[test]
    fn parse_duration_invalid_number() {
        let err = parse_duration_ago("abch").unwrap_err();
        assert!(
            err.to_string().contains("invalid number in duration"),
            "got: {err}"
        );
    }
}
