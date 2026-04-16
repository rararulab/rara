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

/// Pre-computed normalized forms for a single catalog entry.
struct NormalizedEntry {
    /// Light normalization: lowercase + strip provider prefix + strip suffixes.
    /// Preserves version numbers (e.g. `kimi-k2.5`).
    light:      String,
    /// Aggressive normalization: also strip `.N` minor versions
    /// (e.g. `kimi-k2.5` → `kimi-k2`).
    aggressive: String,
    entry:      CatalogEntry,
}

/// Lazy, process-lifetime cache of OpenRouter model capabilities.
///
/// The catalog fetches from OpenRouter on first `lookup` call and caches the
/// result for the remainder of the process.  Network or parse failures are
/// logged and silently degrade to "no data" (empty catalog).
pub struct OpenRouterCatalog {
    client:  reqwest::Client,
    entries: OnceCell<Vec<NormalizedEntry>>,
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
    async fn fetch(&self) -> Vec<NormalizedEntry> {
        let result: Result<Vec<NormalizedEntry>, reqwest::Error> = async {
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
                    let light = normalize_light(&w.id);
                    let aggressive = strip_minor_versions(&light);
                    let entry = CatalogEntry {
                        id: w.id,
                        context_length: w.context_length,
                        supports_vision,
                    };
                    NormalizedEntry {
                        light,
                        aggressive,
                        entry,
                    }
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
// Normalization (two levels)
// ---------------------------------------------------------------------------

/// Light normalization: lowercase + strip provider prefix + strip suffixes.
///
/// Preserves version numbers so that `kimi-k2.5` and `kimi-k2` remain
/// distinguishable.
fn normalize_light(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    base.trim_end_matches("-preview")
        .trim_end_matches("-latest")
        .trim_end_matches("-exp")
        .to_owned()
}

/// Aggressive normalization: strip `.N` minor-version suffixes from each
/// `-`-delimited segment.  `k2.6` → `k2`, `gpt-5.4` → `gpt-5`.
fn strip_minor_versions(light: &str) -> String {
    light
        .split('-')
        .map(|seg| {
            if let Some(dot) = seg.find('.') {
                let suffix = &seg[dot + 1..];
                if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
                    return &seg[..dot];
                }
            }
            seg
        })
        .collect::<Vec<_>>()
        .join("-")
}

// ---------------------------------------------------------------------------
// Fuzzy matching — two-pass
// ---------------------------------------------------------------------------

/// Find the best-matching catalog entry for a user-supplied model name.
///
/// Two passes, each with exact → substring fallback:
///
/// 1. **Light pass** (version-preserving): `K2.5` → `k2.5`, matches `kimi-k2.5`
///    but *not* `kimi-k2`.  This prevents `kimi-k2.5` (vision) and `kimi-k2`
///    (no vision) from colliding.
/// 2. **Aggressive pass** (version-stripped): `K2.6-code-preview` → `k2-code`,
///    broadens the net for models whose OpenRouter ID omits the minor version.
///
/// Within each substring pass the **shortest** matching ID wins (least
/// extraneous content = closest semantic match).
fn fuzzy_find_in_entries<'a>(
    entries: &'a [NormalizedEntry],
    model_name: &str,
) -> Option<&'a CatalogEntry> {
    let needle_light = normalize_light(model_name);
    let needle_agg = strip_minor_versions(&needle_light);

    // Pass 1a: exact on light.
    if let Some(ne) = entries.iter().find(|ne| ne.light == needle_light) {
        return Some(&ne.entry);
    }
    // Pass 1b: substring on light — shortest wins.
    let hit = entries
        .iter()
        .filter(|ne| ne.light.contains(&*needle_light) || needle_light.contains(&*ne.light))
        .min_by_key(|ne| ne.light.len());
    if let Some(ne) = hit {
        return Some(&ne.entry);
    }

    // Pass 2a: exact on aggressive.
    if let Some(ne) = entries.iter().find(|ne| ne.aggressive == needle_agg) {
        return Some(&ne.entry);
    }
    // Pass 2b: substring on aggressive — shortest wins.
    entries
        .iter()
        .filter(|ne| ne.aggressive.contains(&*needle_agg) || needle_agg.contains(&*ne.aggressive))
        .min_by_key(|ne| ne.aggressive.len())
        .map(|ne| &ne.entry)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(id: &str, vision: bool) -> NormalizedEntry {
        let light = normalize_light(id);
        let aggressive = strip_minor_versions(&light);
        NormalizedEntry {
            light,
            aggressive,
            entry: CatalogEntry {
                id:              id.into(),
                context_length:  Some(128_000),
                supports_vision: vision,
            },
        }
    }

    #[test]
    fn light_normalization_preserves_versions() {
        assert_eq!(normalize_light("moonshotai/kimi-k2-code"), "kimi-k2-code");
        assert_eq!(normalize_light("openai/gpt-5"), "gpt-5");
        assert_eq!(normalize_light("gpt-4o"), "gpt-4o");
        // Versions preserved at this level.
        assert_eq!(normalize_light("K2.6-code-preview"), "k2.6-code");
        assert_eq!(
            normalize_light("claude-3.5-sonnet-latest"),
            "claude-3.5-sonnet"
        );
    }

    #[test]
    fn aggressive_normalization_strips_minor_versions() {
        assert_eq!(strip_minor_versions("k2.6-code"), "k2-code");
        assert_eq!(strip_minor_versions("gpt-5.4"), "gpt-5");
        assert_eq!(strip_minor_versions("claude-3.5-sonnet"), "claude-3-sonnet");
        // No minor version to strip.
        assert_eq!(strip_minor_versions("gpt-4o"), "gpt-4o");
    }

    #[test]
    fn two_pass_fuzzy_match() {
        let entries = vec![
            make_entry("moonshotai/kimi-k2-code", false),
            make_entry("moonshotai/kimi-k2.5", true),
            make_entry("openai/gpt-5", true),
        ];

        // "K2.6-code-preview" → light "k2.6-code" (no light match)
        // → aggressive "k2-code", substring of "kimi-k2-code" ✓ → vision=false
        let hit = fuzzy_find_in_entries(&entries, "K2.6-code-preview");
        assert!(hit.is_some());
        assert!(!hit.unwrap().supports_vision);

        // "gpt-5.4" → light "gpt-5.4", substring contains "gpt-5" ✓
        let hit = fuzzy_find_in_entries(&entries, "gpt-5.4");
        assert!(hit.is_some());
        assert!(hit.unwrap().supports_vision);

        // "K2.5" → light "k2.5", substring of "kimi-k2.5" ✓ → vision=true
        // Does NOT collide with kimi-k2-code (light pass finds k2.5 first).
        let hit = fuzzy_find_in_entries(&entries, "K2.5");
        assert!(hit.is_some());
        assert!(hit.unwrap().supports_vision);

        // Unknown → None
        assert!(fuzzy_find_in_entries(&entries, "totally-unknown").is_none());
    }

    /// Smoke test: hit the real OpenRouter endpoint and verify we get parseable
    /// data.
    #[tokio::test]
    #[ignore = "requires network access"]
    async fn catalog_fetches_real_models() {
        let catalog = OpenRouterCatalog::new();
        let hit = catalog.lookup("gpt-4o").await;
        assert!(hit.is_some(), "gpt-4o should be in the OpenRouter catalog");
        assert!(hit.unwrap().supports_vision, "gpt-4o should support vision");

        // Verify kimi models are correctly differentiated.
        let kimi = catalog.lookup("kimi-k2.5").await;
        if let Some(entry) = kimi {
            assert!(entry.supports_vision, "kimi-k2.5 should support vision");
        }

        let kimi_code = catalog.lookup("K2.6-code-preview").await;
        if let Some(entry) = kimi_code {
            assert!(
                !entry.supports_vision,
                "K2.6-code-preview should NOT support vision"
            );
        }
    }
}
