// Copyright 2025 Crrow
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

//! Conversion layer between DB models and domain types for analytics.

use chrono::{DateTime, Datelike as _, NaiveDate, TimeZone as _, Utc};
use jiff::Timestamp;
use jiff::civil::Date;

use crate::{db_models, types};

// ---------------------------------------------------------------------------
// Time helpers
// ---------------------------------------------------------------------------

fn chrono_to_timestamp(dt: DateTime<Utc>) -> Timestamp {
    Timestamp::new(dt.timestamp(), dt.timestamp_subsec_nanos() as i32)
        .expect("chrono DateTime<Utc> fits in jiff Timestamp")
}

fn timestamp_to_chrono(ts: Timestamp) -> DateTime<Utc> {
    let mut second = ts.as_second();
    let mut nanosecond = ts.subsec_nanosecond();
    if nanosecond < 0 {
        second = second.saturating_sub(1);
        nanosecond = nanosecond.saturating_add(1_000_000_000);
    }
    Utc.timestamp_opt(second, nanosecond as u32)
        .single()
        .expect("jiff Timestamp fits in chrono DateTime<Utc>")
}

fn naive_date_to_civil(nd: NaiveDate) -> Date {
    Date::new(nd.year() as i16, nd.month() as i8, nd.day() as i8)
        .expect("chrono NaiveDate fits in jiff Date")
}

fn civil_to_naive_date(d: Date) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year() as i32, d.month() as u32, d.day() as u32)
        .expect("jiff Date fits in chrono NaiveDate")
}

// ---------------------------------------------------------------------------
// Enum helpers
// ---------------------------------------------------------------------------

fn u8_from_i16(value: i16, field: &'static str) -> u8 {
    u8::try_from(value).unwrap_or_else(|_| panic!("invalid {field}: {value}"))
}

fn period_from_i16(value: i16) -> types::MetricsPeriod {
    let repr = u8_from_i16(value, "metrics.period");
    types::MetricsPeriod::from_repr(repr)
        .unwrap_or_else(|| panic!("invalid metrics.period: {value}"))
}

// ---------------------------------------------------------------------------
// MetricsSnapshot conversions
// ---------------------------------------------------------------------------

/// Store `MetricsSnapshot` -> Domain `MetricsSnapshot`.
impl From<db_models::MetricsSnapshot> for types::MetricsSnapshot {
    fn from(r: db_models::MetricsSnapshot) -> Self {
        Self {
            id:                   r.id,
            period:               period_from_i16(r.period),
            snapshot_date:        naive_date_to_civil(r.snapshot_date),
            jobs_discovered:      r.jobs_discovered,
            applications_sent:    r.applications_sent,
            interviews_scheduled: r.interviews_scheduled,
            offers_received:      r.offers_received,
            rejections:           r.rejections,
            ai_runs_count:        r.ai_runs_count,
            ai_total_cost_cents:  r.ai_total_cost_cents,
            extra:                r.extra,
            trace_id:             r.trace_id,
            created_at:           chrono_to_timestamp(r.created_at),
        }
    }
}

/// Domain `MetricsSnapshot` -> Store `MetricsSnapshot`.
impl From<types::MetricsSnapshot> for db_models::MetricsSnapshot {
    fn from(r: types::MetricsSnapshot) -> Self {
        Self {
            id:                   r.id,
            period:               r.period as u8 as i16,
            snapshot_date:        civil_to_naive_date(r.snapshot_date),
            jobs_discovered:      r.jobs_discovered,
            applications_sent:    r.applications_sent,
            interviews_scheduled: r.interviews_scheduled,
            offers_received:      r.offers_received,
            rejections:           r.rejections,
            ai_runs_count:        r.ai_runs_count,
            ai_total_cost_cents:  r.ai_total_cost_cents,
            extra:                r.extra,
            trace_id:             r.trace_id,
            created_at:           timestamp_to_chrono(r.created_at),
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn period_from_i16_works() {
        use types::MetricsPeriod as P;
        assert_eq!(period_from_i16(0), P::Daily);
        assert_eq!(period_from_i16(1), P::Weekly);
        assert_eq!(period_from_i16(2), P::Monthly);
    }

    #[test]
    fn metrics_snapshot_roundtrip() {
        let now = chrono::Utc::now();
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let id = Uuid::new_v4();

        let store = db_models::MetricsSnapshot {
            id,
            period: 0,
            snapshot_date: date,
            jobs_discovered: 10,
            applications_sent: 5,
            interviews_scheduled: 2,
            offers_received: 1,
            rejections: 3,
            ai_runs_count: 4,
            ai_total_cost_cents: 200,
            extra: None,
            trace_id: None,
            created_at: now,
        };

        let domain: types::MetricsSnapshot = store.into();
        assert_eq!(domain.id, id);
        assert_eq!(domain.period, types::MetricsPeriod::Daily);
        assert_eq!(domain.jobs_discovered, 10);

        let back: db_models::MetricsSnapshot = domain.into();
        assert_eq!(back.id, id);
        assert_eq!(back.period, 0);
    }
}
