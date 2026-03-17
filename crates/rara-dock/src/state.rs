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

use crate::models::{
    Actor, DockAnnotation, DockBlock, DockCanvasSnapshot, DockFact, DockMutation, MutationOp,
};

/// Build the dock system prompt injected when dock mode is active.
///
/// Includes contextual facts so the agent can reference them.
pub fn build_dock_system_prompt(facts: &[DockFact]) -> String {
    let mut prompt = String::from(
        "<dock_context>\nYou are operating in Dock workbench mode.\nThe canvas is read-only for \
         the user.\nMutate canvas blocks with dock.block.add, dock.block.update, and \
         dock.block.remove tools.\nManage facts with dock.fact.add, dock.fact.update, and \
         dock.fact.remove tools.\nManage annotations with dock.annotation.add, \
         dock.annotation.update, and dock.annotation.remove tools.\nAlways respond with a brief \
         text reply AND the appropriate mutations.\n</dock_context>",
    );

    if !facts.is_empty() {
        prompt.push_str("\n\n<dock_facts>");
        for fact in facts {
            let source_label = match fact.source {
                Actor::Human => "human",
                Actor::Agent => "agent",
            };
            prompt.push_str(&format!("\n- [{}] {}", source_label, fact.content));
        }
        prompt.push_str("\n</dock_facts>");
    }

    prompt
}

/// Build the user prompt with canvas context, annotations, and selected anchor.
pub fn build_dock_user_prompt(
    content: &str,
    blocks: &[DockBlock],
    annotations: &[DockAnnotation],
    selected_anchor: Option<&str>,
) -> String {
    let mut prompt = String::from(content);

    if !blocks.is_empty() {
        prompt.push_str("\n\n<dock_canvas>");
        for block in blocks {
            prompt.push_str(&format!(
                "\n<block id=\"{}\" type=\"{}\">\n{}\n</block>",
                block.id, block.block_type, block.html
            ));
        }
        prompt.push_str("\n</dock_canvas>");
    }

    if !annotations.is_empty() {
        prompt.push_str("\n\n<dock_annotations>");
        for ann in annotations {
            let selection_part = ann
                .selection
                .as_ref()
                .map(|s| format!(" selection=\"{}\"", s.text))
                .unwrap_or_default();
            prompt.push_str(&format!(
                "\n- block={}{}: {}",
                ann.block_id, selection_part, ann.content
            ));
        }
        prompt.push_str("\n</dock_annotations>");
    }

    if let Some(anchor) = selected_anchor {
        prompt.push_str(&format!(
            "\n\n<dock_selected_anchor>{anchor}</dock_selected_anchor>"
        ));
    }

    prompt
}

/// Apply a single mutation to a canvas snapshot in memory.
pub fn apply_mutation(snapshot: &mut DockCanvasSnapshot, mutation: &DockMutation) {
    match mutation.op {
        MutationOp::BlockAdd => {
            if let Some(block) = &mutation.block {
                snapshot.blocks.push(block.clone());
            }
        }
        MutationOp::BlockUpdate => {
            if let Some(block) = &mutation.block {
                if let Some(existing) = snapshot.blocks.iter_mut().find(|b| b.id == block.id) {
                    existing.html = block.html.clone();
                    if !block.block_type.is_empty() {
                        existing.block_type = block.block_type.clone();
                    }
                    if block.diff.is_some() {
                        existing.diff = block.diff.clone();
                    }
                }
            }
        }
        MutationOp::BlockRemove => {
            let remove_id = mutation
                .id
                .as_deref()
                .or(mutation.block.as_ref().map(|b| b.id.as_str()));
            if let Some(id) = remove_id {
                snapshot.blocks.retain(|b| b.id != id);
            }
        }
        MutationOp::FactAdd => {
            if let Some(fact) = &mutation.fact {
                snapshot.facts.push(fact.clone());
            }
        }
        MutationOp::FactUpdate => {
            if let Some(fact) = &mutation.fact {
                if let Some(existing) = snapshot.facts.iter_mut().find(|f| f.id == fact.id) {
                    *existing = fact.clone();
                }
            }
        }
        MutationOp::FactRemove => {
            let remove_id = mutation
                .id
                .as_deref()
                .or(mutation.fact.as_ref().map(|f| f.id.as_str()));
            if let Some(id) = remove_id {
                snapshot.facts.retain(|f| f.id != id);
            }
        }
        // Annotation and session mutations do not affect the canvas snapshot.
        MutationOp::SessionUpsert
        | MutationOp::AnnotationAdd
        | MutationOp::AnnotationUpdate
        | MutationOp::AnnotationRemove => {}
    }
}

/// Generate a unique block ID.
pub fn next_block_id() -> String { format!("blk-{}", ulid::Ulid::new()) }

/// Generate a unique fact ID.
pub fn next_fact_id() -> String { format!("fact-{}", ulid::Ulid::new()) }

/// Extract plain text from an HTML string by stripping tags.
///
/// This is a simple implementation that handles common cases. For production
/// use with untrusted HTML, consider a proper HTML parser.
pub fn text_of_html(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;

    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_system_prompt_no_facts() {
        let prompt = build_dock_system_prompt(&[]);
        assert!(prompt.contains("<dock_context>"));
        assert!(!prompt.contains("<dock_facts>"));
    }

    #[test]
    fn test_build_system_prompt_with_facts() {
        let facts = vec![
            DockFact {
                id:      "f1".into(),
                content: "user likes rust".into(),
                source:  Actor::Human,
            },
            DockFact {
                id:      "f2".into(),
                content: "project uses snafu".into(),
                source:  Actor::Agent,
            },
        ];
        let prompt = build_dock_system_prompt(&facts);
        assert!(prompt.contains("[human] user likes rust"));
        assert!(prompt.contains("[agent] project uses snafu"));
    }

    #[test]
    fn test_build_user_prompt_basic() {
        let prompt = build_dock_user_prompt("hello", &[], &[], None);
        assert_eq!(prompt, "hello");
    }

    #[test]
    fn test_build_user_prompt_with_blocks() {
        let blocks = vec![DockBlock {
            id:         "blk-1".into(),
            block_type: "text".into(),
            html:       "<p>Hello</p>".into(),
            diff:       None,
        }];
        let prompt = build_dock_user_prompt("question", &blocks, &[], None);
        assert!(prompt.contains("<dock_canvas>"));
        assert!(prompt.contains("blk-1"));
        assert!(prompt.contains("<p>Hello</p>"));
    }

    #[test]
    fn test_apply_block_mutations() {
        let mut snap = DockCanvasSnapshot {
            blocks: Vec::new(),
            facts:  Vec::new(),
        };

        // Add
        apply_mutation(
            &mut snap,
            &DockMutation {
                op:         MutationOp::BlockAdd,
                actor:      Actor::Agent,
                block:      Some(DockBlock {
                    id:         "b1".into(),
                    block_type: "text".into(),
                    html:       "<p>hi</p>".into(),
                    diff:       None,
                }),
                fact:       None,
                annotation: None,
                id:         None,
            },
        );
        assert_eq!(snap.blocks.len(), 1);

        // Update
        apply_mutation(
            &mut snap,
            &DockMutation {
                op:         MutationOp::BlockUpdate,
                actor:      Actor::Agent,
                block:      Some(DockBlock {
                    id:         "b1".into(),
                    block_type: "text".into(),
                    html:       "<p>updated</p>".into(),
                    diff:       None,
                }),
                fact:       None,
                annotation: None,
                id:         None,
            },
        );
        assert_eq!(snap.blocks[0].html, "<p>updated</p>");

        // Remove
        apply_mutation(
            &mut snap,
            &DockMutation {
                op:         MutationOp::BlockRemove,
                actor:      Actor::Agent,
                block:      None,
                fact:       None,
                annotation: None,
                id:         Some("b1".into()),
            },
        );
        assert!(snap.blocks.is_empty());
    }

    #[test]
    fn test_text_of_html() {
        assert_eq!(text_of_html("<p>Hello <b>world</b></p>"), "Hello world");
        assert_eq!(text_of_html("no tags"), "no tags");
        assert_eq!(text_of_html("<br/>"), "");
    }

    #[test]
    fn test_id_generators() {
        let bid = next_block_id();
        assert!(bid.starts_with("blk-"));

        let fid = next_fact_id();
        assert!(fid.starts_with("fact-"));

        // IDs should be unique
        assert_ne!(next_block_id(), next_block_id());
    }
}
