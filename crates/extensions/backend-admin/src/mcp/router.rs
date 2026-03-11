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

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use rara_mcp::manager::{
    log_buffer::McpLogEntry,
    mgr::{ConnectionStatus, McpManager},
};

use super::{
    error::{McpAdminError, McpSnafu, RegistrySnafu, ServerNotFoundSnafu},
    types::{
        CreateServerRequest, McpResourceView, McpServerConfigView, McpServerInfo, McpServerStatus,
        McpToolView, UpdateServerRequest,
    },
};

type McpState = McpManager;

pub fn mcp_router(manager: McpState) -> Router {
    Router::new()
        .route("/api/v1/mcp/servers", get(list_servers).post(add_server))
        .route(
            "/api/v1/mcp/servers/{name}",
            get(get_server).put(update_server).delete(remove_server),
        )
        .route("/api/v1/mcp/servers/{name}/start", post(start_server))
        .route("/api/v1/mcp/servers/{name}/stop", post(stop_server))
        .route("/api/v1/mcp/servers/{name}/restart", post(restart_server))
        .route("/api/v1/mcp/servers/{name}/enable", post(enable_server))
        .route("/api/v1/mcp/servers/{name}/disable", post(disable_server))
        .route("/api/v1/mcp/servers/{name}/tools", get(list_server_tools))
        .route(
            "/api/v1/mcp/servers/{name}/resources",
            get(list_server_resources),
        )
        .route("/api/v1/mcp/servers/{name}/logs", get(list_server_logs))
        .route(
            "/api/v1/mcp/context-mode/status",
            get(context_mode_status),
        )
        .with_state(manager)
}

/// Build an [`McpServerInfo`] from the registry + connection state.
async fn build_server_info(
    manager: &McpManager,
    name: &str,
) -> Result<McpServerInfo, McpAdminError> {
    let registry = manager.registry().await;
    let config = registry
        .get(name)
        .await
        .map_err(|e| {
            RegistrySnafu {
                message: e.to_string(),
            }
            .build()
        })?
        .ok_or_else(|| {
            ServerNotFoundSnafu {
                name: name.to_string(),
            }
            .build()
        })?;
    let status = match manager.server_connection_status(name).await {
        ConnectionStatus::Connected => McpServerStatus::Connected,
        ConnectionStatus::Connecting => McpServerStatus::Connecting,
        ConnectionStatus::Disconnected => McpServerStatus::Disconnected,
    };
    Ok(McpServerInfo {
        name: name.to_string(),
        config: McpServerConfigView::from(config),
        status,
    })
}

async fn list_servers(
    State(manager): State<McpState>,
) -> Result<Json<Vec<McpServerInfo>>, McpAdminError> {
    let registry = manager.registry().await;
    let names = registry.list().await.map_err(|e| {
        RegistrySnafu {
            message: e.to_string(),
        }
        .build()
    })?;

    let mut servers = Vec::with_capacity(names.len());
    for name in names {
        let config = registry.get(&name).await.map_err(|e| {
            RegistrySnafu {
                message: e.to_string(),
            }
            .build()
        })?;
        if let Some(config) = config {
            let status = match manager.server_connection_status(&name).await {
                ConnectionStatus::Connected => McpServerStatus::Connected,
                ConnectionStatus::Connecting => McpServerStatus::Connecting,
                ConnectionStatus::Disconnected => McpServerStatus::Disconnected,
            };
            servers.push(McpServerInfo {
                name,
                config: McpServerConfigView::from(config),
                status,
            });
        }
    }

    Ok(Json(servers))
}

async fn get_server(
    State(manager): State<McpState>,
    Path(name): Path<String>,
) -> Result<Json<McpServerInfo>, McpAdminError> {
    Ok(Json(build_server_info(&manager, &name).await?))
}

async fn add_server(
    State(manager): State<McpState>,
    Json(req): Json<CreateServerRequest>,
) -> Result<Json<McpServerInfo>, McpAdminError> {
    // 1. Save to registry (fast, no handshake)
    let registry = manager.registry().await;
    registry
        .add(req.name.clone(), req.config.clone())
        .await
        .map_err(|e| {
            RegistrySnafu {
                message: e.to_string(),
            }
            .build()
        })?;
    drop(registry);

    // 2. Fire-and-forget start if enabled
    if req.config.enabled {
        let mgr = manager.clone();
        let name = req.name.clone();
        let config = req.config.clone();
        tokio::spawn(async move {
            if let Err(e) = mgr.start_server(&name, &config).await {
                tracing::warn!(server = %name, error = %e, "background MCP server start failed");
            }
        });
    }

    // 3. Return immediately with current status
    Ok(Json(build_server_info(&manager, &req.name).await?))
}

async fn update_server(
    State(manager): State<McpState>,
    Path(name): Path<String>,
    Json(req): Json<UpdateServerRequest>,
) -> Result<Json<McpServerInfo>, McpAdminError> {
    let registry = manager.registry().await;
    let existing = registry.get(&name).await.map_err(|e| {
        RegistrySnafu {
            message: e.to_string(),
        }
        .build()
    })?;
    if existing.is_none() {
        return Err(ServerNotFoundSnafu { name }.build());
    }

    // Preserve the enabled flag from existing config
    let enabled = existing.as_ref().is_none_or(|c| c.enabled);
    let mut new_config = req.config.clone();
    new_config.enabled = enabled;
    registry.add(name.clone(), new_config).await.map_err(|e| {
        RegistrySnafu {
            message: e.to_string(),
        }
        .build()
    })?;
    drop(registry);

    // If server was running (connected or connecting), spawn background restart
    let was_running =
        manager.server_connection_status(&name).await != ConnectionStatus::Disconnected;
    if was_running {
        let mgr = manager.clone();
        let restart_name = name.clone();
        tokio::spawn(async move {
            if let Err(e) = mgr.restart_server(&restart_name).await {
                tracing::warn!(server = %restart_name, error = %e, "background MCP server restart failed");
            }
        });
    }

    Ok(Json(build_server_info(&manager, &name).await?))
}

async fn remove_server(
    State(manager): State<McpState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, McpAdminError> {
    let removed = manager.remove_server(&name).await.map_err(|e| {
        McpSnafu {
            message: e.to_string(),
        }
        .build()
    })?;

    if !removed {
        return Err(ServerNotFoundSnafu { name }.build());
    }

    Ok(Json(serde_json::json!({ "removed": true })))
}

async fn start_server(
    State(manager): State<McpState>,
    Path(name): Path<String>,
) -> Result<Json<McpServerInfo>, McpAdminError> {
    let registry = manager.registry().await;
    let config = registry
        .get(&name)
        .await
        .map_err(|e| {
            RegistrySnafu {
                message: e.to_string(),
            }
            .build()
        })?
        .ok_or_else(|| ServerNotFoundSnafu { name: name.clone() }.build())?;
    drop(registry);

    // Fire-and-forget start
    let mgr = manager.clone();
    let start_name = name.clone();
    tokio::spawn(async move {
        if let Err(e) = mgr.start_server(&start_name, &config).await {
            tracing::warn!(server = %start_name, error = %e, "background MCP server start failed");
        }
    });

    Ok(Json(build_server_info(&manager, &name).await?))
}

async fn stop_server(
    State(manager): State<McpState>,
    Path(name): Path<String>,
) -> Result<Json<McpServerInfo>, McpAdminError> {
    // Verify server exists before stopping
    let info = build_server_info(&manager, &name).await?;
    manager.stop_server(&name).await;

    Ok(Json(McpServerInfo {
        status: McpServerStatus::Disconnected,
        ..info
    }))
}

async fn restart_server(
    State(manager): State<McpState>,
    Path(name): Path<String>,
) -> Result<Json<McpServerInfo>, McpAdminError> {
    // Verify server exists
    let _ = build_server_info(&manager, &name).await?;

    // Fire-and-forget restart
    let mgr = manager.clone();
    let restart_name = name.clone();
    tokio::spawn(async move {
        if let Err(e) = mgr.restart_server(&restart_name).await {
            tracing::warn!(server = %restart_name, error = %e, "background MCP server restart failed");
        }
    });

    Ok(Json(build_server_info(&manager, &name).await?))
}

async fn enable_server(
    State(manager): State<McpState>,
    Path(name): Path<String>,
) -> Result<Json<McpServerInfo>, McpAdminError> {
    // Enable in registry (fast)
    let registry = manager.registry().await;
    let enabled = registry.enable(&name).await.map_err(|e| {
        RegistrySnafu {
            message: e.to_string(),
        }
        .build()
    })?;
    drop(registry);

    if !enabled {
        return Err(ServerNotFoundSnafu { name }.build());
    }

    // Fire-and-forget start
    let mgr = manager.clone();
    let start_name = name.clone();
    tokio::spawn(async move {
        let registry = mgr.registry().await;
        if let Ok(Some(config)) = registry.get(&start_name).await {
            drop(registry);
            if let Err(e) = mgr.start_server(&start_name, &config).await {
                tracing::warn!(server = %start_name, error = %e, "background MCP server start failed");
            }
        }
    });

    Ok(Json(build_server_info(&manager, &name).await?))
}

async fn disable_server(
    State(manager): State<McpState>,
    Path(name): Path<String>,
) -> Result<Json<McpServerInfo>, McpAdminError> {
    let disabled = manager.disable_server(&name).await.map_err(|e| {
        McpSnafu {
            message: e.to_string(),
        }
        .build()
    })?;

    if !disabled {
        return Err(ServerNotFoundSnafu { name }.build());
    }

    Ok(Json(build_server_info(&manager, &name).await?))
}

async fn list_server_tools(
    State(manager): State<McpState>,
    Path(name): Path<String>,
) -> Result<Json<Vec<McpToolView>>, McpAdminError> {
    let tools = match manager.list_tools(&name).await {
        Ok(tools) => tools,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not connected") {
                return Err(ServerNotFoundSnafu { name }.build());
            }
            // Method not supported — return empty list
            if msg.contains("Method not found") || msg.contains("-32601") {
                return Ok(Json(vec![]));
            }
            return Err(McpSnafu { message: msg }.build());
        }
    };

    let views = tools
        .into_iter()
        .map(|t| McpToolView {
            name:         t.name.to_string(),
            description:  t.description.as_deref().map(str::to_owned),
            input_schema: serde_json::to_value(&*t.input_schema).unwrap_or_default(),
        })
        .collect();

    Ok(Json(views))
}

async fn list_server_resources(
    State(manager): State<McpState>,
    Path(name): Path<String>,
) -> Result<Json<Vec<McpResourceView>>, McpAdminError> {
    let result = match manager.list_resources(&name).await {
        Ok(result) => result,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not connected") {
                return Err(ServerNotFoundSnafu { name }.build());
            }
            // Method not supported — return empty list
            if msg.contains("Method not found") || msg.contains("-32601") {
                return Ok(Json(vec![]));
            }
            return Err(McpSnafu { message: msg }.build());
        }
    };

    let views = result
        .resources
        .into_iter()
        .map(|r| McpResourceView {
            uri:         r.raw.uri,
            name:        Some(r.raw.name),
            description: r.raw.description,
            mime_type:   r.raw.mime_type,
        })
        .collect();

    Ok(Json(views))
}

async fn list_server_logs(
    State(manager): State<McpState>,
    Path(name): Path<String>,
) -> Result<Json<Vec<McpLogEntry>>, McpAdminError> {
    let entries = manager.log_buffer().entries(&name).await;
    Ok(Json(entries))
}

async fn context_mode_status(
    State(manager): State<McpState>,
) -> Json<serde_json::Value> {
    let status = manager.server_connection_status("context-mode").await;
    let (status_str, interceptor_enabled) = match status {
        ConnectionStatus::Connected => ("connected", true),
        ConnectionStatus::Connecting => ("connecting", false),
        ConnectionStatus::Disconnected => ("disconnected", false),
    };
    let recent_logs = manager.log_buffer().entries("context-mode").await;
    let log_entries: Vec<serde_json::Value> = recent_logs
        .iter()
        .rev()
        .take(10)
        .map(|e| {
            serde_json::json!({
                "timestamp": e.timestamp,
                "level": e.level,
                "message": e.message,
            })
        })
        .collect();

    Json(serde_json::json!({
        "server": "context-mode",
        "connection_status": status_str,
        "interceptor_enabled": interceptor_enabled,
        "recent_logs": log_entries,
    }))
}
