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

//! OpenRouter model capability catalog with fuzzy lookup.
//!
//! Some LLM drivers (e.g. Kimi, Codex) misreport vision support.  This
//! module queries the OpenRouter public `/models` endpoint once per process
//! lifetime to obtain authoritative capability metadata, then provides a
//! fuzzy-matching lookup so callers can query by whatever model string they
//! already have (with or without provider prefix, version suffixes, etc.).

use std::time::Duration;

use serde::Deserialize;
use tokio::sync::OnceCell;

const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single model's capabilities as reported by OpenRouter.
#[derive(Debug, Clone)]
pub struct CatalogEntry {
    /// Full model ID including provider prefix (e.g. `"openai/gpt-5"`).
    pub id:              String,
    /// Maximum context window size in tokens, if known.
    pub context_length:  Option<usize>,
    /// Whether the model accepts image inputs.
    pub supports_vision: bool,
}

/// Lazy, process-lifetime cache of OpenRouter model capabilities.
///
/// The catalog fetches from OpenRouter on first `lookup` call and caches the
/// result for the remainder of the process.  Network or parse failures are
/// logged and silently degrade to "no data" (empty catalog).
pub struct OpenRouterCatalog {
    client:  reqwest::Client,
    /// `(normalized_id, entry)` pairs, populated on first access.
    entries: OnceCell<Vec<(String, CatalogEntry)>>,
}

impl OpenRouterCatalog {
    /// Create a new catalog.  Does **not** fetch from OpenRouter yet.
    pub fn new() -> Self {
        Self {
            client:  reqwest::Client::new(),
            entries: OnceCell::new(),
        }
    }

    /// Look up a model by name, lazily fetching the catalog on first call.
    ///
    /// The `model_name` is normalized and fuzzy-matched against the cached
    /// catalog entries.  Returns `None` if the catalog is empty or no
    /// reasonable match is found.
    #[tracing::instrument(skip_all, fields(model = %model_name))]
    pub async fn lookup(&self, model_name: &str) -> Option<&CatalogEntry> {
        let entries = self.entries.get_or_init(|| self.fetch()).await;
        fuzzy_find_in_entries(entries, model_name)
    }

    /// Fetch the full model list from OpenRouter, parse, and normalize.
    ///
    /// On any error the function warns and returns an empty vec so that the
    /// `OnceCell` is initialized (no retries — a process restart resets it).
    async fn fetch(&self) -> Vec<(String, CatalogEntry)> {
        let result: Result<Vec<(String, CatalogEntry)>, reqwest::Error> = async {
            let resp: WireModelsResponse = self
                .client
                .get(OPENROUTER_MODELS_URL)
                .timeout(FETCH_TIMEOUT)
                .send()
                .await?
                .json()
                .await?;

            let entries = resp
                .data
                .into_iter()
                .map(|w| {
                    let supports_vision = w
                        .architecture
                        .as_ref()
                        .map_or(false, |a| a.input_modalities.iter().any(|m| m == "image"));
                    let norm = normalize(&w.id);
                    let entry = CatalogEntry {
                        id: w.id,
                        context_length: w.context_length,
                        supports_vision,
                    };
                    (norm, entry)
                })
                .collect();

            Ok(entries)
        }
        .await;

        match result {
            Ok(entries) => {
                tracing::info!(count = entries.len(), "openrouter catalog loaded");
                entries
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to fetch openrouter catalog; vision overrides disabled");
                Vec::new()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Wire types (OpenRouter /models response)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WireModelsResponse {
    data: Vec<WireModelEntry>,
}

#[derive(Deserialize)]
struct WireModelEntry {
    id:             String,
    #[serde(default)]
    context_length: Option<usize>,
    #[serde(default)]
    architecture:   Option<WireModelArchitecture>,
}

#[derive(Deserialize)]
struct WireModelArchitecture {
    #[serde(default)]
    input_modalities: Vec<String>,
}

// ---------------------------------------------------------------------------
// Normalization
// ---------------------------------------------------------------------------

/// Normalize a model identifier for fuzzy comparison.
///
/// Steps:
/// 1. ASCII lowercase
/// 2. Strip provider prefix (`openai/gpt-5` -> `gpt-5`)
/// 3. Strip known suffixes: `-preview`, `-latest`, `-exp`
/// 4. Strip minor versions: segment `k2.6` -> `k2`, `3.5` -> `3`
/// 5. Rejoin segments with `-`
fn normalize(s: &str) -> String {
    let lower = s.to_ascii_lowercase();

    // Strip provider prefix (everything before the last `/`).
    let base = lower.rsplit('/').next().unwrap_or(&lower);

    // Strip known suffixes.
    let trimmed = base
        .trim_end_matches("-preview")
        .trim_end_matches("-latest")
        .trim_end_matches("-exp");

    // Split on `-`, strip minor versions from each segment, rejoin.
    trimmed
        .split('-')
        .map(|seg| {
            // If a segment contains a `.`, strip `.N` where N is all digits.
            if let Some(dot_pos) = seg.find('.') {
                let suffix = &seg[dot_pos + 1..];
                if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
                    return &seg[..dot_pos];
                }
            }
            seg
        })
        .collect::<Vec<_>>()
        .join("-")
}

// ---------------------------------------------------------------------------
// Fuzzy matching
// ---------------------------------------------------------------------------

/// Find the best-matching catalog entry for a user-supplied model name.
///
/// Strategy:
/// 1. **Exact match** on the normalized name.
/// 2. **Bidirectional substring**: either the catalog entry contains the needle
///    or the needle contains the catalog entry.  Among all hits the entry with
///    the **shortest** normalized ID wins (closest semantic match).
fn fuzzy_find_in_entries<'a>(
    entries: &'a [(String, CatalogEntry)],
    model_name: &str,
) -> Option<&'a CatalogEntry> {
    let needle = normalize(model_name);

    // Step 1: exact match.
    if let Some((_norm, entry)) = entries.iter().find(|(norm, _)| *norm == needle) {
        return Some(entry);
    }

    // Step 2: bidirectional substring — pick shortest normalized ID.
    entries
        .iter()
        .filter(|(norm, _)| norm.contains(&*needle) || needle.contains(&**norm))
        .min_by_key(|(norm, _)| norm.len())
        .map(|(_, entry)| entry)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_provider_prefix() {
        assert_eq!(normalize("moonshotai/kimi-k2-code"), "kimi-k2-code");
        assert_eq!(normalize("openai/gpt-5"), "gpt-5");
        assert_eq!(normalize("gpt-4o"), "gpt-4o");
    }

    #[test]
    fn normalize_strips_suffixes_and_minor_version() {
        assert_eq!(normalize("K2.6-code-preview"), "k2-code");
        assert_eq!(normalize("gpt-5.4"), "gpt-5");
        assert_eq!(normalize("claude-3.5-sonnet-latest"), "claude-3-sonnet");
    }

    #[test]
    fn fuzzy_match_bidirectional_substring() {
        let entries = vec![
            (
                "kimi-k2-code".into(),
                CatalogEntry {
                    id:              "moonshotai/kimi-k2-code".into(),
                    context_length:  Some(128_000),
                    supports_vision: false,
                },
            ),
            (
                "kimi-k2".into(),
                CatalogEntry {
                    id:              "moonshotai/kimi-k2.5".into(),
                    context_length:  Some(128_000),
                    supports_vision: true,
                },
            ),
            (
                "gpt-5".into(),
                CatalogEntry {
                    id:              "openai/gpt-5".into(),
                    context_length:  Some(200_000),
                    supports_vision: true,
                },
            ),
        ];

        // "K2.6-code-preview" -> "k2-code", "kimi-k2-code" contains "k2-code"
        let hit = fuzzy_find_in_entries(&entries, "K2.6-code-preview");
        assert!(hit.is_some());
        assert!(!hit.unwrap().supports_vision);

        // "gpt-5.4" -> "gpt-5" exact match
        let hit = fuzzy_find_in_entries(&entries, "gpt-5.4");
        assert!(hit.is_some());
        assert!(hit.unwrap().supports_vision);

        // "K2.5" -> "k2": both "kimi-k2" and "kimi-k2-code" contain "k2"
        // Shortest wins -> "kimi-k2" (len 7) -> kimi-k2.5 (vision=true)
        let hit = fuzzy_find_in_entries(&entries, "K2.5");
        assert!(hit.is_some());
        assert!(hit.unwrap().supports_vision);

        // Unknown -> None
        assert!(fuzzy_find_in_entries(&entries, "totally-unknown").is_none());
    }
}
