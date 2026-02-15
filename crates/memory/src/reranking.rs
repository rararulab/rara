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

//! Lightweight reranking for memory candidates.

use std::collections::HashSet;

use crate::manager::SearchResult;

/// Re-rank candidates using token overlap with query.
///
/// This lightweight reranker is deterministic and avoids additional model
/// calls, making it safe to run on every request.
pub fn rerank_results(
    query: &str,
    mut candidates: Vec<SearchResult>,
    limit: usize,
) -> Vec<SearchResult> {
    let query_tokens = tokenize(query);

    candidates.sort_by(|a, b| {
        let score_a = rerank_score(a, &query_tokens);
        let score_b = rerank_score(b, &query_tokens);
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    candidates.truncate(limit);
    candidates
}

fn rerank_score(candidate: &SearchResult, query_tokens: &HashSet<String>) -> f64 {
    let text = format!("{} {}", candidate.path, candidate.snippet);
    let candidate_tokens = tokenize(&text);
    let overlap = query_tokens.intersection(&candidate_tokens).count() as f64;
    let overlap_ratio = if query_tokens.is_empty() {
        0.0
    } else {
        overlap / query_tokens.len() as f64
    };

    // Base fused score + overlap bonus.
    candidate.score + overlap_ratio * 0.35
}

fn tokenize(input: &str) -> HashSet<String> {
    input
        .split(|c: char| !c.is_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() >= 2)
        .map(|token| token.to_ascii_lowercase())
        .collect()
}
