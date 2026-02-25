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

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};
use tracing::instrument;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use super::{
    error::AnalyticsError,
    service::AnalyticsService,
    types::{CreateSnapshotRequest, MetricsPeriod, MetricsSnapshot, SnapshotFilter},
};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SnapshotListQuery {
    pub period:    Option<MetricsPeriod>,
    #[schema(value_type = Option<String>)]
    pub date_from: Option<jiff::civil::Date>,
    #[schema(value_type = Option<String>)]
    pub date_to:   Option<jiff::civil::Date>,
    pub limit:     Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LatestQuery {
    pub period: Option<MetricsPeriod>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DerivedRates {
    pub offer_rate:          Option<f64>,
    pub interview_rate:      Option<f64>,
    pub rejection_rate:      Option<f64>,
    pub avg_ai_cost_per_run: Option<f64>,
}

/// Register all analytics routes on a new router with shared state.
pub fn routes(service: AnalyticsService) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(create_snapshot, list_snapshots))
        .routes(routes!(get_latest))
        .routes(routes!(get_snapshot, delete_snapshot))
        .routes(routes!(get_derived_rates))
        .with_state(service)
}

/// Create a new metrics snapshot.
#[utoipa::path(
    post,
    path = "/api/v1/analytics/snapshots",
    tag = "analytics",
    request_body = CreateSnapshotRequest,
    responses(
        (status = 200, description = "Snapshot created", body = MetricsSnapshot),
    )
)]
#[instrument(skip(service, req))]
async fn create_snapshot(
    State(service): State<AnalyticsService>,
    Json(req): Json<CreateSnapshotRequest>,
) -> Result<Json<MetricsSnapshot>, AnalyticsError> {
    let snapshot = service.create_snapshot(req).await?;
    Ok(Json(snapshot))
}

/// List metrics snapshots with optional filters.
#[utoipa::path(
    get,
    path = "/api/v1/analytics/snapshots",
    tag = "analytics",
    params(
        ("period" = Option<MetricsPeriod>, Query, description = "Filter by aggregation period"),
        ("date_from" = Option<String>, Query, description = "Filter snapshots from this date"),
        ("date_to" = Option<String>, Query, description = "Filter snapshots up to this date"),
        ("limit" = Option<i64>, Query, description = "Maximum number of snapshots to return"),
    ),
    responses(
        (status = 200, description = "List of metrics snapshots", body = Vec<MetricsSnapshot>),
    )
)]
#[instrument(skip(service))]
async fn list_snapshots(
    State(service): State<AnalyticsService>,
    Query(query): Query<SnapshotListQuery>,
) -> Result<Json<Vec<MetricsSnapshot>>, AnalyticsError> {
    let filter = SnapshotFilter {
        period:    query.period,
        date_from: query.date_from,
        date_to:   query.date_to,
        limit:     query.limit,
    };
    let snapshots = service.list_snapshots(&filter).await?;
    Ok(Json(snapshots))
}

/// Get the latest metrics snapshot.
#[utoipa::path(
    get,
    path = "/api/v1/analytics/snapshots/latest",
    tag = "analytics",
    params(
        ("period" = Option<MetricsPeriod>, Query, description = "Aggregation period (defaults to daily)"),
    ),
    responses(
        (status = 200, description = "Latest snapshot", body = Option<MetricsSnapshot>),
    )
)]
#[instrument(skip(service))]
async fn get_latest(
    State(service): State<AnalyticsService>,
    Query(query): Query<LatestQuery>,
) -> Result<Json<Option<MetricsSnapshot>>, AnalyticsError> {
    let period = query.period.unwrap_or(MetricsPeriod::Daily);
    let snapshot = service.get_latest(period).await?;
    Ok(Json(snapshot))
}

/// Get a single metrics snapshot by ID.
#[utoipa::path(
    get,
    path = "/api/v1/analytics/snapshots/{id}",
    tag = "analytics",
    params(("id" = Uuid, Path, description = "Snapshot ID")),
    responses(
        (status = 200, description = "Snapshot found", body = MetricsSnapshot),
        (status = 404, description = "Snapshot not found"),
    )
)]
#[instrument(skip(service))]
async fn get_snapshot(
    State(service): State<AnalyticsService>,
    Path(id): Path<Uuid>,
) -> Result<Json<MetricsSnapshot>, AnalyticsError> {
    let snapshot = service.get_snapshot(id).await?;
    Ok(Json(snapshot))
}

/// Get derived rates for a metrics snapshot.
#[utoipa::path(
    get,
    path = "/api/v1/analytics/snapshots/{id}/rates",
    tag = "analytics",
    params(("id" = Uuid, Path, description = "Snapshot ID")),
    responses(
        (status = 200, description = "Derived rates", body = DerivedRates),
    )
)]
#[instrument(skip(service))]
async fn get_derived_rates(
    State(service): State<AnalyticsService>,
    Path(id): Path<Uuid>,
) -> Result<Json<DerivedRates>, AnalyticsError> {
    let snapshot = service.get_snapshot(id).await?;
    let rates = DerivedRates {
        offer_rate:          AnalyticsService::offer_rate(&snapshot),
        interview_rate:      AnalyticsService::interview_rate(&snapshot),
        rejection_rate:      AnalyticsService::rejection_rate(&snapshot),
        avg_ai_cost_per_run: AnalyticsService::avg_ai_cost_per_run(&snapshot),
    };
    Ok(Json(rates))
}

/// Delete a metrics snapshot.
#[utoipa::path(
    delete,
    path = "/api/v1/analytics/snapshots/{id}",
    tag = "analytics",
    params(("id" = Uuid, Path, description = "Snapshot ID")),
    responses(
        (status = 200, description = "Snapshot deleted"),
    )
)]
#[instrument(skip(service))]
async fn delete_snapshot(
    State(service): State<AnalyticsService>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, AnalyticsError> {
    service.delete_snapshot(id).await?;
    Ok(Json(()))
}
