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
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
};

use crate::chat::{
    app::{ChatMessage, ChatState, Role, ToolInfo},
    theme,
};

pub fn render(frame: &mut Frame, state: &ChatState, area: Rect) {
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            format!(" {} ", state.agent_name),
            theme::title_style(),
        )]))
        .title_alignment(Alignment::Left)
        .title_bottom(Line::from(vec![Span::styled(
            format!(" {} — {} ", state.model_label, state.mode_label),
            theme::dim_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);

    draw_messages(frame, chunks[0], state);

    let separator = Paragraph::new("─".repeat(chunks[1].width as usize))
        .style(Style::default().fg(theme::BORDER));
    frame.render_widget(separator, chunks[1]);

    let input_line = if state.is_streaming {
        let mut spans = vec![
            Span::styled(" > ", Style::default().fg(theme::YELLOW)),
            Span::raw(&state.input),
            Span::styled(
                "█",
                Style::default()
                    .fg(theme::YELLOW)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ];
        if !state.staged_queue.is_empty() {
            spans.push(Span::styled(
                format!("  ({} staged)", state.staged_queue.len()),
                Style::default().fg(theme::PURPLE),
            ));
        }
        Paragraph::new(Line::from(spans))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled(" > ", theme::input_style()),
            Span::raw(&state.input),
            Span::styled(
                "█",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ]))
    };
    frame.render_widget(input_line, chunks[2]);

    let hints = if state.is_streaming {
        "    [Enter] Stage  [↑↓/PgUp/PgDn] Scroll  [Esc] Stop"
    } else {
        "    [Enter] Send  [/help] Commands  [↑↓/PgUp/PgDn] Scroll  [Esc] Back"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(hints, theme::hint_style())])),
        chunks[3],
    );
}

fn draw_messages(frame: &mut Frame, area: Rect, state: &ChatState) {
    let width = area.width as usize;
    if width < 4 {
        return;
    }

    let mut lines = Vec::new();

    if state.messages.is_empty() && state.streaming_text.is_empty() && !state.thinking {
        let blank_lines = area.height.saturating_sub(4) / 2;
        for _ in 0..blank_lines {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![Span::styled(
            "  Send a message to start chatting.",
            theme::dim_style(),
        )]));
        lines.push(Line::from(vec![Span::styled(
            "  Type /help for available commands.",
            theme::dim_style(),
        )]));
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    for message in &state.messages {
        draw_message_lines(&mut lines, message, width, state.spinner_frame);
    }

    if !state.streaming_text.is_empty() {
        lines.push(Line::from(""));
        for wrapped in wrap_text(&state.streaming_text, width.saturating_sub(4)) {
            lines.push(Line::from(vec![Span::raw("  "), Span::raw(wrapped)]));
        }
    }

    if state.thinking {
        let spinner = theme::SPINNER_FRAMES[state.spinner_frame];
        lines.push(Line::from(vec![
            Span::styled(format!("  {spinner} "), Style::default().fg(theme::CYAN)),
            Span::styled(state.loading_hint.clone(), Style::default().fg(theme::DIM)),
        ]));
    }

    if let Some(tool_name) = &state.active_tool {
        let spinner = theme::SPINNER_FRAMES[state.spinner_frame];
        lines.push(Line::from(vec![
            Span::styled(format!("  {spinner} "), Style::default().fg(theme::RED)),
            Span::styled(tool_name.clone(), Style::default().fg(theme::YELLOW)),
        ]));
    }

    if state.is_streaming && state.streaming_chars > 0 {
        lines.push(Line::from(vec![Span::styled(
            format!("  ~{} tokens", state.streaming_chars / 4),
            theme::dim_style(),
        )]));
    }

    if let Some((input, output)) = state.last_tokens
        && (input > 0 || output > 0)
    {
        let cost = match state.last_cost_usd {
            Some(value) if value > 0.0 => format!(" | ${value:.4}"),
            _ => String::new(),
        };
        lines.push(Line::from(vec![Span::styled(
            format!("  [tokens: {input} in / {output} out{cost}]"),
            theme::dim_style(),
        )]));
    }

    if let Some(message) = &state.status_msg {
        lines.push(Line::from(vec![Span::styled(
            format!("  {message}"),
            Style::default().fg(theme::RED),
        )]));
    }

    let total_lines = lines.len() as u16;
    let visible_height = area.height;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll = max_scroll
        .saturating_sub(state.scroll_offset)
        .min(max_scroll);

    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);

    if state.scroll_offset > 0 && total_lines > visible_height {
        let above = scroll;
        let below = total_lines.saturating_sub(scroll + visible_height);
        let indicator = format!("{}↑ {}↓", above, below);
        let indicator_area = Rect {
            x:      area.x + area.width.saturating_sub(indicator.len() as u16 + 1),
            y:      area.y + area.height.saturating_sub(1),
            width:  indicator.len() as u16,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Span::styled(indicator, theme::dim_style())),
            indicator_area,
        );
    }
}

fn draw_message_lines(
    lines: &mut Vec<Line<'static>>,
    message: &ChatMessage,
    width: usize,
    spinner_frame: usize,
) {
    match message.role {
        Role::User => {
            lines.push(Line::from(""));
            for (index, wrapped) in wrap_text(&message.text, width.saturating_sub(6))
                .into_iter()
                .enumerate()
            {
                if index == 0 {
                    lines.push(Line::from(vec![
                        Span::styled("  ❯ ", theme::input_style()),
                        Span::styled(wrapped, Style::default().fg(theme::TEXT_PRIMARY)),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(wrapped, Style::default().fg(theme::TEXT_PRIMARY)),
                    ]));
                }
            }
        }
        Role::Agent => {
            lines.push(Line::from(""));
            for wrapped in wrap_text(&message.text, width.saturating_sub(4)) {
                lines.push(Line::from(vec![Span::raw("  "), Span::raw(wrapped)]));
            }
        }
        Role::System => {
            for line in message.text.lines() {
                lines.push(Line::from(vec![Span::styled(
                    format!("  {line}"),
                    theme::dim_style(),
                )]));
            }
        }
        Role::Tool => {
            if let Some(tool) = &message.tool {
                draw_tool_lines(lines, tool, width, spinner_frame);
            }
        }
    }
}

fn draw_tool_lines(
    lines: &mut Vec<Line<'static>>,
    tool: &ToolInfo,
    width: usize,
    spinner_frame: usize,
) {
    let border_color = if tool.is_error {
        theme::RED
    } else {
        theme::GREEN
    };
    let icon = if tool.result.is_empty() {
        "…"
    } else if tool.is_error {
        "✘"
    } else {
        "✔"
    };
    let icon_color = if tool.is_error {
        theme::RED
    } else {
        theme::GREEN
    };
    let header_rest = width.saturating_sub(6 + tool.name.len());
    let fill = "─".repeat(header_rest);

    lines.push(Line::from(vec![
        Span::styled("  ┌─ ", Style::default().fg(border_color)),
        Span::styled(format!("{icon} "), Style::default().fg(icon_color)),
        Span::styled(
            tool.name.clone(),
            Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {fill}"), Style::default().fg(border_color)),
    ]));

    if !tool.input.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("  │ ", Style::default().fg(border_color)),
            Span::styled("input: ", theme::dim_style()),
            Span::raw(truncate_line(&tool.input, width.saturating_sub(14))),
        ]));
    }

    if tool.result.is_empty() {
        let spinner = theme::SPINNER_FRAMES[spinner_frame % theme::SPINNER_FRAMES.len()];
        lines.push(Line::from(vec![
            Span::styled("  │ ", Style::default().fg(border_color)),
            Span::styled(
                format!("{spinner} running…"),
                Style::default().fg(theme::CYAN),
            ),
        ]));
    } else if tool.is_error {
        lines.push(Line::from(vec![
            Span::styled("  │ ", Style::default().fg(border_color)),
            Span::styled("error: ", Style::default().fg(theme::RED)),
            Span::raw(truncate_line(&tool.result, width.saturating_sub(14))),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("  │ ", Style::default().fg(border_color)),
            Span::styled("result: ", theme::dim_style()),
            Span::raw(truncate_line(&tool.result, width.saturating_sub(14))),
        ]));
    }

    let footer = "─".repeat(width.saturating_sub(4));
    lines.push(Line::from(vec![Span::styled(
        format!("  └{footer}"),
        Style::default().fg(border_color),
    )]));
}

fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_owned()];
    }

    let mut result = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            result.push(String::new());
            continue;
        }

        let mut current = String::new();
        for word in line.split_whitespace() {
            if current.is_empty() {
                current = word.to_owned();
            } else if current.len() + 1 + word.len() <= max_width {
                current.push(' ');
                current.push_str(word);
            } else {
                result.push(current);
                current = word.to_owned();
            }
        }
        if !current.is_empty() {
            result.push(current);
        }
    }

    if result.is_empty() {
        vec![String::new()]
    } else {
        result
    }
}

fn truncate_line(text: &str, max_width: usize) -> String {
    let line = text.lines().next().unwrap_or(text);
    let mut truncated = line.chars().take(max_width).collect::<String>();
    if line.chars().count() > max_width {
        truncated.push('…');
    }
    truncated
}
