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

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap},
};

use crate::top::app::{App, Tab};
use crate::top::types::KernelEventEnvelope;

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
    render_help(frame, help_area);

    // Event detail popup (rendered on top of everything).
    if app.show_event_detail && app.tab == Tab::Events {
        if let Some(envelope) = app.kernel_events.iter().rev().nth(app.selected_row) {
            render_event_detail_popup(frame, frame.area(), envelope);
        }
    }
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
            "  Processes:{}  Tokens:{}  Spawned:{}  Completed:{}  Failed:{}  Up:{}",
            s.active_processes,
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
        Tab::Processes => render_processes_table(frame, area, app),
        Tab::Agents => render_agents_table(frame, area, app),
        Tab::Approvals => render_approvals_table(frame, area, app),
        Tab::Audit => render_audit_table(frame, area, app),
        Tab::Events => render_events_table(frame, area, app),
    }
}

fn render_processes_table(frame: &mut Frame, area: Rect, app: &App) {
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
        .processes
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
            .title(format!(" Processes ({}) ", app.processes.len())),
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

fn render_events_table(frame: &mut Frame, area: Rect, app: &App) {
    let header = Row::new(vec![
        Cell::from("Timestamp"),
        Cell::from("Event"),
        Cell::from("Priority"),
        Cell::from("Agent"),
        Cell::from("Summary"),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    // Visible height excluding border (top+bottom) and header row.
    let visible_rows = area.height.saturating_sub(3) as usize;

    // Compute scroll_offset so that selected_row is always visible.
    let scroll = if app.selected_row < visible_rows {
        0
    } else {
        app.selected_row - visible_rows + 1
    };

    let selected_style = Style::default()
        .bg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);

    let rows: Vec<Row> = app
        .kernel_events
        .iter()
        .rev()
        .enumerate()
        .skip(scroll)
        .take(visible_rows)
        .map(|(i, envelope)| {
            let c = &envelope.common;
            let row = Row::new(vec![
                Cell::from(c.timestamp.as_str()),
                Cell::from(c.event_type.as_str()),
                Cell::from(priority_styled(&c.priority)),
                Cell::from(
                    c.agent_id
                        .as_deref()
                        .map(short_id)
                        .unwrap_or_else(|| "-".to_string()),
                ),
                Cell::from(c.summary.chars().take(80).collect::<String>()),
            ]);
            if i == app.selected_row {
                row.style(selected_style)
            } else {
                row
            }
        })
        .collect();

    let widths = [
        Constraint::Length(22),
        Constraint::Length(20),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Fill(1),
    ];

    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Events ({}) ", app.kernel_events.len())),
    );

    frame.render_widget(table, area);
}

// ---------------------------------------------------------------------------
// Help bar
// ---------------------------------------------------------------------------

fn render_help(frame: &mut Frame, area: Rect) {
    let help = Line::from(vec![
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::raw(":Quit  "),
        Span::styled("1-5", Style::default().fg(Color::Yellow)),
        Span::raw(":Tab  "),
        Span::styled("\u{2191}\u{2193}", Style::default().fg(Color::Yellow)),
        Span::raw(":Scroll  "),
        Span::styled("r", Style::default().fg(Color::Yellow)),
        Span::raw(":Refresh"),
    ]);
    frame.render_widget(Paragraph::new(help), area);
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

fn priority_styled(priority: &str) -> Span<'_> {
    let color = match priority {
        "critical" => Color::Red,
        "normal" => Color::Cyan,
        "low" => Color::Gray,
        _ => Color::White,
    };
    Span::styled(priority, Style::default().fg(color))
}

// ---------------------------------------------------------------------------
// Event detail popup
// ---------------------------------------------------------------------------

fn render_event_detail_popup(frame: &mut Frame, area: Rect, envelope: &KernelEventEnvelope) {
    let popup_area = centered_rect(70, 60, area);

    // Clear the background behind the popup.
    frame.render_widget(Clear, popup_area);

    let c = &envelope.common;
    let event_json = serde_json::to_string_pretty(&envelope.event).unwrap_or_default();

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Timestamp: ", Style::default().fg(Color::Yellow)),
            Span::raw(&c.timestamp),
        ]),
        Line::from(vec![
            Span::styled("Event:     ", Style::default().fg(Color::Yellow)),
            Span::raw(&c.event_type),
        ]),
        Line::from(vec![
            Span::styled("Priority:  ", Style::default().fg(Color::Yellow)),
            priority_styled(&c.priority),
        ]),
        Line::from(vec![
            Span::styled("Agent:     ", Style::default().fg(Color::Yellow)),
            Span::raw(c.agent_id.as_deref().unwrap_or("-")),
        ]),
        Line::from(vec![
            Span::styled("Summary:   ", Style::default().fg(Color::Yellow)),
            Span::raw(&c.summary),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "Details:",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
    ];

    for json_line in event_json.lines() {
        lines.push(Line::raw(json_line.to_string()));
    }

    let paragraph = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Event Detail (Enter/Esc to close) ")
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, popup_area);
}

/// Return a centered `Rect` of `percent_x`% width and `percent_y`% height.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let [_, v_center, _] = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .areas(area);

    let [_, h_center, _] = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .areas(v_center);

    h_center
}
