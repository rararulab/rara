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

//! Model catalog — dynamic model list with caching.
//!
//! [`ModelCatalog`] fetches available models from the configured LLM provider
//! via [`LlmModelListerRef`] and caches them for a configurable TTL. When the
//! provider is unavailable it falls back to a hand-picked `CURATED_MODELS`
//! list.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use rara_kernel::llm::{LlmModelListerRef, ModelCapabilities};
use serde::Serialize;
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Cache time-to-live — 5 minutes.
const CACHE_TTL: Duration = Duration::from_mins(5);

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single model entry returned by the `GET /api/v1/chat/models` endpoint.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ChatModel {
    /// OpenRouter model identifier (e.g. `"openai/gpt-4o"`).
    pub id:              String,
    /// Human-friendly display name.
    pub name:            String,
    /// Maximum context window in tokens.
    pub context_length:  u32,
    /// Whether the user has pinned this model as a favorite.
    pub is_favorite:     bool,
    /// Whether the model accepts image input. Surfaced so the frontend can
    /// pre-flight image attachments and refuse to send to a text-only model,
    /// instead of letting the kernel silently drop the image block at
    /// request build time.
    pub supports_vision: bool,
}

// ---------------------------------------------------------------------------
// Curated fallback list
// ---------------------------------------------------------------------------

struct CuratedModel {
    id:             &'static str,
    name:           &'static str,
    context_length: u32,
}

const CURATED_MODELS: &[CuratedModel] = &[
    CuratedModel {
        id:             "openai/gpt-4o",
        name:           "GPT-4o",
        context_length: 128_000,
    },
    CuratedModel {
        id:             "openai/gpt-4o-mini",
        name:           "GPT-4o Mini",
        context_length: 128_000,
    },
    CuratedModel {
        id:             "openai/gpt-4.1",
        name:           "GPT-4.1",
        context_length: 1_047_576,
    },
    CuratedModel {
        id:             "openai/o3-mini",
        name:           "o3 Mini",
        context_length: 200_000,
    },
    CuratedModel {
        id:             "anthropic/claude-sonnet-4",
        name:           "Claude Sonnet 4",
        context_length: 200_000,
    },
    CuratedModel {
        id:             "anthropic/claude-3.5-haiku",
        name:           "Claude 3.5 Haiku",
        context_length: 200_000,
    },
    CuratedModel {
        id:             "google/gemini-2.5-pro-preview",
        name:           "Gemini 2.5 Pro",
        context_length: 1_048_576,
    },
    CuratedModel {
        id:             "google/gemini-2.5-flash-preview",
        name:           "Gemini 2.5 Flash",
        context_length: 1_048_576,
    },
    CuratedModel {
        id:             "deepseek/deepseek-chat-v3-0324:free",
        name:           "DeepSeek V3 (Free)",
        context_length: 131_072,
    },
    CuratedModel {
        id:             "meta-llama/llama-4-maverick:free",
        name:           "Llama 4 Maverick (Free)",
        context_length: 1_048_576,
    },
];

fn curated_fallback(favorite_ids: &[String]) -> Vec<ChatModel> {
    let mut models: Vec<ChatModel> = CURATED_MODELS
        .iter()
        .map(|m| ChatModel {
            id:              m.id.to_owned(),
            name:            m.name.to_owned(),
            context_length:  m.context_length,
            is_favorite:     favorite_ids.iter().any(|f| f == m.id),
            supports_vision: ModelCapabilities::detect(None, m.id).supports_vision,
        })
        .collect();
    apply_favorite_sort(&mut models);
    models
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

struct CacheEntry {
    fetched_at: Instant,
    models:     Vec<RawModel>,
}

/// Intermediate model representation stored in the cache (without favorite
/// flag, which is applied at query time).
#[derive(Clone)]
struct RawModel {
    id:              String,
    name:            String,
    context_length:  u32,
    supports_vision: bool,
}

// ---------------------------------------------------------------------------
// ModelCatalog
// ---------------------------------------------------------------------------

/// Fetches and caches the model list from the configured LLM provider.
///
/// Thread-safe and cheaply cloneable.
#[derive(Clone)]
pub struct ModelCatalog {
    model_lister: LlmModelListerRef,
    cache:        Arc<Mutex<Option<CacheEntry>>>,
}

impl ModelCatalog {
    /// Create a new catalog backed by the given model lister.
    pub fn new(model_lister: LlmModelListerRef) -> Self {
        Self {
            model_lister,
            cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Look up a model's context length from the curated list.
    ///
    /// Returns `None` if the model is not in the curated list. Callers
    /// should fall back to a sensible default (e.g. 128 000) when `None`.
    pub fn get_context_length(&self, model_id: &str) -> Option<u32> {
        CURATED_MODELS
            .iter()
            .find(|m| m.id == model_id)
            .map(|m| m.context_length)
    }

    /// Return available models, fetching from the provider via
    /// [`LlmModelListerRef`].
    ///
    /// - `favorite_ids` — model IDs the user has pinned.
    pub async fn list_models(&self, favorite_ids: &[String]) -> Vec<ChatModel> {
        // Fast path: fresh cache
        {
            let guard = self.cache.lock().await;
            if let Some(ref entry) = *guard {
                if entry.fetched_at.elapsed() < CACHE_TTL {
                    return Self::materialize(&entry.models, favorite_ids);
                }
            }
        }

        // Slow path: fetch from provider
        match self.fetch_models().await {
            Ok(raw_models) => {
                let result = Self::materialize(&raw_models, favorite_ids);
                let mut guard = self.cache.lock().await;
                *guard = Some(CacheEntry {
                    fetched_at: Instant::now(),
                    models:     raw_models,
                });
                result
            }
            Err(e) => {
                warn!(error = %e, "failed to fetch models from provider, using fallback");
                // Try stale cache
                let guard = self.cache.lock().await;
                if let Some(ref entry) = *guard {
                    debug!(
                        "using stale cache ({:.0}s old)",
                        entry.fetched_at.elapsed().as_secs_f64()
                    );
                    return Self::materialize(&entry.models, favorite_ids);
                }
                drop(guard);
                curated_fallback(favorite_ids)
            }
        }
    }

    /// Fetch models from the provider via the driver.
    async fn fetch_models(&self) -> anyhow::Result<Vec<RawModel>> {
        debug!("fetching models from LLM provider");
        let models = self
            .model_lister
            .list_models()
            .await
            .map_err(|e| anyhow::anyhow!("failed to list models: {e}"))?;

        Ok(models
            .into_iter()
            .map(|m| {
                // No provider hint here: `LlmModelLister::list_models()` does not
                // carry one. The substring matcher in `is_known_vision_model`
                // still resolves well-known model ids correctly.
                let supports_vision = ModelCapabilities::detect(None, &m.id).supports_vision;
                RawModel {
                    id: m.id.clone(),
                    name: m.id,
                    context_length: 0,
                    supports_vision,
                }
            })
            .collect())
    }

    /// Convert raw cached models into [`ChatModel`]s with favorite flags and
    /// stable sorting.
    fn materialize(raw: &[RawModel], favorite_ids: &[String]) -> Vec<ChatModel> {
        let mut models: Vec<ChatModel> = raw
            .iter()
            .map(|m| ChatModel {
                id:              m.id.clone(),
                name:            m.name.clone(),
                context_length:  m.context_length,
                is_favorite:     favorite_ids.iter().any(|f| f == &m.id),
                supports_vision: m.supports_vision,
            })
            .collect();
        apply_favorite_sort(&mut models);
        models
    }
}

/// Stable sort: favorites first, then alphabetical by name within each group.
fn apply_favorite_sort(models: &mut [ChatModel]) {
    models.sort_by(|a, b| {
        b.is_favorite
            .cmp(&a.is_favorite)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}
