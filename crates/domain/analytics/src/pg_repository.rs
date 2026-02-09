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

//! PostgreSQL-backed implementation of [`crate::repository::AnalyticsRepository`].

use std::fmt::Write;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db_models;
use crate::error::{AnalyticsError, DuplicateSnapshotSnafu, NotFoundSnafu, RepositorySnafu};
use crate::types::{MetricsPeriod, MetricsSnapshot, SnapshotFilter};

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
        let store: db_models::MetricsSnapshot = snapshot.clone().into();

        let row = sqlx::query_as::<_, db_models::MetricsSnapshot>(
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
        let row = sqlx::query_as::<_, db_models::MetricsSnapshot>(
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

        let row = sqlx::query_as::<_, db_models::MetricsSnapshot>(
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

        let mut query = sqlx::query_as::<_, db_models::MetricsSnapshot>(&sql);

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

#[cfg(test)]
mod tests {
    use jiff::civil::Date;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    use super::*;
    use crate::repository::AnalyticsRepository;

    async fn setup_pool() -> (sqlx::PgPool, testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.unwrap();
        let host = container.get_host().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
        let pool = sqlx::PgPool::connect(&url).await.unwrap();

        // Bootstrap extensions and missing functions.
        sqlx::raw_sql(
            r#"
            CREATE EXTENSION IF NOT EXISTS pgcrypto;

            CREATE OR REPLACE FUNCTION set_updated_at()
            RETURNS TRIGGER AS $$
            BEGIN
                NEW.updated_at = now();
                RETURN NEW;
            END;
            $$ LANGUAGE plpgsql;
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        // Run all migrations in order.
        let migrations: &[&str] = &[
            include_str!("../../../job-model/migrations/20260127000000_init.sql"),
            include_str!("../../../job-model/migrations/20260208000000_domain_models.sql"),
            include_str!("../../../job-model/migrations/20260209000000_resume_version_mgmt.sql"),
            include_str!("../../../job-model/migrations/20260210000000_schema_alignment.sql"),
            include_str!("../../../job-model/migrations/20260211000000_notify_priority.sql"),
            include_str!("../../../job-model/migrations/20260211000001_scheduler_tables.sql"),
            include_str!("../../../job-model/migrations/20260212000000_application_int_enums.sql"),
            include_str!("../../../job-model/migrations/20260212000001_interview_int_enums.sql"),
            include_str!("../../../job-model/migrations/20260212000002_notify_int_enums.sql"),
            include_str!("../../../job-model/migrations/20260212000003_resume_int_enums.sql"),
            include_str!("../../../job-model/migrations/20260212000004_scheduler_int_enums.sql"),
            include_str!("../../../job-model/migrations/20260212000005_metrics_int_enums.sql"),
        ];

        for sql in migrations {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }

        (pool, container)
    }

    fn make_snapshot(period: MetricsPeriod, date: Date) -> MetricsSnapshot {
        MetricsSnapshot {
            id:                   Uuid::new_v4(),
            period,
            snapshot_date:        date,
            jobs_discovered:      10,
            applications_sent:    5,
            interviews_scheduled: 2,
            offers_received:      1,
            rejections:           3,
            ai_runs_count:        4,
            ai_total_cost_cents:  200,
            extra:                None,
            trace_id:             None,
            created_at:           jiff::Timestamp::now(),
        }
    }

    #[tokio::test]
    async fn save_and_find_by_id() {
        let (pool, _container) = setup_pool().await;
        let repo = PgAnalyticsRepository::new(pool);

        let date = Date::new(2026, 1, 15).unwrap();
        let snapshot = make_snapshot(MetricsPeriod::Daily, date);
        let id = snapshot.id;

        let saved = repo.save_snapshot(&snapshot).await.unwrap();
        assert_eq!(saved.id, id);
        assert_eq!(saved.jobs_discovered, 10);
        assert_eq!(saved.period, MetricsPeriod::Daily);

        let found = repo.find_by_id(id).await.unwrap().unwrap();
        assert_eq!(found.id, id);
        assert_eq!(found.snapshot_date, date);
    }

    #[tokio::test]
    async fn find_by_id_returns_none_for_missing() {
        let (pool, _container) = setup_pool().await;
        let repo = PgAnalyticsRepository::new(pool);

        let result = repo.find_by_id(Uuid::new_v4()).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_latest_returns_most_recent() {
        let (pool, _container) = setup_pool().await;
        let repo = PgAnalyticsRepository::new(pool);

        let older_date = Date::new(2026, 1, 10).unwrap();
        let newer_date = Date::new(2026, 1, 20).unwrap();

        let older = make_snapshot(MetricsPeriod::Daily, older_date);
        let mut newer = make_snapshot(MetricsPeriod::Daily, newer_date);
        newer.jobs_discovered = 99;

        repo.save_snapshot(&older).await.unwrap();
        repo.save_snapshot(&newer).await.unwrap();

        let latest = repo.get_latest(MetricsPeriod::Daily).await.unwrap().unwrap();
        assert_eq!(latest.snapshot_date, newer_date);
        assert_eq!(latest.jobs_discovered, 99);
    }

    #[tokio::test]
    async fn list_snapshots_with_period_filter() {
        let (pool, _container) = setup_pool().await;
        let repo = PgAnalyticsRepository::new(pool);

        let d1 = Date::new(2026, 1, 15).unwrap();
        let d2 = Date::new(2026, 1, 16).unwrap();
        let d3 = Date::new(2026, 1, 12).unwrap();

        repo.save_snapshot(&make_snapshot(MetricsPeriod::Daily, d1)).await.unwrap();
        repo.save_snapshot(&make_snapshot(MetricsPeriod::Daily, d2)).await.unwrap();
        repo.save_snapshot(&make_snapshot(MetricsPeriod::Weekly, d3)).await.unwrap();

        let filter = SnapshotFilter {
            period: Some(MetricsPeriod::Daily),
            ..Default::default()
        };
        let results = repo.list_snapshots(&filter).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].snapshot_date, d2);
    }

    #[tokio::test]
    async fn list_snapshots_with_date_range() {
        let (pool, _container) = setup_pool().await;
        let repo = PgAnalyticsRepository::new(pool);

        let d1 = Date::new(2026, 1, 10).unwrap();
        let d2 = Date::new(2026, 1, 15).unwrap();
        let d3 = Date::new(2026, 1, 20).unwrap();

        repo.save_snapshot(&make_snapshot(MetricsPeriod::Daily, d1)).await.unwrap();
        repo.save_snapshot(&make_snapshot(MetricsPeriod::Daily, d2)).await.unwrap();
        repo.save_snapshot(&make_snapshot(MetricsPeriod::Daily, d3)).await.unwrap();

        let filter = SnapshotFilter {
            date_from: Some(Date::new(2026, 1, 12).unwrap()),
            date_to:   Some(Date::new(2026, 1, 18).unwrap()),
            ..Default::default()
        };
        let results = repo.list_snapshots(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].snapshot_date, d2);
    }

    #[tokio::test]
    async fn delete_snapshot_removes_row() {
        let (pool, _container) = setup_pool().await;
        let repo = PgAnalyticsRepository::new(pool);

        let date = Date::new(2026, 1, 15).unwrap();
        let snapshot = make_snapshot(MetricsPeriod::Daily, date);
        let id = snapshot.id;

        repo.save_snapshot(&snapshot).await.unwrap();
        repo.delete_snapshot(id).await.unwrap();

        let found = repo.find_by_id(id).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn delete_snapshot_not_found() {
        let (pool, _container) = setup_pool().await;
        let repo = PgAnalyticsRepository::new(pool);

        let result = repo.delete_snapshot(Uuid::new_v4()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn duplicate_period_date_returns_error() {
        let (pool, _container) = setup_pool().await;
        let repo = PgAnalyticsRepository::new(pool);

        let date = Date::new(2026, 3, 1).unwrap();
        repo.save_snapshot(&make_snapshot(MetricsPeriod::Daily, date)).await.unwrap();

        let result = repo.save_snapshot(&make_snapshot(MetricsPeriod::Daily, date)).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AnalyticsError::DuplicateSnapshot { .. }));
    }
}
