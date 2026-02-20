
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};

use rara_mcp::manager::mgr::McpManager;

use crate::error::{McpAdminError, McpSnafu, RegistrySnafu, ServerNotFoundSnafu};
use crate::types::{
    CreateServerRequest, McpResourceView, McpServerConfigView, McpServerInfo, McpServerStatus,
    McpToolView, UpdateServerRequest,
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
        .route(
            "/api/v1/mcp/servers/{name}/tools",
            get(list_server_tools),
        )
        .route(
            "/api/v1/mcp/servers/{name}/resources",
            get(list_server_resources),
        )
        .with_state(manager)
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

    let connected = manager.connected_servers().await;

    let mut servers = Vec::with_capacity(names.len());
    for name in names {
        let config = registry.get(&name).await.map_err(|e| {
            RegistrySnafu {
                message: e.to_string(),
            }
            .build()
        })?;
        if let Some(config) = config {
            let status = if connected.contains(&name) {
                McpServerStatus::Connected
            } else {
                McpServerStatus::Disconnected
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

    let connected = manager.connected_servers().await;
    let status = if connected.contains(&name) {
        McpServerStatus::Connected
    } else {
        McpServerStatus::Disconnected
    };

    Ok(Json(McpServerInfo {
        name,
        config: McpServerConfigView::from(config),
        status,
    }))
}

async fn add_server(
    State(manager): State<McpState>,
    Json(req): Json<CreateServerRequest>,
) -> Result<Json<McpServerInfo>, McpAdminError> {
    manager
        .add_server(req.name.clone(), req.config.clone(), true)
        .await
        .map_err(|e| {
            McpSnafu {
                message: e.to_string(),
            }
            .build()
        })?;

    let connected = manager.connected_servers().await;
    let status = if connected.contains(&req.name) {
        McpServerStatus::Connected
    } else {
        McpServerStatus::Disconnected
    };

    Ok(Json(McpServerInfo {
        name:   req.name,
        config: McpServerConfigView::from(req.config),
        status,
    }))
}

async fn update_server(
    State(manager): State<McpState>,
    Path(name): Path<String>,
    Json(req): Json<UpdateServerRequest>,
) -> Result<Json<McpServerInfo>, McpAdminError> {
    let registry = manager.registry().await;
    if registry
        .get(&name)
        .await
        .map_err(|e| {
            RegistrySnafu {
                message: e.to_string(),
            }
            .build()
        })?
        .is_none()
    {
        return Err(ServerNotFoundSnafu { name }.build());
    }
    drop(registry);

    manager
        .update_server(&name, req.config.clone())
        .await
        .map_err(|e| {
            McpSnafu {
                message: e.to_string(),
            }
            .build()
        })?;

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

    let connected = manager.connected_servers().await;
    let status = if connected.contains(&name) {
        McpServerStatus::Connected
    } else {
        McpServerStatus::Disconnected
    };

    Ok(Json(McpServerInfo {
        name,
        config: McpServerConfigView::from(config),
        status,
    }))
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

    manager.start_server(&name, &config).await.map_err(|e| {
        McpSnafu {
            message: e.to_string(),
        }
        .build()
    })?;

    let status = McpServerStatus::Connected;

    Ok(Json(McpServerInfo {
        name,
        config: McpServerConfigView::from(config),
        status,
    }))
}

async fn stop_server(
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

    manager.stop_server(&name).await;

    Ok(Json(McpServerInfo {
        name,
        config: McpServerConfigView::from(config),
        status: McpServerStatus::Disconnected,
    }))
}

async fn restart_server(
    State(manager): State<McpState>,
    Path(name): Path<String>,
) -> Result<Json<McpServerInfo>, McpAdminError> {
    manager.restart_server(&name).await.map_err(|e| {
        McpSnafu {
            message: e.to_string(),
        }
        .build()
    })?;

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

    Ok(Json(McpServerInfo {
        name,
        config: McpServerConfigView::from(config),
        status: McpServerStatus::Connected,
    }))
}

async fn enable_server(
    State(manager): State<McpState>,
    Path(name): Path<String>,
) -> Result<Json<McpServerInfo>, McpAdminError> {
    let enabled = manager.enable_server(&name).await.map_err(|e| {
        McpSnafu {
            message: e.to_string(),
        }
        .build()
    })?;

    if !enabled {
        return Err(ServerNotFoundSnafu { name }.build());
    }

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

    let connected = manager.connected_servers().await;
    let status = if connected.contains(&name) {
        McpServerStatus::Connected
    } else {
        McpServerStatus::Disconnected
    };

    Ok(Json(McpServerInfo {
        name,
        config: McpServerConfigView::from(config),
        status,
    }))
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

    Ok(Json(McpServerInfo {
        name,
        config: McpServerConfigView::from(config),
        status: McpServerStatus::Disconnected,
    }))
}

async fn list_server_tools(
    State(manager): State<McpState>,
    Path(name): Path<String>,
) -> Result<Json<Vec<McpToolView>>, McpAdminError> {
    let tools = manager.list_tools(&name).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("not connected") {
            return ServerNotFoundSnafu { name: name.clone() }.build();
        }
        McpSnafu { message: msg }.build()
    })?;

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
    let result = manager.list_resources(&name).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("not connected") {
            return ServerNotFoundSnafu { name: name.clone() }.build();
        }
        McpSnafu { message: msg }.build()
    })?;

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
