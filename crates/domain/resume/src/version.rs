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

//! Resume version tree and text diffing utilities.
//!
//! Each resume can optionally reference a *parent* resume via
//! `parent_resume_id`, forming a derivation tree.  This module provides
//! the [`ResumeVersionTree`] type for inspecting that tree as well as a
//! simple line-level diff function.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{DiffHunk, DiffLine, Resume, ResumeDiff, ResumeSource};

// ---------------------------------------------------------------------------
// Version node
// ---------------------------------------------------------------------------

/// Lightweight view of a single resume version in the derivation chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeVersion {
    pub resume_id:  Uuid,
    pub parent_id:  Option<Uuid>,
    pub title:      String,
    pub source:     ResumeSource,
    pub created_at: Timestamp,
}

impl From<&Resume> for ResumeVersion {
    fn from(r: &Resume) -> Self {
        Self {
            resume_id:  r.id,
            parent_id:  r.parent_resume_id,
            title:      r.title.clone(),
            source:     r.source,
            created_at: r.created_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Version tree
// ---------------------------------------------------------------------------

/// The full derivation chain for a resume, ordered from root to leaf.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeVersionTree {
    /// Ordered list of versions from the root ancestor to the target
    /// resume.
    pub versions: Vec<ResumeVersion>,
}

impl ResumeVersionTree {
    /// Build a version tree from a flat list of ancestor resumes.
    ///
    /// The input should be ordered from the *oldest ancestor* (root) to
    /// the most recent descendant.
    #[must_use]
    pub fn from_history(resumes: &[Resume]) -> Self {
        let versions = resumes.iter().map(ResumeVersion::from).collect();
        Self { versions }
    }

    /// Return the root (baseline) version, if the tree is non-empty.
    #[must_use]
    pub fn root(&self) -> Option<&ResumeVersion> { self.versions.first() }

    /// Return the leaf (most recent) version, if the tree is non-empty.
    #[must_use]
    pub fn leaf(&self) -> Option<&ResumeVersion> { self.versions.last() }

    /// Return the depth of the derivation chain (number of versions).
    #[allow(clippy::missing_const_for_fn)]
    #[must_use]
    pub fn depth(&self) -> usize { self.versions.len() }

    /// Returns `true` if the tree contains no versions.
    #[allow(clippy::missing_const_for_fn)]
    #[must_use]
    pub fn is_empty(&self) -> bool { self.versions.is_empty() }
}

// ---------------------------------------------------------------------------
// Text diff
// ---------------------------------------------------------------------------

/// Compute a simple line-level diff between two pieces of text.
///
/// This implements a basic longest-common-subsequence (LCS) diff and
/// groups consecutive changes into [`DiffHunk`]s.
#[must_use]
#[allow(clippy::similar_names)]
pub fn compute_diff(resume_a_id: Uuid, resume_b_id: Uuid, old: &str, new: &str) -> ResumeDiff {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let ops = lcs_diff(&old_lines, &new_lines);
    let hunks = build_hunks(&ops);

    ResumeDiff {
        resume_a_id,
        resume_b_id,
        hunks,
    }
}

// ---------------------------------------------------------------------------
// Internal LCS diff implementation
// ---------------------------------------------------------------------------

/// An edit operation produced by the LCS algorithm.
#[derive(Debug, Clone)]
enum EditOp {
    Equal(String),
    Insert(String),
    Delete(String),
}

/// Classic LCS-based diff producing a sequence of edit operations.
fn lcs_diff(old: &[&str], new: &[&str]) -> Vec<EditOp> {
    let m = old.len();
    let n = new.len();

    // Build LCS table.
    let mut table = vec![vec![0u32; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            table[i][j] = if old[i - 1] == new[j - 1] {
                table[i - 1][j - 1] + 1
            } else {
                table[i - 1][j].max(table[i][j - 1])
            };
        }
    }

    // Back-track to produce edit operations.
    let mut ops = Vec::new();
    let (mut i, mut j) = (m, n);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
            ops.push(EditOp::Equal(old[i - 1].to_owned()));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || table[i][j - 1] >= table[i - 1][j]) {
            ops.push(EditOp::Insert(new[j - 1].to_owned()));
            j -= 1;
        } else {
            ops.push(EditOp::Delete(old[i - 1].to_owned()));
            i -= 1;
        }
    }
    ops.reverse();
    ops
}

/// Group a flat list of [`EditOp`]s into [`DiffHunk`]s.
///
/// Consecutive non-equal operations (possibly interspersed with a small
/// amount of context) are grouped into a single hunk.
fn build_hunks(ops: &[EditOp]) -> Vec<DiffHunk> {
    let mut hunks: Vec<DiffHunk> = Vec::new();

    let mut old_line = 1usize;
    let mut new_line = 1usize;

    // We accumulate changed ops; when we see enough equal lines we flush.
    let mut current_lines: Vec<DiffLine> = Vec::new();
    let mut hunk_old_start = old_line;
    let mut hunk_new_start = new_line;
    let mut hunk_old_count = 0usize;
    let mut hunk_new_count = 0usize;
    let mut in_hunk = false;
    let context_size: usize = 3;
    let mut trailing_context = 0usize;

    for op in ops {
        match op {
            EditOp::Equal(line) => {
                if in_hunk {
                    trailing_context += 1;
                    current_lines.push(DiffLine::Context(line.clone()));
                    hunk_old_count += 1;
                    hunk_new_count += 1;

                    if trailing_context >= context_size {
                        // Flush hunk.
                        hunks.push(DiffHunk {
                            old_start: hunk_old_start,
                            old_count: hunk_old_count,
                            new_start: hunk_new_start,
                            new_count: hunk_new_count,
                            lines:     std::mem::take(&mut current_lines),
                        });
                        in_hunk = false;
                    }
                }
                old_line += 1;
                new_line += 1;
            }
            EditOp::Insert(line) => {
                if !in_hunk {
                    in_hunk = true;
                    hunk_old_start = old_line;
                    hunk_new_start = new_line;
                    hunk_old_count = 0;
                    hunk_new_count = 0;
                }
                trailing_context = 0;
                current_lines.push(DiffLine::Added(line.clone()));
                hunk_new_count += 1;
                new_line += 1;
            }
            EditOp::Delete(line) => {
                if !in_hunk {
                    in_hunk = true;
                    hunk_old_start = old_line;
                    hunk_new_start = new_line;
                    hunk_old_count = 0;
                    hunk_new_count = 0;
                }
                trailing_context = 0;
                current_lines.push(DiffLine::Removed(line.clone()));
                hunk_old_count += 1;
                old_line += 1;
            }
        }
    }

    // Flush any remaining hunk.
    if in_hunk && !current_lines.is_empty() {
        hunks.push(DiffHunk {
            old_start: hunk_old_start,
            old_count: hunk_old_count,
            new_start: hunk_new_start,
            new_count: hunk_new_count,
            lines:     current_lines,
        });
    }

    hunks
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;

    use super::*;

    #[test]
    fn diff_identical_texts() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let diff = compute_diff(id_a, id_b, "hello\nworld", "hello\nworld");
        assert!(diff.hunks.is_empty());
    }

    #[test]
    fn diff_detects_additions() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let diff = compute_diff(id_a, id_b, "line1\nline2", "line1\ninserted\nline2");
        assert!(!diff.hunks.is_empty());
        let has_added = diff
            .hunks
            .iter()
            .flat_map(|h| &h.lines)
            .any(|l| matches!(l, DiffLine::Added(_)));
        assert!(has_added);
    }

    #[test]
    fn diff_detects_removals() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let diff = compute_diff(id_a, id_b, "line1\nremoved\nline2", "line1\nline2");
        assert!(!diff.hunks.is_empty());
        let has_removed = diff
            .hunks
            .iter()
            .flat_map(|h| &h.lines)
            .any(|l| matches!(l, DiffLine::Removed(_)));
        assert!(has_removed);
    }

    #[test]
    fn version_tree_from_history() {
        let now = Timestamp::now();
        let root_id = Uuid::new_v4();
        let child_id = Uuid::new_v4();
        let resumes = vec![
            Resume {
                id:                  root_id,
                title:               "Base".into(),
                version_tag:         "v1".into(),
                content_hash:        "aaa".into(),
                source:              ResumeSource::Manual,
                content:             Some("root content".into()),
                parent_resume_id:    None,
                target_job_id:       None,
                customization_notes: None,
                tags:                vec![],
                metadata:            None,
                trace_id:            None,
                is_deleted:          false,
                deleted_at:          None,
                created_at:          now,
                updated_at:          now,
            },
            Resume {
                id:                  child_id,
                title:               "Derived".into(),
                version_tag:         "v2".into(),
                content_hash:        "bbb".into(),
                source:              ResumeSource::Optimized,
                content:             Some("child content".into()),
                parent_resume_id:    Some(root_id),
                target_job_id:       None,
                customization_notes: Some("tailored for SWE role".into()),
                tags:                vec!["swe".into()],
                metadata:            None,
                trace_id:            None,
                is_deleted:          false,
                deleted_at:          None,
                created_at:          now,
                updated_at:          now,
            },
        ];

        let tree = ResumeVersionTree::from_history(&resumes);
        assert_eq!(tree.depth(), 2);
        assert_eq!(tree.root().unwrap().resume_id, root_id);
        assert_eq!(tree.leaf().unwrap().resume_id, child_id);
    }
}
