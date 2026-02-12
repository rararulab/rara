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

//! Internal bot routes for JD parse + persistence workflow.

use std::sync::Arc;

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use uuid::Uuid;

use crate::{repository::JobRepository, types::NormalizedJob};

#[derive(Clone)]
struct BotInternalState {
    ai_service: job_ai::service::AiService,
    job_repo:   Arc<dyn JobRepository>,
}

#[derive(Debug, serde::Deserialize)]
struct BotJdParseRequest {
    text: String,
}

#[derive(Debug, serde::Deserialize)]
struct ParsedJob {
    title:           String,
    company:         String,
    location:        Option<String>,
    description:     Option<String>,
    url:             Option<String>,
    salary_min:      Option<i32>,
    salary_max:      Option<i32>,
    salary_currency: Option<String>,
    tags:            Option<Vec<String>>,
}

#[derive(Debug, serde::Serialize)]
struct BotJdParseResponse {
    id:      Uuid,
    title:   String,
    company: String,
}

/// Build internal bot routes.
pub fn routes(
    ai_service: job_ai::service::AiService,
    job_repo: Arc<dyn JobRepository>,
) -> Router {
    Router::new()
        .route("/api/v1/internal/bot/jd-parse", post(parse_jd_from_bot))
        .with_state(BotInternalState {
            ai_service,
            job_repo,
        })
}

async fn parse_jd_from_bot(
    State(state): State<BotInternalState>,
    Json(req): Json<BotJdParseRequest>,
) -> Result<(StatusCode, Json<BotJdParseResponse>), (StatusCode, String)> {
    if req.text.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "text must not be empty".to_owned()));
    }

    let agent = state.ai_service.jd_parser().map_err(|e| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("ai service not available: {e}"),
        )
    })?;

    let json_str = agent.parse(&req.text).await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to parse jd via ai service: {e}"),
        )
    })?;

    let parsed: ParsedJob = serde_json::from_str(&json_str).map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to deserialize ai response: {e}"),
        )
    })?;

    let job = NormalizedJob {
        id:              Uuid::new_v4(),
        source_job_id:   Uuid::new_v4().to_string(),
        source_name:     "telegram".to_owned(),
        title:           parsed.title,
        company:         parsed.company,
        location:        parsed.location,
        description:     parsed.description,
        url:             parsed.url,
        salary_min:      parsed.salary_min,
        salary_max:      parsed.salary_max,
        salary_currency: parsed.salary_currency,
        tags:            parsed.tags.unwrap_or_default(),
        raw_data:        serde_json::to_value(&req.text).ok(),
        posted_at:       None,
    };

    let saved = state.job_repo.save(&job).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to save parsed jd job: {e}"),
        )
    })?;

    Ok((
        StatusCode::OK,
        Json(BotJdParseResponse {
            id:      saved.id,
            title:   saved.title,
            company: saved.company,
        }),
    ))
}
