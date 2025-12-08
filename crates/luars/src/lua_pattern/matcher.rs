// Lua pattern matcher
// Implements actual pattern matching logic

use super::parser::{AnchorType, Pattern, RepeatMode};

/// Find pattern in string, returns (start, end, captures)
/// Positions are char indices (not byte indices)
pub fn find(text: &str, pattern: &Pattern, init: usize) -> Option<(usize, usize, Vec<String>)> {
    let text_chars: Vec<char> = text.chars().collect();

    // Try matching from each position
    for start_pos in init..=text_chars.len() {
        if let Some((end_pos, captures)) = try_match(pattern, &text_chars, start_pos) {
            return Some((start_pos, end_pos, captures));
        }
    }

    None
}

/// Match pattern against string, returns (end_pos, captures) on success
pub fn match_pattern(text: &str, pattern: &Pattern) -> Option<(usize, Vec<String>)> {
    let text_chars: Vec<char> = text.chars().collect();
    try_match(pattern, &text_chars, 0)
}

/// Global substitution with capture support
/// Supports:
/// - %0: entire match
/// - %1-%9: capture groups
/// - %%: literal %
pub fn gsub(
    text: &str,
    pattern: &Pattern,
    replacement: &str,
    max: Option<usize>,
) -> Result<(String, usize), String> {
    let mut result = String::new();
    let mut count = 0;
    let mut pos = 0;
    let text_chars: Vec<char> = text.chars().collect();
    let mut last_was_nonempty = false; // Track if last match was non-empty

    // Fast path: if replacement doesn't contain %, no substitution needed
    let needs_substitution = replacement.contains('%');

    // We need to try matching one position past the end to handle empty matches at the end
    while pos <= text_chars.len() {
        if let Some(max_count) = max {
            if count >= max_count {
                // Reached max replacements, copy rest
                result.extend(&text_chars[pos..]);
                break;
            }
        }

        if let Some((end_pos, captures)) = try_match(pattern, &text_chars, pos) {
            // Skip empty match right after non-empty match
            if end_pos == pos && last_was_nonempty {
                // Copy character and continue
                if pos < text_chars.len() {
                    result.push(text_chars[pos]);
                }
                pos += 1;
                last_was_nonempty = false;
                continue;
            }

            // Found match (either non-empty, or empty not after non-empty)
            count += 1;

            if needs_substitution {
                // Build replacement with capture substitution
                let matched_text: String = text_chars[pos..end_pos].iter().collect();
                let replaced = substitute_captures(replacement, &matched_text, &captures)?;
                result.push_str(&replaced);
            } else {
                // Fast path: no % in replacement, just copy it
                result.push_str(replacement);
            }

            // Handle empty vs non-empty match
            if end_pos == pos {
                // Empty match: copy character and advance
                if pos < text_chars.len() {
                    result.push(text_chars[pos]);
                }
                pos += 1;
                last_was_nonempty = false;
            } else {
                // Non-empty match: just advance position
                pos = end_pos;
                last_was_nonempty = true;
            }
        } else {
            // No match, copy character if within bounds
            if pos < text_chars.len() {
                result.push(text_chars[pos]);
            }
            pos += 1;
            last_was_nonempty = false;
        }
    }

    Ok((result, count))
}

/// Substitute %0-%9 and %% in replacement string
fn substitute_captures(
    replacement: &str,
    full_match: &str,
    captures: &[String],
) -> Result<String, String> {
    let mut result = String::new();
    let chars: Vec<char> = replacement.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '%' {
            if i + 1 < chars.len() {
                let next = chars[i + 1];
                if next == '%' {
                    // %% -> literal %
                    result.push('%');
                    i += 2;
                } else if next >= '0' && next <= '9' {
                    // %0-%9 -> capture
                    let capture_idx = (next as u8 - b'0') as usize;
                    if capture_idx == 0 {
                        // %0 is full match
                        result.push_str(full_match);
                    } else if capture_idx <= captures.len() {
                        // %1-%9 are captures
                        result.push_str(&captures[capture_idx - 1]);
                    } else {
                        // Invalid capture index
                        return Err(format!("invalid capture index %{}", capture_idx));
                    }
                    i += 2;
                } else {
                    // Invalid escape sequence, just copy
                    result.push('%');
                    i += 1;
                }
            } else {
                // Trailing %, just copy
                result.push('%');
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    Ok(result)
}

/// Try to match pattern at specific position
pub fn try_match(pattern: &Pattern, text: &[char], pos: usize) -> Option<(usize, Vec<String>)> {
    let mut captures = Vec::new();
    match match_impl(pattern, text, pos, &mut captures) {
        Some(end_pos) => Some((end_pos, captures)),
        None => None,
    }
}

fn match_impl(
    pattern: &Pattern,
    text: &[char],
    mut pos: usize,
    captures: &mut Vec<String>,
) -> Option<usize> {
    match pattern {
        Pattern::Char(c) => {
            if pos < text.len() && text[pos] == *c {
                Some(pos + 1)
            } else {
                None
            }
        }
        Pattern::Dot => {
            if pos < text.len() {
                Some(pos + 1)
            } else {
                None
            }
        }
        Pattern::Class(class) => {
            if pos < text.len() && class.matches(text[pos]) {
                Some(pos + 1)
            } else {
                None
            }
        }
        Pattern::Set { chars, negated } => {
            if pos >= text.len() {
                return None;
            }
            let ch = text[pos];
            let found = chars.contains(&ch);
            if *negated != found {
                // XOR: match if (negated and not found) or (not negated and found)
                Some(pos + 1)
            } else {
                None
            }
        }
        Pattern::Seq(patterns) => {
            for pat in patterns {
                match match_impl(pat, text, pos, captures) {
                    Some(new_pos) => pos = new_pos,
                    None => return None,
                }
            }
            Some(pos)
        }
        Pattern::Repeat {
            pattern: inner,
            mode,
        } => {
            match mode {
                RepeatMode::ZeroOrMore => {
                    // Greedy: match as many as possible
                    let mut last_pos = pos;
                    while let Some(new_pos) = match_impl(inner, text, last_pos, captures) {
                        if new_pos == last_pos {
                            break; // Avoid infinite loop
                        }
                        last_pos = new_pos;
                    }
                    Some(last_pos)
                }
                RepeatMode::OneOrMore => {
                    // Must match at least once
                    let first_pos = match_impl(inner, text, pos, captures)?;
                    let mut last_pos = first_pos;
                    while let Some(new_pos) = match_impl(inner, text, last_pos, captures) {
                        if new_pos == last_pos {
                            break;
                        }
                        last_pos = new_pos;
                    }
                    Some(last_pos)
                }
                RepeatMode::ZeroOrOne => {
                    // Optional: try to match once
                    if let Some(new_pos) = match_impl(inner, text, pos, captures) {
                        Some(new_pos)
                    } else {
                        Some(pos) // Match zero times
                    }
                }
                RepeatMode::Lazy => {
                    // Non-greedy: match as few as possible
                    Some(pos) // For now, match zero times
                }
            }
        }
        Pattern::Capture(inner) => {
            let start = pos;
            let end = match_impl(inner, text, pos, captures)?;
            let captured: String = text[start..end].iter().collect();
            captures.push(captured);
            Some(end)
        }
        Pattern::Anchor(anchor_type) => match anchor_type {
            AnchorType::Start => {
                if pos == 0 {
                    Some(pos)
                } else {
                    None
                }
            }
            AnchorType::End => {
                if pos == text.len() {
                    Some(pos)
                } else {
                    None
                }
            }
        },
        Pattern::Balanced { open, close } => {
            if pos >= text.len() || text[pos] != *open {
                return None;
            }

            let mut depth = 1;
            let mut curr = pos + 1;

            while curr < text.len() && depth > 0 {
                if text[curr] == *open {
                    depth += 1;
                } else if text[curr] == *close {
                    depth -= 1;
                }
                curr += 1;
            }

            if depth == 0 { Some(curr) } else { None }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua_pattern::parser::parse_pattern;

    #[test]
    fn test_simple_match() {
        let pattern = parse_pattern("hello").unwrap();
        let text = "hello world";
        assert!(match_pattern(text, &pattern).is_some());
    }

    #[test]
    fn test_digit_class() {
        let pattern = parse_pattern("%d+").unwrap();
        let text = "abc123def";
        if let Some((start, end, _)) = find(text, &pattern, 0) {
            assert_eq!(&text[start..end], "123");
        } else {
            panic!("Should find match");
        }
    }
}
