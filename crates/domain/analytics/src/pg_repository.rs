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

//! PostgreSQL-backed implementation of
//! [`crate::repository::AnalyticsRepository`].

use std::fmt::Write;

use async_trait::async_trait;
use rara_model::metrics::MetricsSnapshot as StoreMetricsSnapshot;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    error::{AnalyticsError, DuplicateSnapshotSnafu, NotFoundSnafu, RepositorySnafu},
    types::{MetricsPeriod, MetricsSnapshot, SnapshotFilter},
};

/// PostgreSQL implementation of the analytics repository.
pub struct PgAnalyticsRepository {
    pool: PgPool,
}

impl PgAnalyticsRepository {
    /// Create a new repository backed by the given connection pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

/// Map a `sqlx::Error` into an `AnalyticsError`.
fn map_err(e: sqlx::Error) -> AnalyticsError {
    if let sqlx::Error::Database(ref db_err) = e {
        if db_err.code().as_deref() == Some("23505") {
            return DuplicateSnapshotSnafu {
                period: "unknown".to_owned(),
                date:   "unknown".to_owned(),
            }
            .build();
        }
    }
    RepositorySnafu {
        message: e.to_string(),
    }
    .build()
}

#[async_trait]
impl crate::repository::AnalyticsRepository for PgAnalyticsRepository {
    async fn save_snapshot(
        &self,
        snapshot: &MetricsSnapshot,
    ) -> Result<MetricsSnapshot, AnalyticsError> {
        let store: StoreMetricsSnapshot = snapshot.clone().into();

        let row = sqlx::query_as::<_, StoreMetricsSnapshot>(
            r#"INSERT INTO metrics_snapshot
               (id, period, snapshot_date, jobs_discovered, applications_sent,
                interviews_scheduled, offers_received, rejections,
                ai_runs_count, ai_total_cost_cents, extra, trace_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
               RETURNING *"#,
        )
        .bind(store.id)
        .bind(store.period)
        .bind(store.snapshot_date)
        .bind(store.jobs_discovered)
        .bind(store.applications_sent)
        .bind(store.interviews_scheduled)
        .bind(store.offers_received)
        .bind(store.rejections)
        .bind(store.ai_runs_count)
        .bind(store.ai_total_cost_cents)
        .bind(&store.extra)
        .bind(&store.trace_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err.code().as_deref() == Some("23505") {
                    return DuplicateSnapshotSnafu {
                        period: snapshot.period.to_string(),
                        date:   snapshot.snapshot_date.to_string(),
                    }
                    .build();
                }
            }
            RepositorySnafu {
                message: e.to_string(),
            }
            .build()
        })?;

        Ok(row.into())
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<MetricsSnapshot>, AnalyticsError> {
        let row = sqlx::query_as::<_, StoreMetricsSnapshot>(
            "SELECT * FROM metrics_snapshot WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(Into::into))
    }

    async fn get_latest(
        &self,
        period: MetricsPeriod,
    ) -> Result<Option<MetricsSnapshot>, AnalyticsError> {
        let period_i16 = period as u8 as i16;

        let row = sqlx::query_as::<_, StoreMetricsSnapshot>(
            r#"SELECT * FROM metrics_snapshot
               WHERE period = $1
               ORDER BY snapshot_date DESC
               LIMIT 1"#,
        )
        .bind(period_i16)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)?;

        Ok(row.map(Into::into))
    }

    async fn list_snapshots(
        &self,
        filter: &SnapshotFilter,
    ) -> Result<Vec<MetricsSnapshot>, AnalyticsError> {
        let mut sql = String::from("SELECT * FROM metrics_snapshot WHERE 1=1");
        let mut param_idx = 0u32;

        if filter.period.is_some() {
            param_idx += 1;
            let _ = write!(sql, " AND period = ${param_idx}");
        }

        if filter.date_from.is_some() {
            param_idx += 1;
            let _ = write!(sql, " AND snapshot_date >= ${param_idx}");
        }

        if filter.date_to.is_some() {
            param_idx += 1;
            let _ = write!(sql, " AND snapshot_date <= ${param_idx}");
        }

        sql.push_str(" ORDER BY snapshot_date DESC");

        if filter.limit.is_some() {
            param_idx += 1;
            let _ = write!(sql, " LIMIT ${param_idx}");
        }

        let mut query = sqlx::query_as::<_, StoreMetricsSnapshot>(&sql);

        if let Some(period) = filter.period {
            query = query.bind(period as u8 as i16);
        }

        if let Some(date_from) = filter.date_from {
            let nd = chrono::NaiveDate::from_ymd_opt(
                date_from.year() as i32,
                date_from.month() as u32,
                date_from.day() as u32,
            )
            .expect("jiff Date fits in NaiveDate");
            query = query.bind(nd);
        }

        if let Some(date_to) = filter.date_to {
            let nd = chrono::NaiveDate::from_ymd_opt(
                date_to.year() as i32,
                date_to.month() as u32,
                date_to.day() as u32,
            )
            .expect("jiff Date fits in NaiveDate");
            query = query.bind(nd);
        }

        if let Some(limit) = filter.limit {
            query = query.bind(limit);
        }

        let rows = query.fetch_all(&self.pool).await.map_err(map_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn delete_snapshot(&self, id: Uuid) -> Result<(), AnalyticsError> {
        let result = sqlx::query("DELETE FROM metrics_snapshot WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;

        if result.rows_affected() == 0 {
            return Err(NotFoundSnafu { id }.build());
        }
        Ok(())
    }
}
