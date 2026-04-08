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

//! Accessibility tree → text snapshot conversion.
//!
//! Fetches the full accessibility tree via CDP, filters decorative/invisible
//! nodes, assigns sequential ref IDs, and renders a compact indented text
//! representation suitable for LLM consumption.

use std::collections::HashMap;

use chromiumoxide::{
    Page,
    cdp::browser_protocol::accessibility::{AxNode, GetFullAxTreeParams},
};

use super::{
    error::{BrowserResult, CdpSnafu},
    ref_map::RefMap,
};

/// Result of taking an accessibility tree snapshot.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The rendered text representation of the accessibility tree.
    pub text:    String,
    /// The ref map built during rendering (ref_id → BackendNodeId).
    pub ref_map: RefMap,
}

/// Roles that are purely structural/decorative and should be skipped in the
/// snapshot unless they have meaningful children or a name.
const SKIP_ROLES: &[&str] = &[
    "none",
    "presentation",
    "generic",
    "InlineTextBox",
    "LineBreak",
];

/// Roles that represent interactive or semantically meaningful elements.
/// Nodes with these roles always get a ref ID assigned.
const INTERACTIVE_ROLES: &[&str] = &[
    "link",
    "button",
    "textbox",
    "checkbox",
    "radio",
    "combobox",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "searchbox",
    "textarea",
];

/// Take an accessibility tree snapshot of the current page.
///
/// Returns the formatted text and a fresh [`RefMap`] mapping ref IDs to CDP
/// `BackendNodeId` values for subsequent interactions.
pub async fn take_snapshot(page: &Page, max_bytes: usize) -> BrowserResult<Snapshot> {
    let result = page
        .execute(GetFullAxTreeParams::default())
        .await
        .map_err(|e| {
            CdpSnafu {
                message: e.to_string(),
            }
            .build()
        })?;

    let nodes = result.result.nodes;
    let mut ref_map = RefMap::new();
    let mut output = String::new();

    // Build a parent→children index from the flat node list.
    let node_map: HashMap<String, &AxNode> = nodes
        .iter()
        .map(|n| (n.node_id.inner().clone(), n))
        .collect();

    // Find root node(s): nodes with no parent_id.
    let roots: Vec<&AxNode> = nodes.iter().filter(|n| n.parent_id.is_none()).collect();

    for root in &roots {
        render_node(root, &node_map, &mut ref_map, &mut output, 0, max_bytes);
    }

    // Truncate if over budget.
    if output.len() > max_bytes {
        output.truncate(max_bytes);
        if let Some(last_newline) = output.rfind('\n') {
            output.truncate(last_newline + 1);
        }
        output.push_str("[... truncated, use browser-evaluate to extract specific data]\n");
    }

    Ok(Snapshot {
        text: output,
        ref_map,
    })
}

/// Recursively render a single AX node and its children.
fn render_node(
    node: &AxNode,
    node_map: &HashMap<String, &AxNode>,
    ref_map: &mut RefMap,
    output: &mut String,
    depth: usize,
    max_bytes: usize,
) {
    // Bail early if we're already over budget.
    if output.len() >= max_bytes {
        return;
    }

    // Skip ignored nodes.
    if node.ignored {
        // But still render children — some ignored nodes are structural
        // containers whose children are meaningful.
        render_children(node, node_map, ref_map, output, depth, max_bytes);
        return;
    }

    let role = node
        .role
        .as_ref()
        .and_then(|v| v.value.as_ref())
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let name = node
        .name
        .as_ref()
        .and_then(|v| v.value.as_ref())
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Skip decorative/structural roles without meaningful content.
    if SKIP_ROLES.contains(&role) && name.is_empty() {
        render_children(node, node_map, ref_map, output, depth, max_bytes);
        return;
    }

    // Determine if this node should get a ref ID.
    let is_interactive = INTERACTIVE_ROLES.contains(&role);
    let has_backend_id = node.backend_dom_node_id.is_some();
    let should_ref = (is_interactive || has_backend_id) && !role.is_empty();

    let indent = "  ".repeat(depth);

    if should_ref {
        if let Some(backend_id) = node.backend_dom_node_id {
            let ref_id = ref_map.insert(backend_id);
            output.push_str(&indent);
            output.push_str(&format!("[ref={ref_id}] "));
        } else {
            output.push_str(&indent);
        }
    } else {
        output.push_str(&indent);
    }

    // Render role.
    if !role.is_empty() {
        output.push_str(role);
    }

    // Render name.
    if !name.is_empty() {
        output.push_str(&format!(" \"{name}\""));
    }

    // Render value (for inputs).
    if let Some(value) = node
        .value
        .as_ref()
        .and_then(|v| v.value.as_ref())
        .and_then(|v| v.as_str())
    {
        if !value.is_empty() {
            output.push_str(&format!(" [value=\"{value}\"]"));
        }
    }

    // Render key properties (checked, disabled, href, level, etc.).
    if let Some(props) = &node.properties {
        for prop in props {
            let prop_name = prop.name.as_ref();
            let prop_val = prop
                .value
                .value
                .as_ref()
                .map(|v| {
                    if let Some(s) = v.as_str() {
                        s.to_string()
                    } else {
                        v.to_string()
                    }
                })
                .unwrap_or_default();

            match prop_name {
                "checked" | "disabled" | "required" | "readonly" | "expanded" | "selected"
                | "level" | "url" => {
                    output.push_str(&format!(" [{prop_name}={prop_val}]"));
                }
                _ => {}
            }
        }
    }

    output.push('\n');

    render_children(node, node_map, ref_map, output, depth + 1, max_bytes);
}

/// Render child nodes by looking up child_ids in the node map.
fn render_children(
    node: &AxNode,
    node_map: &HashMap<String, &AxNode>,
    ref_map: &mut RefMap,
    output: &mut String,
    depth: usize,
    max_bytes: usize,
) {
    if let Some(child_ids) = &node.child_ids {
        for child_id in child_ids {
            if let Some(child) = node_map.get(child_id.inner()) {
                render_node(child, node_map, ref_map, output, depth, max_bytes);
            }
        }
    }
}
