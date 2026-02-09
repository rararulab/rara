//! HTTP API routes for analytics metrics snapshots.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::AnalyticsError,
    service::AnalyticsService,
    types::{CreateSnapshotRequest, MetricsPeriod, MetricsSnapshot, SnapshotFilter},
};

#[derive(Debug, Deserialize)]
pub struct SnapshotListQuery {
    pub period:    Option<MetricsPeriod>,
    pub date_from: Option<jiff::civil::Date>,
    pub date_to:   Option<jiff::civil::Date>,
    pub limit:     Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct LatestQuery {
    pub period: Option<MetricsPeriod>,
}

#[derive(Debug, Serialize)]
pub struct DerivedRates {
    pub offer_rate:          Option<f64>,
    pub interview_rate:      Option<f64>,
    pub rejection_rate:      Option<f64>,
    pub avg_ai_cost_per_run: Option<f64>,
}

/// Register all analytics routes on a new router with shared state.
pub fn routes(service: Arc<AnalyticsService>) -> Router {
    Router::new()
        .route("/api/v1/analytics/snapshots", post(create_snapshot))
        .route("/api/v1/analytics/snapshots", get(list_snapshots))
        .route(
            "/api/v1/analytics/snapshots/latest",
            get(get_latest),
        )
        .route(
            "/api/v1/analytics/snapshots/{id}",
            get(get_snapshot),
        )
        .route(
            "/api/v1/analytics/snapshots/{id}/rates",
            get(get_derived_rates),
        )
        .route(
            "/api/v1/analytics/snapshots/{id}",
            delete(delete_snapshot),
        )
        .with_state(service)
}

async fn create_snapshot(
    State(service): State<Arc<AnalyticsService>>,
    Json(req): Json<CreateSnapshotRequest>,
) -> Result<Json<MetricsSnapshot>, AnalyticsError> {
    let snapshot = service.create_snapshot(req).await?;
    Ok(Json(snapshot))
}

async fn list_snapshots(
    State(service): State<Arc<AnalyticsService>>,
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

async fn get_latest(
    State(service): State<Arc<AnalyticsService>>,
    Query(query): Query<LatestQuery>,
) -> Result<Json<Option<MetricsSnapshot>>, AnalyticsError> {
    let period = query.period.unwrap_or(MetricsPeriod::Daily);
    let snapshot = service.get_latest(period).await?;
    Ok(Json(snapshot))
}

async fn get_snapshot(
    State(service): State<Arc<AnalyticsService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<MetricsSnapshot>, AnalyticsError> {
    let snapshot = service.get_snapshot(id).await?;
    Ok(Json(snapshot))
}

async fn get_derived_rates(
    State(service): State<Arc<AnalyticsService>>,
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

async fn delete_snapshot(
    State(service): State<Arc<AnalyticsService>>,
    Path(id): Path<Uuid>,
) -> Result<Json<()>, AnalyticsError> {
    service.delete_snapshot(id).await?;
    Ok(Json(()))
}
