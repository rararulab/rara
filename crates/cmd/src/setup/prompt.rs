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

use std::io::{self, Write};

/// Setup mode -- how to handle existing configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupMode {
    /// Ignore existing config, configure from scratch.
    Fresh,
    /// Show existing values as defaults, allow edits.
    Modify,
    /// Skip already-configured sections.
    FillMissing,
}

/// Print a section header.
pub fn print_step(title: &str) {
    println!("\n--- {title} ---");
}

/// Print success message.
pub fn print_ok(msg: &str) {
    println!("  ok: {msg}");
}

/// Print error message.
pub fn print_err(msg: &str) {
    eprintln!("  err: {msg}");
}

/// Read a line from stdin with an optional default value.
pub fn ask(prompt: &str, default: Option<&str>) -> String {
    match default {
        Some(d) => print!("{prompt} [{d}]: "),
        None => print!("{prompt}: "),
    }
    io::stdout().flush().expect("flush stdout");

    let mut input = String::new();
    io::stdin().read_line(&mut input).expect("read stdin");
    let trimmed = input.trim();

    if trimmed.is_empty() {
        default.unwrap_or("").to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Read a password from stdin (no echo). Falls back to plain `ask` if raw mode
/// is unavailable.
pub fn ask_password(prompt: &str) -> String {
    use crossterm::{
        event::{self, Event, KeyCode, KeyModifiers},
        terminal,
    };

    print!("{prompt}: ");
    io::stdout().flush().expect("flush stdout");

    if terminal::enable_raw_mode().is_err() {
        // Fallback: plain read (will echo).
        let mut input = String::new();
        io::stdin().read_line(&mut input).expect("read stdin");
        println!();
        return input.trim().to_owned();
    }

    let mut buf = String::new();
    loop {
        if let Ok(Event::Key(key)) = event::read() {
            match key.code {
                KeyCode::Enter => break,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = terminal::disable_raw_mode();
                    std::process::exit(130);
                }
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c) => buf.push(c),
                _ => {}
            }
        }
    }

    let _ = terminal::disable_raw_mode();
    println!();
    buf
}

/// Present numbered choices and return selected index (0-based).
pub fn ask_choice(prompt: &str, options: &[&str]) -> usize {
    println!("{prompt}");
    for (i, opt) in options.iter().enumerate() {
        println!("  {}. {opt}", i + 1);
    }
    loop {
        print!("> ");
        io::stdout().flush().expect("flush stdout");
        let mut input = String::new();
        io::stdin().read_line(&mut input).expect("read stdin");
        if let Ok(n) = input.trim().parse::<usize>() {
            if n >= 1 && n <= options.len() {
                return n - 1;
            }
        }
        print_err(&format!("enter a number between 1 and {}", options.len()));
    }
}

/// Ask a yes/no question with a default.
pub fn confirm(prompt: &str, default: bool) -> bool {
    let hint = if default { "Y/n" } else { "y/N" };
    print!("{prompt} [{hint}]: ");
    io::stdout().flush().expect("flush stdout");
    let mut input = String::new();
    io::stdin().read_line(&mut input).expect("read stdin");
    let trimmed = input.trim().to_lowercase();
    if trimmed.is_empty() {
        default
    } else {
        matches!(trimmed.as_str(), "y" | "yes")
    }
}

/// Mask a secret string for display, showing only the first 8 characters.
pub fn mask_secret(s: &str) -> String {
    if s.len() <= 8 {
        "****".to_owned()
    } else {
        format!("{}****", &s[..8])
    }
}
