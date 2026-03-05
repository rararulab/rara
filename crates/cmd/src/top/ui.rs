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

use std::time::Instant;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

use crate::top::{
    app::{App, Tab},
    types::{PanelFocus, SessionView},
};

/// Render the full TUI.
pub fn render(frame: &mut Frame, app: &App) {
    let [header_area, tab_area, content_area, help_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_header(frame, header_area, app);
    render_tab_bar(frame, tab_area, app);
    render_content(frame, content_area, app);
    render_help(frame, help_area, app);
}

// ---------------------------------------------------------------------------
// Header — system overview
// ---------------------------------------------------------------------------

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let status_icon = if app.connected {
        Span::styled(" CONNECTED ", Style::default().fg(Color::Green))
    } else {
        Span::styled(
            " DISCONNECTED ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )
    };

    let stats_text = if let Some(ref s) = app.stats {
        format!(
            "  Sessions:{}  Tokens:{}  Spawned:{}  Completed:{}  Failed:{}  Up:{}",
            s.active_sessions,
            format_tokens(s.total_tokens_consumed),
            s.total_spawned,
            s.total_completed,
            s.total_failed,
            format_uptime(s.uptime_ms),
        )
    } else {
        String::from("  (no data)")
    };

    let line = Line::from(vec![
        Span::styled(
            "rara-top",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        status_icon,
        Span::raw(stats_text),
    ]);

    frame.render_widget(Paragraph::new(line), area);
}

// ---------------------------------------------------------------------------
// Tab bar
// ---------------------------------------------------------------------------

fn render_tab_bar(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = Vec::new();
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let label = format!(" {} {} ", i + 1, tab.title());
        let style = if *tab == app.tab {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---------------------------------------------------------------------------
// Content area (table per tab)
// ---------------------------------------------------------------------------

fn render_content(frame: &mut Frame, area: Rect, app: &App) {
    if let Some(ref err) = app.error {
        let msg = Paragraph::new(format!("Error: {err}"))
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title("Error"));
        frame.render_widget(msg, area);
        return;
    }

    match app.tab {
        Tab::Sessions => render_sessions_table(frame, area, app),
        Tab::Agents => render_agents_table(frame, area, app),
        Tab::Approvals => render_approvals_table(frame, area, app),
        Tab::Audit => render_audit_table(frame, area, app),
        Tab::SessionDetails => render_session_details_tab(frame, area, app),
    }
}

fn render_sessions_table(frame: &mut Frame, area: Rect, app: &App) {
    let header = Row::new(vec![
        Cell::from("ID"),
        Cell::from("Name"),
        Cell::from("State"),
        Cell::from("Parent"),
        Cell::from("Uptime"),
        Cell::from("LLM"),
        Cell::from("Tokens"),
        Cell::from("Tools"),
        Cell::from("Msgs"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = app
        .sessions_list
        .iter()
        .skip(app.scroll_offset)
        .map(|p| {
            Row::new(vec![
                Cell::from(short_id(&p.agent_id)),
                Cell::from(p.name.as_str()),
                Cell::from(state_styled(&p.state)),
                Cell::from(p.parent_id.as_deref().map(short_id).unwrap_or_default()),
                Cell::from(format_uptime(p.uptime_ms)),
                Cell::from(p.metrics.llm_calls.to_string()),
                Cell::from(format_tokens(p.metrics.tokens_consumed)),
                Cell::from(p.metrics.tool_calls.to_string()),
                Cell::from(p.metrics.messages_received.to_string()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(10),
        Constraint::Length(16),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(6),
    ];

    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Sessions ({}) ", app.sessions_list.len())),
    );

    frame.render_widget(table, area);
}

fn render_agents_table(frame: &mut Frame, area: Rect, app: &App) {
    let header = Row::new(vec![
        Cell::from("Name"),
        Cell::from("Role"),
        Cell::from("Description"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = app
        .agents
        .iter()
        .skip(app.scroll_offset)
        .map(|a| {
            Row::new(vec![
                Cell::from(a.name.as_str()),
                Cell::from(a.role.as_deref().unwrap_or("-")),
                Cell::from(
                    a.description
                        .as_deref()
                        .unwrap_or("-")
                        .chars()
                        .take(60)
                        .collect::<String>(),
                ),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(20),
        Constraint::Length(12),
        Constraint::Fill(1),
    ];

    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Agents ({}) ", app.agents.len())),
    );

    frame.render_widget(table, area);
}

fn render_approvals_table(frame: &mut Frame, area: Rect, app: &App) {
    let header = Row::new(vec![
        Cell::from("ID"),
        Cell::from("Agent"),
        Cell::from("Tool"),
        Cell::from("Risk"),
        Cell::from("Requested"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = app
        .approvals
        .iter()
        .skip(app.scroll_offset)
        .map(|a| {
            let risk_style = match a.risk_level.as_str() {
                "High" | "high" => Style::default().fg(Color::Red),
                "Medium" | "medium" => Style::default().fg(Color::Yellow),
                _ => Style::default().fg(Color::Green),
            };
            Row::new(vec![
                Cell::from(short_id(&a.id)),
                Cell::from(short_id(&a.agent_id)),
                Cell::from(a.tool_name.as_str()),
                Cell::from(Span::styled(a.risk_level.as_str(), risk_style)),
                Cell::from(a.requested_at.as_str()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(20),
        Constraint::Length(8),
        Constraint::Fill(1),
    ];

    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Approvals ({}) ", app.approvals.len())),
    );

    frame.render_widget(table, area);
}

fn render_audit_table(frame: &mut Frame, area: Rect, app: &App) {
    let header = Row::new(vec![
        Cell::from("Timestamp"),
        Cell::from("Agent"),
        Cell::from("Event"),
        Cell::from("Details"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = app
        .audit
        .iter()
        .rev()
        .skip(app.scroll_offset)
        .map(|e| {
            let details_str = match &e.details {
                serde_json::Value::String(s) => s.chars().take(60).collect::<String>(),
                other => {
                    let s = other.to_string();
                    s.chars().take(60).collect()
                }
            };
            Row::new(vec![
                Cell::from(e.timestamp.as_str()),
                Cell::from(short_id(&e.agent_id)),
                Cell::from(e.event_type.as_str()),
                Cell::from(details_str),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(22),
        Constraint::Length(10),
        Constraint::Length(16),
        Constraint::Fill(1),
    ];

    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Audit ({}) ", app.audit.len())),
    );

    frame.render_widget(table, area);
}

// ---------------------------------------------------------------------------
// Sessions tab — 3-panel layout
// ---------------------------------------------------------------------------

fn render_session_details_tab(frame: &mut Frame, area: Rect, app: &App) {
    let [list_area, detail_area] =
        Layout::horizontal([Constraint::Percentage(20), Constraint::Percentage(80)]).areas(area);

    render_session_list(frame, list_area, app);

    if let Some(session_view) = app.session_state.selected_session_view() {
        let [gantt_area, tree_area] =
            Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(detail_area);

        render_gantt(
            frame,
            gantt_area,
            session_view,
            app.session_state.gantt_selected,
            app.session_state.focus == PanelFocus::Gantt,
        );
        render_session_tree(
            frame,
            tree_area,
            session_view,
            &app.sessions_list,
            app.session_state.tree_selected,
            app.session_state.focus == PanelFocus::SessionTree,
        );
    } else {
        let msg = Paragraph::new("No sessions available")
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL).title(" Details "));
        frame.render_widget(msg, detail_area);
    }
}

fn render_session_list(frame: &mut Frame, area: Rect, app: &App) {
    let ss = &app.session_state;
    let is_focused = ss.focus == PanelFocus::SessionList;

    let header = Row::new(vec![
        Cell::from("Session"),
        Cell::from("Agents"),
        Cell::from("Last"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let now = Instant::now();
    let selected_style = Style::default()
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);

    let rows: Vec<Row> = ss
        .sessions
        .values()
        .enumerate()
        .map(|(i, sv)| {
            let elapsed = now.duration_since(sv.last_event);
            let last_str = format_duration_ago(elapsed);
            let row = Row::new(vec![
                Cell::from(short_id(&sv.session_id)),
                Cell::from(sv.agents.len().to_string()),
                Cell::from(last_str),
            ]);
            if i == ss.selected_session {
                row.style(selected_style)
            } else {
                row
            }
        })
        .collect();

    let widths = [
        Constraint::Length(10),
        Constraint::Length(6),
        Constraint::Fill(1),
    ];

    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Sessions ({}) ", ss.sessions.len()))
            .border_style(border_style),
    );

    frame.render_widget(table, area);
}

fn render_gantt(
    frame: &mut Frame,
    area: Rect,
    session_view: &SessionView,
    selected: usize,
    is_focused: bool,
) {
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Gantt Timeline ")
        .border_style(border_style);

    // Inner area for content (excluding borders).
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if session_view.agents.is_empty() || inner.width < 16 || inner.height < 1 {
        return;
    }

    let now = Instant::now();
    let session_start = session_view.first_seen;
    let session_duration = now.duration_since(session_start).as_secs_f64().max(1.0);

    let label_width: u16 = 12;
    let bar_width = inner.width.saturating_sub(label_width + 1) as usize; // +1 for separator

    if bar_width < 2 {
        return;
    }

    // Build depth-first ordered agents.
    let ordered = depth_first_agents(session_view);

    let selected_style = Style::default()
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);

    // Reserve last row for time axis.
    let chart_height = inner.height.saturating_sub(1);

    for (i, agent_id) in ordered.iter().enumerate() {
        if i as u16 >= chart_height {
            break;
        }
        let y = inner.y + i as u16;

        let Some(timeline) = session_view.agents.get(agent_id.as_str()) else {
            continue;
        };

        // Label column.
        let name: String = timeline.name.chars().take(label_width as usize).collect();
        let label_span = Span::styled(
            format!("{:<width$}", name, width = label_width as usize),
            Style::default().fg(Color::White),
        );
        let label_area = Rect::new(inner.x, y, label_width, 1);
        frame.render_widget(Paragraph::new(Line::from(label_span)), label_area);

        // Separator.
        let sep_area = Rect::new(inner.x + label_width, y, 1, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(
                "\u{2502}",
                Style::default().fg(Color::DarkGray),
            )),
            sep_area,
        );

        // Bar.
        let agent_start_frac =
            timeline.start.duration_since(session_start).as_secs_f64() / session_duration;
        let agent_end_frac = match timeline.end {
            Some(end) => end.duration_since(session_start).as_secs_f64() / session_duration,
            None => now.duration_since(session_start).as_secs_f64() / session_duration,
        };

        let bar_start = (agent_start_frac * bar_width as f64).round() as usize;
        let bar_end = (agent_end_frac * bar_width as f64).round() as usize;
        let bar_start = bar_start.min(bar_width);
        let bar_end = bar_end.min(bar_width).max(bar_start + 1); // at least 1 char

        let bar_color = match timeline.state.as_str() {
            "Running" | "running" => Color::Green,
            "Failed" | "failed" => Color::Red,
            "Idle" | "idle" => Color::Cyan,
            _ => Color::Gray,
        };

        // Build bar with inline metrics overlay.
        let bar_len = bar_end - bar_start;
        let duration_secs = match timeline.end {
            Some(end) => end.duration_since(timeline.start).as_secs(),
            None => now.duration_since(timeline.start).as_secs(),
        };
        let overlay = format!(
            " {}  {}tok  {}llm ",
            format_uptime(duration_secs * 1000),
            format_tokens(timeline.metrics.tokens_consumed),
            timeline.metrics.llm_calls,
        );
        let overlay_len = overlay.len();

        let mut bar_chars = String::with_capacity(bar_width);
        for col in 0..bar_width {
            if col >= bar_start && col < bar_end {
                let offset = col - bar_start;
                if offset < overlay_len && bar_len >= overlay_len {
                    // Overlay metrics text on the bar.
                    bar_chars.push(overlay.as_bytes()[offset] as char);
                } else {
                    bar_chars.push('\u{2588}'); // Full block
                }
            } else {
                bar_chars.push(' ');
            }
        }

        let bar_style = if i == selected {
            selected_style.fg(bar_color)
        } else {
            Style::default().fg(bar_color)
        };

        let bar_area = Rect::new(inner.x + label_width + 1, y, bar_width as u16, 1);
        frame.render_widget(Paragraph::new(Span::styled(bar_chars, bar_style)), bar_area);
    }

    // Time axis on the last row.
    if inner.height > 1 {
        let axis_y = inner.y + inner.height - 1;

        // Label area: empty padding.
        let axis_label_area = Rect::new(inner.x, axis_y, label_width + 1, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!("{:<width$}\u{2502}", "", width = label_width as usize),
                Style::default().fg(Color::DarkGray),
            )),
            axis_label_area,
        );

        // Build time axis marks.
        let num_marks = 5usize;
        let mut axis = vec!['\u{2500}'; bar_width]; // ─
        for m in 0..=num_marks {
            let pos = (m as f64 / num_marks as f64 * (bar_width - 1) as f64).round() as usize;
            if pos < bar_width {
                axis[pos] = '\u{253C}'; // ┼
                let t = session_duration * m as f64 / num_marks as f64;
                let label = format_uptime((t * 1000.0) as u64);
                // Write label starting at mark position.
                for (j, ch) in label.chars().enumerate() {
                    if pos + j + 1 < bar_width {
                        axis[pos + j + 1] = ch;
                    }
                }
            }
        }

        let axis_str: String = axis.into_iter().collect();
        let axis_area = Rect::new(inner.x + label_width + 1, axis_y, bar_width as u16, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(axis_str, Style::default().fg(Color::DarkGray))),
            axis_area,
        );
    }
}

fn render_session_tree(
    frame: &mut Frame,
    area: Rect,
    session_view: &SessionView,
    _sessions: &[crate::top::types::SessionStats],
    selected: usize,
    is_focused: bool,
) {
    let border_style = if is_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Session Tree ")
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if session_view.agents.is_empty() || inner.height < 1 {
        return;
    }

    // Build tree using parent_id relationships.
    let ordered = depth_first_agents(session_view);

    // For tree connectors, compute depth per agent.
    let mut lines: Vec<Line> = Vec::new();
    for (i, agent_id) in ordered.iter().enumerate() {
        let Some(timeline) = session_view.agents.get(agent_id.as_str()) else {
            continue;
        };

        let depth = compute_depth(session_view, agent_id);
        let is_last_child = is_last_sibling(session_view, agent_id, &ordered);

        // Build prefix with tree connectors.
        let mut prefix = String::new();
        if depth > 0 {
            for _ in 0..depth.saturating_sub(1) {
                prefix.push_str("  ");
            }
            if is_last_child {
                prefix.push_str("\u{2514}\u{2500} "); // └─
            } else {
                prefix.push_str("\u{251C}\u{2500} "); // ├─
            }
        }

        let state_color = match timeline.state.as_str() {
            "Running" | "running" => Color::Green,
            "Failed" | "failed" => Color::Red,
            "Completed" | "completed" => Color::Gray,
            "Idle" | "idle" => Color::Cyan,
            _ => Color::White,
        };

        let tokens_str = format_tokens(timeline.metrics.tokens_consumed);

        let spans = vec![
            Span::raw(prefix),
            Span::styled(
                &timeline.name,
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(&timeline.state, Style::default().fg(state_color)),
            Span::raw("  "),
            Span::raw(format!("{tokens_str} tokens")),
        ];

        let mut line = Line::from(spans);

        if i == selected {
            // Apply selected style by wrapping spans with background.
            line = Line::from(
                line.spans
                    .into_iter()
                    .map(|s| {
                        Span::styled(
                            s.content,
                            s.style.bg(Color::DarkGray).add_modifier(Modifier::BOLD),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
        }

        lines.push(line);
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

// ---------------------------------------------------------------------------
// Help bar
// ---------------------------------------------------------------------------

fn render_help(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::raw(":Quit  "),
        Span::styled("1-5", Style::default().fg(Color::Yellow)),
        Span::raw(":Tab  "),
        Span::styled("\u{2191}\u{2193}", Style::default().fg(Color::Yellow)),
        Span::raw(":Select  "),
        Span::styled("r", Style::default().fg(Color::Yellow)),
        Span::raw(":Refresh"),
    ];

    if app.tab == Tab::SessionDetails {
        spans.push(Span::raw("  "));
        Span::styled("Tab", Style::default().fg(Color::Yellow));
        spans.push(Span::styled("Tab", Style::default().fg(Color::Yellow)));
        spans.push(Span::raw(":Panel  "));
        spans.push(Span::styled("Enter", Style::default().fg(Color::Yellow)));
        spans.push(Span::raw(":Detail"));
    }

    let help = Line::from(spans);
    frame.render_widget(Paragraph::new(help), area);
}

// ---------------------------------------------------------------------------
// Tree helpers
// ---------------------------------------------------------------------------

/// Return agent IDs in depth-first order based on parent_id relationships.
fn depth_first_agents(session_view: &SessionView) -> Vec<String> {
    let agents = &session_view.agents;

    // Find roots: agents whose parent_id is None or not in this session.
    let roots: Vec<String> = agents
        .values()
        .filter(|a| {
            a.parent_id.is_none() || !agents.contains_key(a.parent_id.as_deref().unwrap_or(""))
        })
        .map(|a| a.agent_id.clone())
        .collect();

    let mut result = Vec::new();
    for root in roots {
        dfs_collect(&root, agents, &mut result);
    }

    // Add any agents not yet visited (orphans with broken parent refs).
    for a in agents.values() {
        if !result.contains(&a.agent_id) {
            result.push(a.agent_id.clone());
        }
    }

    result
}

fn dfs_collect(
    agent_id: &str,
    agents: &indexmap::IndexMap<String, crate::top::types::AgentTimeline>,
    result: &mut Vec<String>,
) {
    result.push(agent_id.to_string());
    // Find children of this agent.
    let children: Vec<String> = agents
        .values()
        .filter(|a| a.parent_id.as_deref() == Some(agent_id))
        .map(|a| a.agent_id.clone())
        .collect();
    for child in children {
        dfs_collect(&child, agents, result);
    }
}

fn compute_depth(session_view: &SessionView, agent_id: &str) -> usize {
    let mut depth = 0;
    let mut current = agent_id.to_string();
    while let Some(timeline) = session_view.agents.get(&current) {
        match &timeline.parent_id {
            Some(pid) if session_view.agents.contains_key(pid) => {
                depth += 1;
                current = pid.clone();
            }
            _ => break,
        }
    }
    depth
}

fn is_last_sibling(session_view: &SessionView, agent_id: &str, ordered: &[String]) -> bool {
    let agents = &session_view.agents;
    let Some(timeline) = agents.get(agent_id) else {
        return true;
    };

    // Find all siblings (same parent_id).
    let parent = &timeline.parent_id;
    let siblings: Vec<&String> = agents
        .values()
        .filter(|a| &a.parent_id == parent && agents.contains_key(&a.agent_id))
        .map(|a| &a.agent_id)
        .collect();

    // The last sibling in the ordered list is the "last" one.
    let mut last_in_order: Option<&String> = None;
    for id in ordered {
        if siblings.contains(&id) {
            last_in_order = Some(id);
        }
    }

    last_in_order.map_or(true, |last| last == agent_id)
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Shorten an ID (UUID/ULID) to its first 8 characters.
fn short_id(id: &str) -> String {
    if id.len() > 8 {
        format!("{}...", &id[..8])
    } else {
        id.to_owned()
    }
}

/// Format milliseconds into a human-readable `Xh Ym Zs` string.
fn format_uptime(ms: u64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

/// Format a token count into human-readable form (e.g. `45.2k`).
fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

/// Format a Duration into a "Xs ago" / "Xm ago" style string.
fn format_duration_ago(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}

/// Apply color to the state string.
fn state_styled(state: &str) -> Span<'_> {
    let color = match state {
        "Running" | "running" => Color::Green,
        "Idle" | "idle" => Color::Cyan,
        "Paused" | "paused" => Color::Yellow,
        "Failed" | "failed" => Color::Red,
        "Completed" | "completed" => Color::Gray,
        _ => Color::White,
    };
    Span::styled(state, Style::default().fg(color))
}
