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

use std::{
    hash::{Hash, Hasher},
    io::Write,
    process::{Command, Stdio},
};

use rara_kernel::memory::{AnchorTree, SessionBranch};

/// Render an anchor tree to Graphviz DOT text.
pub fn render_dot(tree: &AnchorTree) -> String {
    let mut dot = String::new();
    dot.push_str("digraph anchor_tree {\n");
    dot.push_str("  rankdir=TB;\n");
    dot.push_str("  nodesep=0.5;\n");
    dot.push_str("  ranksep=0.8;\n");
    dot.push_str(
        "  node [shape=box, style=\"rounded,filled\", fontname=\"Helvetica\", fontsize=11];\n",
    );
    dot.push_str("  edge [fontname=\"Helvetica\", fontsize=9];\n");
    dot.push('\n');

    render_branch(&mut dot, &tree.root, &tree.current_session);

    dot.push_str("}\n");
    dot
}

/// Return the node ID that represents the session in the graph. When the
/// session has anchors the caller should use the first/last anchor node;
/// when it has none a dedicated placeholder is emitted here.
fn ensure_session_node(
    dot: &mut String,
    branch: &SessionBranch,
    is_current: bool,
    session_label: &str,
) -> Option<String> {
    if !branch.anchors.is_empty() {
        return None; // anchors will be rendered in the main loop
    }
    // Emit a single placeholder node so the graph is never blank.
    let node = node_id(&branch.session_key, "__empty__");
    let label = format!(
        "[{}]\\n({})\\n(no anchors)",
        escape_dot(session_label),
        escape_dot(&branch.session_key),
    );
    let fill = if is_current { "#d9f4dd" } else { "#f8f9fa" };
    let border = if is_current { "#1f8f3a" } else { "#5a6773" };
    dot.push_str(&format!(
        "  {node} [label=\"{label}\", fillcolor=\"{fill}\", color=\"{border}\"];\n"
    ));
    Some(node)
}

fn render_branch(dot: &mut String, branch: &SessionBranch, current_session: &str) {
    let is_current = branch.session_key == current_session;
    let session_label = branch.title.as_deref().unwrap_or(&branch.session_key);

    // If the session has no anchors, emit a placeholder so the graph is visible.
    let placeholder = ensure_session_node(dot, branch, is_current, session_label);

    let mut previous_node_id: Option<String> = placeholder;
    for anchor in &branch.anchors {
        // One node per anchor, chained in-session by append order.
        let node = node_id(&branch.session_key, &anchor.name);
        let mut label = format!(
            "[{}]\\n({})\\n{}",
            escape_dot(session_label),
            escape_dot(&branch.session_key),
            escape_dot(&anchor.name)
        );
        if let Some(summary) = &anchor.summary {
            label.push_str("\\n");
            label.push_str(&escape_dot(&truncate(summary, 40)));
        }

        let fill = if is_current { "#d9f4dd" } else { "#f8f9fa" };
        let border = if is_current { "#1f8f3a" } else { "#5a6773" };
        dot.push_str(&format!(
            "  {node} [label=\"{label}\", fillcolor=\"{fill}\", color=\"{border}\"];\n"
        ));

        if let Some(prev) = previous_node_id {
            dot.push_str(&format!("  {prev} -> {node};\n"));
        }
        previous_node_id = Some(node);
    }

    for fork in &branch.forks {
        let parent = node_id(&branch.session_key, &fork.at_anchor);
        // Resolve the first node of the child branch — either its first
        // anchor or, when the child has none, the placeholder node.
        let child_first = fork
            .branch
            .anchors
            .first()
            .map(|a| node_id(&fork.branch.session_key, &a.name))
            .unwrap_or_else(|| node_id(&fork.branch.session_key, "__empty__"));
        dot.push_str(&format!(
            "  {parent} -> {child_first} [style=dashed, color=\"#1f6feb\", label=\"fork\"];\n"
        ));
        render_branch(dot, &fork.branch, current_session);
    }
}

fn node_id(session_key: &str, anchor_name: &str) -> String {
    // Deterministic within a single build to avoid node-id collisions.
    // Note: DefaultHasher output may change across Rust versions.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    session_key.hash(&mut hasher);
    "::".hash(&mut hasher);
    anchor_name.hash(&mut hasher);
    format!("n_{:x}", hasher.finish())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        return s.to_owned();
    }
    let shortened: String = s.chars().take(max_len).collect();
    format!("{shortened}...")
}

fn escape_dot(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Render DOT to PNG bytes by invoking the `dot` binary.
pub fn render_png(dot: &str) -> Result<Vec<u8>, String> {
    let mut child = Command::new("dot")
        .args(["-Tpng"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn dot: {e}"))?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(dot.as_bytes())
            .map_err(|e| format!("failed to write DOT input: {e}"))?;
    } else {
        return Err("failed to open dot stdin".to_owned());
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed to wait for dot process: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "dot failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use rara_kernel::memory::{AnchorNode, AnchorTree, ForkEdge, SessionBranch};

    use super::*;

    fn sample_tree() -> AnchorTree {
        AnchorTree {
            root:            SessionBranch {
                session_key: "root".into(),
                title:       Some("Root Session".into()),
                anchors:     vec![
                    AnchorNode {
                        name:     "session/start".into(),
                        summary:  None,
                        entry_id: 1,
                    },
                    AnchorNode {
                        name:     "topic/a".into(),
                        summary:  Some("Discussed A".into()),
                        entry_id: 5,
                    },
                ],
                forks:       vec![ForkEdge {
                    at_anchor: "topic/a".into(),
                    branch:    SessionBranch {
                        session_key: "fork-1".into(),
                        title:       Some("Fork 1".into()),
                        anchors:     vec![AnchorNode {
                            name:     "session/start".into(),
                            summary:  None,
                            entry_id: 1,
                        }],
                        forks:       vec![],
                    },
                }],
            },
            current_session: "fork-1".into(),
        }
    }

    #[test]
    fn generates_valid_dot() {
        let dot = render_dot(&sample_tree());
        assert!(dot.contains("digraph"));
        assert!(dot.contains("topic/a"));
        assert!(dot.contains("Fork 1"));
        assert!(dot.contains("fork-1"));
    }

    #[test]
    fn empty_anchors_produces_placeholder_node() {
        let tree = AnchorTree {
            root:            SessionBranch {
                session_key: "solo".into(),
                title:       Some("Empty Session".into()),
                anchors:     vec![],
                forks:       vec![],
            },
            current_session: "solo".into(),
        };
        let dot = render_dot(&tree);
        assert!(dot.contains("digraph"));
        assert!(dot.contains("Empty Session"));
        assert!(dot.contains("(no anchors)"));
    }

    #[test]
    fn fork_to_empty_child_produces_edge() {
        let tree = AnchorTree {
            root:            SessionBranch {
                session_key: "root".into(),
                title:       Some("Root".into()),
                anchors:     vec![AnchorNode {
                    name:     "session/start".into(),
                    summary:  None,
                    entry_id: 1,
                }],
                forks:       vec![ForkEdge {
                    at_anchor: "session/start".into(),
                    branch:    SessionBranch {
                        session_key: "child".into(),
                        title:       None,
                        anchors:     vec![],
                        forks:       vec![],
                    },
                }],
            },
            current_session: "root".into(),
        };
        let dot = render_dot(&tree);
        assert!(dot.contains("fork"));
        assert!(dot.contains("(no anchors)"));
        // The fork edge should connect to the placeholder node.
        assert!(dot.contains("style=dashed"));
    }
}
