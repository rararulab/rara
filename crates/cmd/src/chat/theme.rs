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

use ratatui::style::{Color, Modifier, Style};

pub const ACCENT: Color = Color::Rgb(255, 92, 0);
pub const BORDER: Color = Color::Rgb(63, 59, 56);
pub const TEXT_PRIMARY: Color = Color::Rgb(240, 239, 238);
pub const TEXT_SECONDARY: Color = Color::Rgb(168, 162, 158);
pub const TEXT_TERTIARY: Color = Color::Rgb(120, 113, 108);
pub const GREEN: Color = Color::Rgb(34, 197, 94);
pub const BLUE: Color = Color::Rgb(59, 130, 246);
pub const YELLOW: Color = Color::Rgb(234, 179, 8);
pub const RED: Color = Color::Rgb(239, 68, 68);
pub const PURPLE: Color = Color::Rgb(168, 85, 247);
pub const CYAN: Color = BLUE;
pub const DIM: Color = TEXT_SECONDARY;

pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn title_style() -> Style { Style::default().fg(ACCENT).add_modifier(Modifier::BOLD) }

pub fn dim_style() -> Style { Style::default().fg(TEXT_SECONDARY) }

pub fn input_style() -> Style { Style::default().fg(ACCENT).add_modifier(Modifier::BOLD) }

pub fn hint_style() -> Style { Style::default().fg(TEXT_TERTIARY) }

/// Style for the thinking section header ("Thinking…" / "Thought for Xs").
pub fn thinking_header_style() -> Style { Style::default().fg(CYAN).add_modifier(Modifier::BOLD) }

/// Style for thinking content text (dimmed italic).
pub fn thinking_text_style() -> Style {
    Style::default()
        .fg(TEXT_TERTIARY)
        .add_modifier(Modifier::ITALIC)
}
