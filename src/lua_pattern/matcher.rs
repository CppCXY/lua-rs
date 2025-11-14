// Lua pattern matcher
// Implements actual pattern matching logic

use super::parser::{AnchorType, Pattern, RepeatMode};

/// Find pattern in string, returns (start, end, captures)
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

/// Global substitution
pub fn gsub(
    text: &str,
    pattern: &Pattern,
    replacement: &str,
    max: Option<usize>,
) -> (String, usize) {
    let mut result = String::new();
    let mut count = 0;
    let mut pos = 0;
    let text_chars: Vec<char> = text.chars().collect();

    while pos < text_chars.len() {
        if let Some(max_count) = max {
            if count >= max_count {
                // Reached max replacements, copy rest
                result.extend(&text_chars[pos..]);
                break;
            }
        }

        if let Some((end_pos, _)) = try_match(pattern, &text_chars, pos) {
            // Found match
            count += 1;

            // Do replacement (simplified - doesn't handle %1, %2 yet)
            result.push_str(replacement);

            pos = end_pos.max(pos + 1); // Move past match
        } else {
            // No match, copy character
            result.push(text_chars[pos]);
            pos += 1;
        }
    }

    (result, count)
}

/// Try to match pattern at specific position
fn try_match(pattern: &Pattern, text: &[char], pos: usize) -> Option<(usize, Vec<String>)> {
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
