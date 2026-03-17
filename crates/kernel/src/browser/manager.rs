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

//! Manages the Lightpanda browser process and CDP connection.

use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use chromiumoxide::{Page, browser::Browser};
use indexmap::IndexMap;
use serde::Deserialize;
use tokio::{
    process::{Child, Command},
    sync::RwLock,
};
use tracing::info;

use super::{error::*, ref_map::RefMap, snapshot};

/// Configuration for the browser subsystem.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct BrowserConfig {
    /// Path to the lightpanda binary. Defaults to `"lightpanda"` (PATH lookup).
    #[serde(default = "default_binary_path")]
    pub binary_path: PathBuf,

    /// Host for the CDP server. Defaults to `"127.0.0.1"`.
    #[serde(default = "default_host")]
    pub host: String,

    /// Port for the CDP server. Defaults to `9222`.
    #[serde(default = "default_port")]
    pub port: u16,

    /// Timeout for Lightpanda startup. Defaults to 10 seconds.
    #[serde(default = "default_startup_timeout_secs")]
    pub startup_timeout_secs: u64,

    /// Idle timeout before auto-closing a tab. Defaults to 300 seconds.
    #[serde(default = "default_page_idle_timeout_secs")]
    pub page_idle_timeout_secs: u64,

    /// Maximum snapshot size in bytes before truncation. Defaults to 50 KB.
    #[serde(default = "default_snapshot_max_bytes")]
    pub snapshot_max_bytes: usize,
}

fn default_binary_path() -> PathBuf { PathBuf::from("lightpanda") }
fn default_host() -> String { "127.0.0.1".to_string() }
fn default_port() -> u16 { 9222 }
fn default_startup_timeout_secs() -> u64 { 10 }
fn default_page_idle_timeout_secs() -> u64 { 300 }
fn default_snapshot_max_bytes() -> usize { 51200 }

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            binary_path:            default_binary_path(),
            host:                   default_host(),
            port:                   default_port(),
            startup_timeout_secs:   default_startup_timeout_secs(),
            page_idle_timeout_secs: default_page_idle_timeout_secs(),
            snapshot_max_bytes:     default_snapshot_max_bytes(),
        }
    }
}

/// Per-tab state tracked by the manager.
struct TabState {
    page:    Page,
    ref_map: RefMap,
}

/// Combined tab storage behind a single lock to prevent deadlocks.
///
/// All tab-related state lives here. Uses `IndexMap` so that iteration order
/// matches insertion order, making numeric indices stable across calls.
struct TabStore {
    /// Tabs in insertion order. Numeric indices come from this order.
    tabs:   IndexMap<String, TabState>,
    /// The "active" tab ID (most recently navigated/selected).
    active: Option<String>,
}

impl TabStore {
    fn new() -> Self {
        Self {
            tabs:   IndexMap::new(),
            active: None,
        }
    }
}

/// Kernel-level browser subsystem.
///
/// Manages a persistent Lightpanda CDP connection and provides high-level
/// methods for page navigation, interaction, and accessibility tree snapshots.
pub struct BrowserManager {
    process:  Option<Child>,
    browser:  Browser,
    /// Event handler task — must be kept alive for the browser to work.
    _handler: tokio::task::JoinHandle<()>,
    /// Single lock for all tab state — prevents deadlocks from split locks.
    store:    RwLock<TabStore>,
    config:   BrowserConfig,
}

/// Shared reference to the browser manager.
pub type BrowserManagerRef = Arc<BrowserManager>;

impl BrowserManager {
    /// Start Lightpanda and connect via CDP.
    ///
    /// # Panics
    ///
    /// Panics if the Lightpanda binary is not found or fails to start within
    /// the configured timeout. This is intentional — the browser subsystem
    /// is required for rara to function.
    pub async fn start(config: BrowserConfig) -> BrowserResult<Self> {
        // Verify binary exists by attempting to resolve via PATH.
        let binary_path = &config.binary_path;
        if !binary_path.is_absolute() {
            // For relative/bare names, check that spawning works below.
            // We defer the check to the spawn call itself.
        } else if !binary_path.exists() {
            return Err(BinaryNotFoundSnafu {
                path: binary_path.display().to_string(),
            }
            .build());
        }

        info!(binary = %binary_path.display(), host = %config.host, port = config.port, "starting lightpanda");

        // Spawn lightpanda serve process.
        let child = Command::new(binary_path)
            .args([
                "serve",
                "--host",
                &config.host,
                "--port",
                &config.port.to_string(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                CdpSnafu {
                    message: format!("failed to spawn lightpanda: {e}"),
                }
                .build()
            })?;

        // Wait for CDP to become ready.
        let ws_url = format!("ws://{}:{}", config.host, config.port);
        let timeout = Duration::from_secs(config.startup_timeout_secs);
        let start = std::time::Instant::now();

        let (browser, mut handler) = loop {
            if start.elapsed() > timeout {
                return Err(StartupTimeoutSnafu { timeout }.build());
            }
            match Browser::connect(&ws_url).await {
                Ok(pair) => break pair,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        };

        // Spawn the event handler — required for chromiumoxide to process
        // CDP events.
        let handler_task = tokio::spawn(async move {
            use futures::StreamExt;
            while handler.next().await.is_some() {}
        });

        info!("lightpanda CDP connection established");

        Ok(Self {
            process: Some(child),
            browser,
            _handler: handler_task,
            store: RwLock::new(TabStore::new()),
            config,
        })
    }

    /// Navigate to a URL.
    ///
    /// Reuses the active tab if one exists (preserving browser history), or
    /// creates a new tab if none is active. Returns page metadata and an
    /// accessibility snapshot.
    pub async fn navigate(&self, url: &str) -> BrowserResult<NavigateResult> {
        // Check if there is an active page we can reuse.
        let existing_page = {
            let store = self.store.read().await;
            store
                .active
                .as_ref()
                .and_then(|id| store.tabs.get(id))
                .map(|tab| (store.active.clone().unwrap(), tab.page.clone()))
        };

        let (tab_id, page) = if let Some((id, page)) = existing_page {
            // Reuse the active tab — navigate in-place to preserve history.
            page.goto(url).await.map_err(|e| {
                CdpSnafu {
                    message: e.to_string(),
                }
                .build()
            })?;
            (id, page)
        } else {
            // No active tab — create a new one.
            let page = self.browser.new_page(url).await.map_err(|e| {
                CdpSnafu {
                    message: e.to_string(),
                }
                .build()
            })?;
            let id = ulid::Ulid::new().to_string();
            (id, page)
        };

        let title = page
            .evaluate("document.title")
            .await
            .map_err(|e| {
                CdpSnafu {
                    message: e.to_string(),
                }
                .build()
            })?
            .into_value::<String>()
            .unwrap_or_default();

        let current_url = page
            .url()
            .await
            .map_err(|e| {
                CdpSnafu {
                    message: e.to_string(),
                }
                .build()
            })?
            .unwrap_or_else(|| url.to_string());

        let snap = snapshot::take_snapshot(&page, self.config.snapshot_max_bytes).await?;

        let tab_state = TabState {
            page,
            ref_map: snap.ref_map.clone(),
        };

        {
            let mut store = self.store.write().await;
            store.tabs.insert(tab_id.clone(), tab_state);
            store.active = Some(tab_id.clone());
        }

        Ok(NavigateResult {
            tab_id,
            url: current_url,
            title,
            snapshot: snap.text,
        })
    }

    /// Navigate back in the active tab.
    pub async fn navigate_back(&self) -> BrowserResult<String> {
        let page = self.active_page().await?;
        page.evaluate("window.history.back()").await.map_err(|e| {
            CdpSnafu {
                message: e.to_string(),
            }
            .build()
        })?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let snap = self.take_snapshot_active().await?;
        Ok(snap)
    }

    /// Take a fresh accessibility tree snapshot of the active page.
    ///
    /// Clones the `Page` handle out of the lock before doing async CDP I/O,
    /// then briefly re-acquires the write lock to update the ref map.
    pub async fn take_snapshot_active(&self) -> BrowserResult<String> {
        let (active_id, page) = {
            let store = self.store.read().await;
            let id = store
                .active
                .clone()
                .ok_or_else(|| NoActivePageSnafu.build())?;
            let page = store
                .tabs
                .get(&id)
                .map(|t| t.page.clone())
                .ok_or_else(|| NoActivePageSnafu.build())?;
            (id, page)
        };

        // Async CDP call happens outside any lock.
        let snap = snapshot::take_snapshot(&page, self.config.snapshot_max_bytes).await?;

        // Brief write lock to update the ref map only.
        {
            let mut store = self.store.write().await;
            if let Some(tab) = store.tabs.get_mut(&active_id) {
                tab.ref_map = snap.ref_map;
            }
        }

        Ok(snap.text)
    }

    /// Click an element by ref ID in the active tab.
    pub async fn click(&self, ref_id: &str) -> BrowserResult<String> {
        let (page, backend_node_id) = self.resolve_ref(ref_id).await?;
        // Resolve backend node to a remote object, then click.
        let node = page
            .execute(
                chromiumoxide::cdp::browser_protocol::dom::ResolveNodeParams::builder()
                    .backend_node_id(backend_node_id)
                    .build(),
            )
            .await
            .map_err(|e| {
                CdpSnafu {
                    message: e.to_string(),
                }
                .build()
            })?;

        let object_id = node.result.object.object_id.ok_or_else(|| {
            CdpSnafu {
                message: "element has no object_id".to_string(),
            }
            .build()
        })?;

        // Scroll element into view and compute click coordinates from the box
        // model center.
        use chromiumoxide::cdp::browser_protocol::dom;

        page.execute(
            dom::ScrollIntoViewIfNeededParams::builder()
                .object_id(object_id.clone())
                .build(),
        )
        .await
        .map_err(|e| {
            CdpSnafu {
                message: e.to_string(),
            }
            .build()
        })?;

        let box_model = page
            .execute(
                dom::GetBoxModelParams::builder()
                    .object_id(object_id)
                    .build(),
            )
            .await
            .map_err(|e| {
                CdpSnafu {
                    message: e.to_string(),
                }
                .build()
            })?;

        let content = box_model.result.model.content.inner();
        let cx = (content[0] + content[2] + content[4] + content[6]) / 4.0;
        let cy = (content[1] + content[3] + content[5] + content[7]) / 4.0;

        use chromiumoxide::cdp::browser_protocol::input::{
            DispatchMouseEventParams, DispatchMouseEventType, MouseButton,
        };

        page.execute(
            DispatchMouseEventParams::builder()
                .r#type(DispatchMouseEventType::MousePressed)
                .x(cx)
                .y(cy)
                .button(MouseButton::Left)
                .click_count(1)
                .build()
                .expect("valid mouse press params"),
        )
        .await
        .map_err(|e| {
            CdpSnafu {
                message: e.to_string(),
            }
            .build()
        })?;

        page.execute(
            DispatchMouseEventParams::builder()
                .r#type(DispatchMouseEventType::MouseReleased)
                .x(cx)
                .y(cy)
                .button(MouseButton::Left)
                .click_count(1)
                .build()
                .expect("valid mouse release params"),
        )
        .await
        .map_err(|e| {
            CdpSnafu {
                message: e.to_string(),
            }
            .build()
        })?;

        // Brief wait for any navigation/DOM update.
        tokio::time::sleep(Duration::from_millis(300)).await;

        self.take_snapshot_active().await
    }

    /// Type text into an element by ref ID.
    pub async fn type_text(&self, ref_id: &str, text: &str, submit: bool) -> BrowserResult<String> {
        let (page, backend_node_id) = self.resolve_ref(ref_id).await?;
        // Focus the element.
        page.execute(
            chromiumoxide::cdp::browser_protocol::dom::FocusParams::builder()
                .backend_node_id(backend_node_id)
                .build(),
        )
        .await
        .map_err(|e| {
            CdpSnafu {
                message: e.to_string(),
            }
            .build()
        })?;

        // Type each character.
        use chromiumoxide::cdp::browser_protocol::input::{
            DispatchKeyEventParams, DispatchKeyEventType,
        };
        for ch in text.chars() {
            page.execute(
                DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyDown)
                    .text(ch.to_string())
                    .build()
                    .expect("valid key event params"),
            )
            .await
            .map_err(|e| {
                CdpSnafu {
                    message: e.to_string(),
                }
                .build()
            })?;
        }

        if submit {
            page.execute(
                DispatchKeyEventParams::builder()
                    .r#type(DispatchKeyEventType::KeyDown)
                    .key("Enter".to_string())
                    .code("Enter".to_string())
                    .build()
                    .expect("valid key event params"),
            )
            .await
            .map_err(|e| {
                CdpSnafu {
                    message: e.to_string(),
                }
                .build()
            })?;

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        self.take_snapshot_active().await
    }

    /// Evaluate a JavaScript expression in the active page.
    pub async fn evaluate(&self, expression: &str) -> BrowserResult<serde_json::Value> {
        let page = self.active_page().await?;
        let result = page.evaluate(expression).await.map_err(|e| {
            EvaluationSnafu {
                message: e.to_string(),
            }
            .build()
        })?;

        Ok(result.into_value().unwrap_or(serde_json::Value::Null))
    }

    /// Press a key in the active page.
    pub async fn press_key(&self, key: &str) -> BrowserResult<()> {
        let page = self.active_page().await?;
        use chromiumoxide::cdp::browser_protocol::input::{
            DispatchKeyEventParams, DispatchKeyEventType,
        };
        page.execute(
            DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyDown)
                .key(key.to_string())
                .build()
                .expect("valid key event params"),
        )
        .await
        .map_err(|e| {
            CdpSnafu {
                message: e.to_string(),
            }
            .build()
        })?;

        page.execute(
            DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyUp)
                .key(key.to_string())
                .build()
                .expect("valid key event params"),
        )
        .await
        .map_err(|e| {
            CdpSnafu {
                message: e.to_string(),
            }
            .build()
        })?;

        Ok(())
    }

    /// Wait for text to appear or disappear, or for a timeout.
    pub async fn wait_for(
        &self,
        text: Option<&str>,
        text_gone: Option<&str>,
        time_secs: Option<f64>,
    ) -> BrowserResult<String> {
        if let Some(secs) = time_secs {
            tokio::time::sleep(Duration::from_secs_f64(secs)).await;
        }

        if let Some(target) = text {
            let page = self.active_page().await?;
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                let body: String = page
                    .evaluate("document.body?.innerText || ''")
                    .await
                    .map_err(|e| {
                        CdpSnafu {
                            message: e.to_string(),
                        }
                        .build()
                    })?
                    .into_value()
                    .unwrap_or_default();
                if body.contains(target) {
                    break;
                }
                if std::time::Instant::now() > deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }

        if let Some(target) = text_gone {
            let page = self.active_page().await?;
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                let body: String = page
                    .evaluate("document.body?.innerText || ''")
                    .await
                    .map_err(|e| {
                        CdpSnafu {
                            message: e.to_string(),
                        }
                        .build()
                    })?
                    .into_value()
                    .unwrap_or_default();
                if !body.contains(target) {
                    break;
                }
                if std::time::Instant::now() > deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }

        self.take_snapshot_active().await
    }

    /// List all open tabs.
    ///
    /// Indices are stable (insertion-ordered via `IndexMap`) and remain
    /// consistent between calls as long as no tabs are inserted or removed.
    pub async fn list_tabs(&self) -> Vec<TabInfo> {
        let store = self.store.read().await;
        store
            .tabs
            .keys()
            .enumerate()
            .map(|(i, id)| TabInfo {
                index:     i,
                tab_id:    id.clone(),
                is_active: store.active.as_deref() == Some(id),
            })
            .collect()
    }

    /// Switch to a tab by index.
    pub async fn select_tab(&self, index: usize) -> BrowserResult<()> {
        let mut store = self.store.write().await;
        let count = store.tabs.len();
        let tab_id = store
            .tabs
            .get_index(index)
            .map(|(id, _)| id.clone())
            .ok_or_else(|| TabIndexOutOfRangeSnafu { index, count }.build())?;
        store.active = Some(tab_id);
        Ok(())
    }

    /// Close a tab by index, or the active tab if no index given.
    ///
    /// Actually closes the underlying CDP page to free Lightpanda resources.
    pub async fn close_tab(&self, index: Option<usize>) -> BrowserResult<Vec<TabInfo>> {
        // Determine which tab to close and remove it, all under one lock.
        let removed_page = {
            let mut store = self.store.write().await;
            let tab_id = if let Some(idx) = index {
                let count = store.tabs.len();
                store
                    .tabs
                    .get_index(idx)
                    .map(|(id, _)| id.clone())
                    .ok_or_else(|| TabIndexOutOfRangeSnafu { index: idx, count }.build())?
            } else {
                store
                    .active
                    .clone()
                    .ok_or_else(|| NoActivePageSnafu.build())?
            };

            let tab_state = store.tabs.shift_remove(&tab_id);

            // Update active pointer if needed.
            if store.active.as_deref() == Some(&tab_id) {
                store.active = store.tabs.keys().next().cloned();
            }

            tab_state.map(|t| t.page)
        };

        // Close the CDP page outside the lock to avoid holding it during I/O.
        if let Some(page) = removed_page {
            let _ = page.close().await;
        }

        Ok(self.list_tabs().await)
    }

    /// Close all tabs and release their CDP pages.
    pub async fn close_all(&self) -> BrowserResult<Vec<TabInfo>> {
        let pages: Vec<Page> = {
            let mut store = self.store.write().await;
            let pages = store.tabs.drain(..).map(|(_, t)| t.page).collect();
            store.active = None;
            pages
        };

        // Close CDP pages outside the lock.
        for page in pages {
            let _ = page.close().await;
        }

        Ok(vec![])
    }

    // -- Private helpers --

    /// Get a clone of the active page (no lock held after return).
    async fn active_page(&self) -> BrowserResult<Page> {
        let store = self.store.read().await;
        let active_id = store
            .active
            .as_ref()
            .ok_or_else(|| NoActivePageSnafu.build())?;
        store
            .tabs
            .get(active_id)
            .map(|t| t.page.clone())
            .ok_or_else(|| NoActivePageSnafu.build())
    }

    /// Resolve a ref ID to a (Page, BackendNodeId) pair.
    async fn resolve_ref(
        &self,
        ref_id: &str,
    ) -> BrowserResult<(
        Page,
        chromiumoxide::cdp::browser_protocol::dom::BackendNodeId,
    )> {
        let store = self.store.read().await;
        let active_id = store
            .active
            .as_ref()
            .ok_or_else(|| NoActivePageSnafu.build())?;
        let tab = store
            .tabs
            .get(active_id)
            .ok_or_else(|| NoActivePageSnafu.build())?;
        let backend_id = tab.ref_map.resolve(ref_id)?;
        Ok((tab.page.clone(), backend_id))
    }
}

impl Drop for BrowserManager {
    fn drop(&mut self) {
        if let Some(mut child) = self.process.take() {
            // Best-effort kill — we're dropping, so can't await.
            let _ = child.start_kill();
        }
    }
}

/// Result of a navigation operation.
#[derive(Debug, Clone)]
pub struct NavigateResult {
    /// Unique identifier for the tab.
    pub tab_id:   String,
    /// The URL after navigation (may differ from requested due to redirects).
    pub url:      String,
    /// Page title.
    pub title:    String,
    /// Accessibility tree snapshot text.
    pub snapshot: String,
}

/// Basic info about an open tab.
#[derive(Debug, Clone)]
pub struct TabInfo {
    /// Tab position index (stable insertion order).
    pub index:     usize,
    /// Unique tab identifier.
    pub tab_id:    String,
    /// Whether this is the currently active tab.
    pub is_active: bool,
}
