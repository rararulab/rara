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

//! LaTeX math to Unicode text converter for Telegram.
//!
//! Telegram does not support LaTeX rendering. This module converts `$...$`
//! (inline math) and `$$...$$` (display math) blocks into readable Unicode
//! text by replacing LaTeX commands with their Unicode equivalents.
//!
//! # Supported Conversions
//!
//! - Greek letters: `\alpha` → `α`, `\beta` → `β`, etc.
//! - Operators: `\times` → `×`, `\div` → `÷`, `\cdot` → `·`
//! - Relations: `\leq` → `≤`, `\geq` → `≥`, `\neq` → `≠`
//! - Arrows: `\to` → `→`, `\leftarrow` → `←`
//! - Big operators: `\sum` → `∑`, `\prod` → `∏`, `\int` → `∫`
//! - Misc: `\infty` → `∞`, `\sqrt` → `√`, `\partial` → `∂`
//! - Subscripts/superscripts via Unicode characters (limited charset)
//! - `\frac{a}{b}` → `a/b`, `\text{...}` → content as-is

/// Maximum recursion depth for nested LaTeX constructs (e.g. subscripts inside
/// subscripts). Prevents stack overflow from adversarial input.
const MAX_DEPTH: usize = 8;

/// Convert LaTeX math delimiters and their contents to Unicode text.
///
/// Processes `$$...$$` (display math, rendered on its own line) and `$...$`
/// (inline math) blocks. Text outside math delimiters is returned unchanged.
///
/// **Important:** This function must only be called on plain-text segments. It
/// does not understand Markdown code spans or fenced code blocks — the caller
/// is responsible for protecting those regions (see [`super::markdown`]).
pub fn latex_to_unicode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Display math: $$...$$
        // Only convert when a valid closing $$ exists; otherwise preserve literal.
        if i + 1 < len && chars[i] == '$' && chars[i + 1] == '$' {
            if let Some((content, end)) = try_parse_display_math(&chars, i) {
                let converted = convert_latex_content_depth(content.trim(), 0);
                result.push('\n');
                result.push_str(&converted);
                result.push('\n');
                i = end;
                continue;
            }
            // No closing $$ found — emit literal $$
            result.push('$');
            result.push('$');
            i += 2;
            continue;
        }

        // Inline math: $...$
        // Guard against false positives: require non-space after opening $
        // and non-space before closing $.
        if chars[i] == '$' {
            if let Some((content, end)) = try_parse_inline_math(&chars, i) {
                let converted = convert_latex_content_depth(content.trim(), 0);
                result.push_str(&converted);
                i = end;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Try to parse a display math block `$$...$$` starting at `pos`.
///
/// Returns `(content, end_position_after_closing_$$)` only if a valid closing
/// `$$` is found. Returns `None` for unclosed delimiters to avoid corrupting
/// trailing text.
fn try_parse_display_math(chars: &[char], pos: usize) -> Option<(String, usize)> {
    let len = chars.len();
    if pos + 1 >= len || chars[pos] != '$' || chars[pos + 1] != '$' {
        return None;
    }

    let content_start = pos + 2;
    let mut i = content_start;
    while i + 1 < len {
        if chars[i] == '$' && chars[i + 1] == '$' {
            let content: String = chars[content_start..i].iter().collect();
            return Some((content, i + 2));
        }
        i += 1;
    }

    // No closing $$ found
    None
}

/// Try to parse an inline math block `$...$` starting at `pos`.
///
/// Returns `(content, end_position_after_closing_dollar)` if successful.
/// Rejects empty content and requires non-space chars adjacent to delimiters.
fn try_parse_inline_math(chars: &[char], pos: usize) -> Option<(String, usize)> {
    let len = chars.len();
    if pos >= len || chars[pos] != '$' {
        return None;
    }

    let content_start = pos + 1;
    if content_start >= len || chars[content_start].is_whitespace() {
        return None;
    }

    let mut i = content_start;
    while i < len {
        if chars[i] == '$' {
            if i > content_start && !chars[i - 1].is_whitespace() {
                let content: String = chars[content_start..i].iter().collect();
                return Some((content, i + 1));
            }
            // Space before closing $ — not valid inline math
            return None;
        }
        // Don't span across newlines for inline math
        if chars[i] == '\n' {
            return None;
        }
        i += 1;
    }

    None
}

/// Convert the interior of a LaTeX math block to Unicode with depth tracking.
///
/// When `depth` reaches [`MAX_DEPTH`], returns the raw text without further
/// conversion to prevent stack overflow from adversarial input.
fn convert_latex_content_depth(latex: &str, depth: usize) -> String {
    if depth >= MAX_DEPTH {
        return latex.to_string();
    }

    let mut result = String::with_capacity(latex.len());
    let chars: Vec<char> = latex.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // LaTeX commands starting with backslash
        if chars[i] == '\\' {
            if let Some(cmd) = try_latex_command_full(&chars, i, depth) {
                match cmd {
                    CommandResult::Static(s, end) => {
                        result.push_str(s);
                        i = end;
                    }
                    CommandResult::Dynamic(s, end) => {
                        result.push_str(&s);
                        i = end;
                    }
                }
                continue;
            }
            // Escaped braces or other chars
            if i + 1 < len && matches!(chars[i + 1], '{' | '}' | '\\' | '$' | '%' | '&') {
                result.push(chars[i + 1]);
                i += 2;
                continue;
            }
            // Unknown command — skip the backslash, keep the rest
            i += 1;
            continue;
        }

        // Subscript: _{...} or _x
        if chars[i] == '_' {
            let (sub_content, end) = parse_sub_super_arg(&chars, i + 1, depth);
            for ch in sub_content.chars() {
                result.push(to_subscript(ch));
            }
            i = end;
            continue;
        }

        // Superscript: ^{...} or ^x
        if chars[i] == '^' {
            let (sup_content, end) = parse_sub_super_arg(&chars, i + 1, depth);
            for ch in sup_content.chars() {
                result.push(to_superscript(ch));
            }
            i = end;
            continue;
        }

        // Strip curly braces (grouping only, content preserved)
        if chars[i] == '{' || chars[i] == '}' {
            i += 1;
            continue;
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Parse argument for subscript/superscript: either `{content}` or a single
/// char.
///
/// Returns `(content, end_position)`. The `depth` parameter is forwarded to
/// recursive calls to enforce [`MAX_DEPTH`].
fn parse_sub_super_arg(chars: &[char], pos: usize, depth: usize) -> (String, usize) {
    let len = chars.len();
    if pos >= len {
        return (String::new(), pos);
    }

    if chars[pos] == '{' {
        // Braced group
        let start = pos + 1;
        let mut brace_depth = 1;
        let mut i = start;
        while i < len && brace_depth > 0 {
            match chars[i] {
                '{' => brace_depth += 1,
                '}' => brace_depth -= 1,
                _ => {}
            }
            if brace_depth > 0 {
                i += 1;
            }
        }
        let content: String = chars[start..i].iter().collect();
        let converted = convert_latex_content_depth(&content, depth + 1);
        (converted, if brace_depth == 0 { i + 1 } else { i })
    } else if chars[pos] == '\\' {
        // A LaTeX command as subscript/superscript argument
        if let Some((replacement, end)) = try_latex_command(chars, pos) {
            (replacement.to_string(), end)
        } else {
            (String::new(), pos + 1)
        }
    } else {
        // Single character
        (chars[pos].to_string(), pos + 1)
    }
}

/// Result of matching a LaTeX command — either a static Unicode string or
/// dynamically constructed text.
enum CommandResult {
    Static(&'static str, usize),
    Dynamic(String, usize),
}

/// Try to match a LaTeX command at `pos` (which must point to `\`).
///
/// Returns the replacement text and end position on success. The `depth`
/// parameter is forwarded to recursive conversion calls.
fn try_latex_command_full(chars: &[char], pos: usize, depth: usize) -> Option<CommandResult> {
    let len = chars.len();
    if pos >= len || chars[pos] != '\\' {
        return None;
    }

    // Extract the command name (alphabetic chars after \)
    let name_start = pos + 1;
    let mut name_end = name_start;
    while name_end < len && chars[name_end].is_ascii_alphabetic() {
        name_end += 1;
    }

    if name_end == name_start {
        return None;
    }

    let name: String = chars[name_start..name_end].iter().collect();

    // Handle \frac{a}{b} → a/b
    if name == "frac" {
        let (num, after_num) = parse_braced_group(chars, name_end)?;
        let (den, after_den) = parse_braced_group(chars, after_num)?;
        let num_conv = convert_latex_content_depth(&num, depth + 1);
        let den_conv = convert_latex_content_depth(&den, depth + 1);
        return Some(CommandResult::Dynamic(
            format!("{num_conv}/{den_conv}"),
            after_den,
        ));
    }

    // Handle \text{...}, \mathrm{...}, etc. — passthrough content
    if matches!(name.as_str(), "text" | "mathrm" | "textrm" | "textbf") {
        let (content, after) = parse_braced_group(chars, name_end)?;
        return Some(CommandResult::Dynamic(content, after));
    }

    // Handle \sqrt{x} → √x
    if name == "sqrt" {
        let (content, after) = parse_braced_group(chars, name_end)?;
        let conv = convert_latex_content_depth(&content, depth + 1);
        return Some(CommandResult::Dynamic(format!("√{conv}"), after));
    }

    // Simple command → unicode lookup
    let replacement = latex_command_to_unicode(&name)?;
    Some(CommandResult::Static(replacement, name_end))
}

/// Convenience wrapper that returns a static str reference when possible.
/// Used by `parse_sub_super_arg` which only needs the replacement string.
fn try_latex_command(chars: &[char], pos: usize) -> Option<(&'static str, usize)> {
    match try_latex_command_full(chars, pos, 0)? {
        CommandResult::Static(s, end) => Some((s, end)),
        CommandResult::Dynamic(..) => None,
    }
}

/// Parse a `{content}` group starting at `pos`. Returns `(content, end_pos)`.
fn parse_braced_group(chars: &[char], pos: usize) -> Option<(String, usize)> {
    let len = chars.len();
    if pos >= len || chars[pos] != '{' {
        return None;
    }
    let start = pos + 1;
    let mut depth = 1;
    let mut i = start;
    while i < len && depth > 0 {
        match chars[i] {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        if depth > 0 {
            i += 1;
        }
    }
    if depth != 0 {
        return None;
    }
    let content: String = chars[start..i].iter().collect();
    Some((content, i + 1))
}

/// Map a Unicode subscript character. Falls back to the original char if no
/// Unicode subscript exists.
fn to_subscript(ch: char) -> char {
    match ch {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        _ => ch,
    }
}

/// Map a Unicode superscript character. Falls back to the original char if no
/// Unicode superscript exists.
fn to_superscript(ch: char) -> char {
    match ch {
        '0' => '⁰',
        '1' => '¹',
        '2' => '²',
        '3' => '³',
        '4' => '⁴',
        '5' => '⁵',
        '6' => '⁶',
        '7' => '⁷',
        '8' => '⁸',
        '9' => '⁹',
        '+' => '⁺',
        '-' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'n' => 'ⁿ',
        'i' => 'ⁱ',
        _ => ch,
    }
}

/// Lookup table: LaTeX command name → Unicode replacement.
fn latex_command_to_unicode(name: &str) -> Option<&'static str> {
    Some(match name {
        // Greek lowercase
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" | "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" | "vartheta" => "θ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" | "varpi" => "π",
        "rho" | "varrho" => "ρ",
        "sigma" | "varsigma" => "σ",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" | "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",

        // Greek uppercase
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",

        // Binary operators
        "times" => "×",
        "div" => "÷",
        "cdot" => "·",
        "pm" => "±",
        "mp" => "∓",
        "ast" => "∗",
        "star" => "⋆",
        "circ" => "∘",
        "bullet" => "•",

        // Relations
        "leq" | "le" => "≤",
        "geq" | "ge" => "≥",
        "neq" | "ne" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "sim" => "∼",
        "simeq" => "≃",
        "cong" => "≅",
        "propto" => "∝",
        "ll" => "≪",
        "gg" => "≫",
        "subset" => "⊂",
        "supset" => "⊃",
        "subseteq" => "⊆",
        "supseteq" => "⊇",
        "in" => "∈",
        "notin" => "∉",
        "ni" => "∋",

        // Arrows
        "to" | "rightarrow" => "→",
        "leftarrow" => "←",
        "leftrightarrow" => "↔",
        "Rightarrow" => "⇒",
        "Leftarrow" => "⇐",
        "Leftrightarrow" => "⇔",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "mapsto" => "↦",

        // Big operators
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "iint" => "∬",
        "iiint" => "∭",
        "oint" => "∮",
        "bigcup" => "⋃",
        "bigcap" => "⋂",

        // Misc symbols
        "infty" => "∞",
        "partial" => "∂",
        "nabla" => "∇",
        "forall" => "∀",
        "exists" => "∃",
        "nexists" => "∄",
        "emptyset" | "varnothing" => "∅",
        "neg" | "lnot" => "¬",
        "land" | "wedge" => "∧",
        "lor" | "vee" => "∨",
        "cap" => "∩",
        "cup" => "∪",
        "therefore" => "∴",
        "because" => "∵",
        "angle" => "∠",
        "triangle" => "△",
        "perp" => "⊥",
        "parallel" => "∥",
        "prime" => "′",
        "hbar" => "ℏ",
        "ell" => "ℓ",
        "Re" => "ℜ",
        "Im" => "ℑ",
        "aleph" => "ℵ",

        // Dots
        "ldots" | "dots" => "…",
        "cdots" => "⋯",
        "vdots" => "⋮",
        "ddots" => "⋱",

        // Spacing and formatting — just produce a space or nothing
        "quad" => "  ",
        "qquad" => "    ",
        "," => " ",
        ";" => " ",
        "!" => "",
        "left" | "right" | "big" | "Big" | "bigg" | "Bigg" => "",
        "displaystyle" | "textstyle" | "scriptstyle" => "",
        "mathrm" | "mathbf" | "mathit" | "mathcal" | "mathbb" => "",

        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_math_greek() {
        assert_eq!(latex_to_unicode("price is $\\alpha$"), "price is α");
    }

    #[test]
    fn display_math_ema_formula() {
        let input =
            "The formula:\n$$EMA_t = \\alpha \\times P_t + (1 - \\alpha) \\times EMA_{t-1}$$";
        let output = latex_to_unicode(input);
        assert!(output.contains("EMAₜ = α × Pₜ + (1 - α) × EMAₜ₋₁"));
    }

    #[test]
    fn subscript_and_superscript() {
        assert_eq!(latex_to_unicode("$x_1^2$"), "x₁²");
        assert_eq!(latex_to_unicode("$a_{n+1}$"), "aₙ₊₁");
    }

    #[test]
    fn no_false_positive_on_dollar_amounts() {
        // $5 should not be treated as math (space after $)
        assert_eq!(latex_to_unicode("costs $5 or $ 10"), "costs $5 or $ 10");
    }

    #[test]
    fn operators_and_relations() {
        assert_eq!(latex_to_unicode("$a \\leq b \\neq c$"), "a ≤ b ≠ c");
    }

    #[test]
    fn escaped_braces() {
        assert_eq!(latex_to_unicode("$\\{a, b\\}$"), "{a, b}");
    }

    #[test]
    fn preserves_non_math_text() {
        let input = "Hello world, no math here.";
        assert_eq!(latex_to_unicode(input), input);
    }

    #[test]
    fn big_operators() {
        assert_eq!(latex_to_unicode("$\\sum_{i=0}^{n}$"), "∑ᵢ₌₀ⁿ");
    }

    #[test]
    fn multiple_inline_math() {
        assert_eq!(
            latex_to_unicode("where $P_t$ is the price and $\\alpha$ is the weight"),
            "where Pₜ is the price and α is the weight"
        );
    }

    #[test]
    fn nested_subscript_with_latex_command() {
        assert_eq!(latex_to_unicode("$EMA_{t-1}$"), "EMAₜ₋₁");
    }

    #[test]
    fn unclosed_display_math_preserved() {
        // Unclosed $$ must not corrupt trailing text
        let input = "price is $$100 and something";
        assert_eq!(latex_to_unicode(input), "price is $$100 and something");
    }

    #[test]
    fn unclosed_inline_math_preserved() {
        let input = "cost is $100 total";
        assert_eq!(latex_to_unicode(input), input);
    }

    #[test]
    fn deeply_nested_subscripts_cap() {
        // Build deeply nested subscripts: x_{_{_{_{_{_{_{_{_x}}}}}}}}
        let mut input = "x".to_string();
        for _ in 0..20 {
            input = format!("{input}_{{x}}");
        }
        let input = format!("${input}$");
        // Should not panic/stack overflow — just returns something
        let result = latex_to_unicode(&input);
        assert!(!result.is_empty());
    }

    #[test]
    fn frac_conversion() {
        assert_eq!(latex_to_unicode("$\\frac{a}{b}$"), "a/b");
    }

    #[test]
    fn sqrt_conversion() {
        assert_eq!(latex_to_unicode("$\\sqrt{x}$"), "√x");
    }
}
