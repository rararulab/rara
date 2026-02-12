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

//! Deduplication utilities for job discovery.
//!
//! Two levels of dedup are provided:
//!
//! 1. **Exact dedup** -- based on the idempotent key `(source_job_id,
//!    source_name)`.
//! 2. **Fuzzy cross-source dedup** -- based on a normalized `(title, company)`
//!    pair to detect the same position posted on multiple platforms.

use std::{collections::HashSet, hash::BuildHasher};

use crate::types::NormalizedJob;

/// An idempotent key that uniquely identifies a job within a single
/// source.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceKey {
    pub source_job_id: String,
    pub source_name:   String,
}

impl SourceKey {
    /// Build a [`SourceKey`] from a [`NormalizedJob`].
    #[must_use]
    pub fn from_job(job: &NormalizedJob) -> Self {
        Self {
            source_job_id: job.source_job_id.clone(),
            source_name:   job.source_name.clone(),
        }
    }
}

/// A fuzzy key for cross-source duplicate detection.
///
/// Both `title` and `company` are lowercased and trimmed so that
/// minor formatting differences across sources do not prevent
/// matching.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FuzzyKey {
    pub title:   String,
    pub company: String,
}

impl FuzzyKey {
    /// Build a [`FuzzyKey`] from a [`NormalizedJob`].
    #[must_use]
    pub fn from_job(job: &NormalizedJob) -> Self {
        Self {
            title:   job.title.trim().to_lowercase(),
            company: job.company.trim().to_lowercase(),
        }
    }
}

/// Check whether a job already exists based on its
/// `(source_job_id, source_name)` idempotent key.
///
/// Returns `true` if the key is already present in `known_keys`.
#[must_use]
pub fn is_exact_duplicate<S: BuildHasher>(
    job: &NormalizedJob,
    known_keys: &HashSet<SourceKey, S>,
) -> bool {
    known_keys.contains(&SourceKey::from_job(job))
}

/// Check whether a job is a likely cross-source duplicate by fuzzy
/// matching on `(title, company)`.
///
/// Returns `true` if a job with the same normalized title and company
/// already exists in `known_fuzzy`.
#[must_use]
pub fn is_fuzzy_duplicate<S: BuildHasher>(
    job: &NormalizedJob,
    known_fuzzy: &HashSet<FuzzyKey, S>,
) -> bool {
    known_fuzzy.contains(&FuzzyKey::from_job(job))
}

/// Remove duplicates from a list of normalized jobs.
///
/// Jobs are deduplicated in two passes:
/// 1. **Exact**: only the first occurrence per `(source_job_id, source_name)`
///    is kept.
/// 2. **Fuzzy**: only the first occurrence per `(lowered_title,
///    lowered_company)` is kept.
///
/// The `existing_source_keys` and `existing_fuzzy_keys` sets
/// represent jobs that are already known (e.g. persisted in the
/// database).  Any incoming job that collides with an existing key
/// is dropped.
#[must_use]
pub fn deduplicate<S1, S2>(
    jobs: Vec<NormalizedJob>,
    existing_source_keys: &HashSet<SourceKey, S1>,
    existing_fuzzy_keys: &HashSet<FuzzyKey, S2>,
) -> Vec<NormalizedJob>
where
    S1: BuildHasher,
    S2: BuildHasher,
{
    let mut seen_source: HashSet<SourceKey> = existing_source_keys.iter().cloned().collect();
    let mut seen_fuzzy: HashSet<FuzzyKey> = existing_fuzzy_keys.iter().cloned().collect();
    let mut result = Vec::with_capacity(jobs.len());

    for job in jobs {
        let source_key = SourceKey::from_job(&job);
        let fuzzy_key = FuzzyKey::from_job(&job);

        if seen_source.contains(&source_key) {
            tracing::debug!(
                source_job_id = %job.source_job_id,
                source_name = %job.source_name,
                "Dropping exact duplicate"
            );
            continue;
        }

        if seen_fuzzy.contains(&fuzzy_key) {
            tracing::debug!(
                title = %job.title,
                company = %job.company,
                source_name = %job.source_name,
                "Dropping fuzzy cross-source duplicate"
            );
            continue;
        }

        seen_source.insert(source_key);
        seen_fuzzy.insert(fuzzy_key);
        result.push(job);
    }

    result
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use uuid::Uuid;

    use super::*;

    fn make_job(
        source_job_id: &str,
        source_name: &str,
        title: &str,
        company: &str,
    ) -> NormalizedJob {
        NormalizedJob {
            id:              Uuid::new_v4(),
            source_job_id:   source_job_id.to_owned(),
            source_name:     source_name.to_owned(),
            title:           title.to_owned(),
            company:         company.to_owned(),
            location:        None,
            description:     None,
            url:             None,
            salary_min:      None,
            salary_max:      None,
            salary_currency: None,
            tags:            vec![],
            raw_data:        None,
            posted_at:       None,
        }
    }

    #[test]
    fn exact_dedup_removes_same_source_key() {
        let jobs = vec![
            make_job("1", "linkedin", "Rust Dev", "Acme"),
            make_job("1", "linkedin", "Rust Dev", "Acme"), // dup
            make_job("2", "linkedin", "Go Dev", "Acme"),
        ];

        let result = deduplicate(jobs, &HashSet::new(), &HashSet::new());
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn fuzzy_dedup_removes_cross_source_duplicates() {
        let jobs = vec![
            make_job("1", "linkedin", "Rust Dev", "Acme Corp"),
            make_job("42", "indeed", "rust dev", "acme corp"), // fuzzy dup
        ];

        let result = deduplicate(jobs, &HashSet::new(), &HashSet::new());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_name, "linkedin");
    }

    #[test]
    fn existing_keys_are_honored() {
        let existing = {
            let mut s = HashSet::new();
            s.insert(SourceKey {
                source_job_id: "1".to_owned(),
                source_name:   "linkedin".to_owned(),
            });
            s
        };

        let jobs = vec![
            make_job("1", "linkedin", "Rust Dev", "Acme"),
            make_job("2", "linkedin", "Go Dev", "Beta Inc"),
        ];

        let result = deduplicate(jobs, &existing, &HashSet::new());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_job_id, "2");
    }
}
