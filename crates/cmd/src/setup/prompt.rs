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
/// fails.
pub fn ask_password(prompt: &str) -> String {
    use crossterm::terminal;

    print!("{prompt}: ");
    io::stdout().flush().expect("flush stdout");

    let was_raw = terminal::enable_raw_mode().is_ok();
    let mut input = String::new();
    // In raw mode, read_line still works but won't echo.
    io::stdin().read_line(&mut input).expect("read stdin");
    if was_raw {
        let _ = terminal::disable_raw_mode();
    }
    println!();
    input.trim().to_owned()
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
