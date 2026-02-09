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

//! HTTP API routes for analytics metrics snapshots.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, post},
};
use job_domain_analytics::{
    service::AnalyticsService,
    types::{CreateSnapshotRequest, MetricsPeriod, MetricsSnapshot, SnapshotFilter},
};
use job_domain_resume::repository::ResumeRepository;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{api::error::ApiError, state::AppState};

/// Register all analytics routes on a new router with shared state.
pub fn analytics_routes<R: ResumeRepository + 'static>(state: Arc<AppState<R>>) -> Router {
    Router::new()
        .route("/api/v1/analytics/snapshots", post(create_snapshot::<R>))
        .route("/api/v1/analytics/snapshots", get(list_snapshots::<R>))
        .route(
            "/api/v1/analytics/snapshots/latest",
            get(get_latest::<R>),
        )
        .route(
            "/api/v1/analytics/snapshots/{id}",
            get(get_snapshot::<R>),
        )
        .route(
            "/api/v1/analytics/snapshots/{id}/rates",
            get(get_derived_rates::<R>),
        )
        .route(
            "/api/v1/analytics/snapshots/{id}",
            delete(delete_snapshot::<R>),
        )
        .with_state(state)
}

/// Query parameters for listing snapshots.
#[derive(Debug, Deserialize)]
pub struct SnapshotListQuery {
    pub period:    Option<MetricsPeriod>,
    pub date_from: Option<jiff::civil::Date>,
    pub date_to:   Option<jiff::civil::Date>,
    pub limit:     Option<i64>,
}

/// Query parameters for getting the latest snapshot.
#[derive(Debug, Deserialize)]
pub struct LatestQuery {
    pub period: Option<MetricsPeriod>,
}

/// Response body for derived rates.
#[derive(Debug, Serialize)]
pub struct DerivedRates {
    pub offer_rate:           Option<f64>,
    pub interview_rate:       Option<f64>,
    pub rejection_rate:       Option<f64>,
    pub avg_ai_cost_per_run:  Option<f64>,
}

/// POST /api/v1/analytics/snapshots
async fn create_snapshot<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Json(req): Json<CreateSnapshotRequest>,
) -> Result<Json<MetricsSnapshot>, ApiError> {
    let snapshot = state.analytics_service.create_snapshot(req).await?;
    Ok(Json(snapshot))
}

/// GET /api/v1/analytics/snapshots
async fn list_snapshots<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Query(query): Query<SnapshotListQuery>,
) -> Result<Json<Vec<MetricsSnapshot>>, ApiError> {
    let filter = SnapshotFilter {
        period:    query.period,
        date_from: query.date_from,
        date_to:   query.date_to,
        limit:     query.limit,
    };
    let snapshots = state.analytics_service.list_snapshots(&filter).await?;
    Ok(Json(snapshots))
}

/// GET /api/v1/analytics/snapshots/latest
async fn get_latest<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Query(query): Query<LatestQuery>,
) -> Result<Json<Option<MetricsSnapshot>>, ApiError> {
    let period = query.period.unwrap_or(MetricsPeriod::Daily);
    let snapshot = state.analytics_service.get_latest(period).await?;
    Ok(Json(snapshot))
}

/// GET /api/v1/analytics/snapshots/:id
async fn get_snapshot<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<MetricsSnapshot>, ApiError> {
    let snapshot = state.analytics_service.get_snapshot(id).await?;
    Ok(Json(snapshot))
}

/// GET /api/v1/analytics/snapshots/:id/rates
async fn get_derived_rates<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<DerivedRates>, ApiError> {
    let snapshot = state.analytics_service.get_snapshot(id).await?;
    let rates = DerivedRates {
        offer_rate:          AnalyticsService::offer_rate(&snapshot),
        interview_rate:      AnalyticsService::interview_rate(&snapshot),
        rejection_rate:      AnalyticsService::rejection_rate(&snapshot),
        avg_ai_cost_per_run: AnalyticsService::avg_ai_cost_per_run(&snapshot),
    };
    Ok(Json(rates))
}

/// DELETE /api/v1/analytics/snapshots/:id
async fn delete_snapshot<R: ResumeRepository + 'static>(
    State(state): State<Arc<AppState<R>>>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, ApiError> {
    state.analytics_service.delete_snapshot(id).await?;
    Ok(Json(()))
}
