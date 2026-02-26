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

//! [Reciprocal Rank Fusion][rrf] (RRF) for merging multi-source search results.
//!
//! When the memory layer searches mem0 and Hindsight in parallel, each backend
//! returns its own ranked list. RRF merges these lists into a single ranking
//! without requiring score normalisation — it only uses *rank positions*.
//!
//! ## Algorithm
//!
//! For each item appearing at rank *r* in list *i*, it receives a score of
//! `1 / (k + r)`. Scores are summed across all lists. Items appearing in
//! multiple lists are naturally boosted because they accumulate scores from
//! each list.
//!
//! The constant *k* (default 60.0) dampens the influence of top positions,
//! preventing a single high-ranked item in one list from dominating the
//! fused ranking.
//!
//! ## References
//!
//! [rrf]: https://plg.uwaterloo.ca/~gvcormac/cormacksigir09-rrf.pdf
//!   "Reciprocal Rank Fusion outperforms Condorcet and individual Rank Learning
//! Methods (Cormack et al., 2009)"

use std::collections::HashMap;

use crate::manager::SearchResult;

/// Merge multiple ranked result sets using Reciprocal Rank Fusion.
///
/// Each result set is an ordered list of [`SearchResult`] items (best first).
/// RRF assigns each item a score of `1 / (k + rank)` for each list it appears
/// in, then sums across all lists. The constant `k` (typically 60.0) dampens
/// the influence of high-ranking positions.
///
/// Returns at most `limit` results, sorted by descending fused score.
pub fn reciprocal_rank_fusion(
    result_sets: Vec<Vec<SearchResult>>,
    limit: usize,
    k: f64,
) -> Vec<SearchResult> {
    // Accumulate RRF scores keyed by result id.
    let mut scores: HashMap<String, (f64, SearchResult)> = HashMap::new();

    for set in result_sets {
        for (rank, result) in set.into_iter().enumerate() {
            let rrf_score = 1.0 / (k + rank as f64 + 1.0);
            let entry = scores
                .entry(result.id.clone())
                .or_insert_with(|| (0.0, result));
            entry.0 += rrf_score;
        }
    }

    // Sort by accumulated score descending.
    let mut fused: Vec<SearchResult> = scores
        .into_values()
        .map(|(score, mut result)| {
            result.score = score;
            result
        })
        .collect();

    fused.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    fused.truncate(limit);
    fused
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::MemorySource;

    fn make_result(id: &str, score: f64) -> SearchResult {
        SearchResult {
            id: id.to_owned(),
            source: MemorySource::Mem0,
            content: format!("content for {id}"),
            score,
        }
    }

    #[test]
    fn single_list_preserves_order() {
        let set = vec![
            make_result("a", 1.0),
            make_result("b", 0.5),
            make_result("c", 0.1),
        ];
        let fused = reciprocal_rank_fusion(vec![set], 10, 60.0);
        assert_eq!(fused.len(), 3);
        assert_eq!(fused[0].id, "a");
        assert_eq!(fused[1].id, "b");
        assert_eq!(fused[2].id, "c");
    }

    #[test]
    fn overlapping_items_get_boosted() {
        let set_a = vec![make_result("x", 1.0), make_result("y", 0.5)];
        let set_b = vec![make_result("y", 1.0), make_result("z", 0.5)];
        let fused = reciprocal_rank_fusion(vec![set_a, set_b], 10, 60.0);

        // "y" appears in both lists so should have the highest fused score.
        assert_eq!(fused[0].id, "y");
    }

    #[test]
    fn limit_is_respected() {
        let set = vec![
            make_result("a", 1.0),
            make_result("b", 0.5),
            make_result("c", 0.1),
        ];
        let fused = reciprocal_rank_fusion(vec![set], 2, 60.0);
        assert_eq!(fused.len(), 2);
    }

    #[test]
    fn empty_input() {
        let fused = reciprocal_rank_fusion(vec![], 10, 60.0);
        assert!(fused.is_empty());
    }
}
