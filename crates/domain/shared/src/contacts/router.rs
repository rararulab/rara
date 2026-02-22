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

//! REST routes for telegram contacts management.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::contacts::{
    error::ContactError,
    repository::ContactRepository,
    types::{CreateContactRequest, TelegramContact, UpdateContactRequest},
};

#[derive(Clone)]
struct RouteState {
    repo: ContactRepository,
}

/// Build contacts CRUD routes.
pub fn routes(repo: ContactRepository) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_contacts))
        .routes(routes!(create_contact))
        .routes(routes!(update_contact))
        .routes(routes!(delete_contact))
        .with_state(RouteState { repo })
}

/// List all telegram contacts.
#[utoipa::path(
    get,
    path = "/api/v1/contacts",
    tag = "contacts",
    responses(
        (status = 200, description = "All contacts", body = Vec<TelegramContact>),
    )
)]
async fn list_contacts(
    State(state): State<RouteState>,
) -> Result<Json<Vec<TelegramContact>>, ContactError> {
    let contacts = state.repo.list().await?;
    Ok(Json(contacts))
}

/// Create a new telegram contact.
#[utoipa::path(
    post,
    path = "/api/v1/contacts",
    tag = "contacts",
    request_body = CreateContactRequest,
    responses(
        (status = 201, description = "Contact created", body = TelegramContact),
    )
)]
async fn create_contact(
    State(state): State<RouteState>,
    Json(req): Json<CreateContactRequest>,
) -> Result<(StatusCode, Json<TelegramContact>), ContactError> {
    let contact = state.repo.create(req).await?;
    Ok((StatusCode::CREATED, Json(contact)))
}

/// Update an existing telegram contact.
#[utoipa::path(
    put,
    path = "/api/v1/contacts/{id}",
    tag = "contacts",
    request_body = UpdateContactRequest,
    responses(
        (status = 200, description = "Contact updated", body = TelegramContact),
    )
)]
async fn update_contact(
    State(state): State<RouteState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateContactRequest>,
) -> Result<Json<TelegramContact>, ContactError> {
    let contact = state.repo.update(id, req).await?;
    Ok(Json(contact))
}

/// Delete a telegram contact.
#[utoipa::path(
    delete,
    path = "/api/v1/contacts/{id}",
    tag = "contacts",
    responses(
        (status = 204, description = "Contact deleted"),
    )
)]
async fn delete_contact(
    State(state): State<RouteState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ContactError> {
    state.repo.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
