// Lua pattern matcher
// Implements actual pattern matching logic

use super::parser::{AnchorType, Pattern, RepeatMode};

/// A capture value: either a string or a position (for `()` position captures)
#[derive(Debug, Clone)]
pub enum CaptureValue {
    String(String),
    Position(usize), // 1-based byte position
}

/// Information about a pattern match
#[derive(Debug, Clone)]
pub struct MatchInfo {
    pub start: usize,                // Start byte offset
    pub end: usize,                  // End byte offset
    pub captures: Vec<CaptureValue>, // Captured values
}

/// Build a mapping from char index to byte offset. The mapping has len+1 entries
/// (the last entry is text.len(), representing the end position).
fn char_to_byte_map(text: &str) -> Vec<usize> {
    text.char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect()
}

/// Convert a byte offset to a char index (for the `init` parameter).
fn byte_to_char_index(text: &str, byte_pos: usize) -> usize {
    let clamped = byte_pos.min(text.len());
    text[..clamped].chars().count()
}

/// Find all matches of pattern in text
pub fn find_all_matches(text: &str, pattern: &Pattern, max: Option<usize>) -> Vec<MatchInfo> {
    let mut matches = Vec::new();
    let mut pos = 0;
    let text_chars: Vec<char> = text.chars().collect();
    let c2b = char_to_byte_map(text);
    let mut last_was_nonempty = false;

    while pos <= text_chars.len() {
        if let Some(max_count) = max {
            if matches.len() >= max_count {
                break;
            }
        }

        if let Some((end_pos, captures)) = try_match(pattern, &text_chars, &c2b, pos) {
            // Skip empty match right after non-empty match
            if end_pos == pos && last_was_nonempty {
                if pos < text_chars.len() {
                    pos += 1;
                }
                last_was_nonempty = false;
                continue;
            }

            // Convert char positions to byte positions
            matches.push(MatchInfo {
                start: c2b[pos],
                end: c2b[end_pos],
                captures,
            });

            // Handle empty vs non-empty match
            if end_pos == pos {
                if pos < text_chars.len() {
                    pos += 1;
                } else {
                    break; // empty match at end of string, we're done
                }
                last_was_nonempty = false;
            } else {
                pos = end_pos;
                last_was_nonempty = true;
            }
        } else {
            if pos < text_chars.len() {
                pos += 1;
            } else {
                break;
            }
            last_was_nonempty = false;
        }
    }

    matches
}

/// Find pattern in string, returns (byte_start, byte_end, captures)
/// `init` is a 0-based byte offset. Return positions are byte offsets.
pub fn find(
    text: &str,
    pattern: &Pattern,
    init: usize,
) -> Option<(usize, usize, Vec<CaptureValue>)> {
    if init > text.len() {
        return None;
    }
    let text_chars: Vec<char> = text.chars().collect();
    let c2b = char_to_byte_map(text);

    // Convert byte-based init to char index
    let init_char = byte_to_char_index(text, init);

    // Try matching from each position
    for start_pos in init_char..=text_chars.len() {
        if let Some((end_pos, captures)) = try_match(pattern, &text_chars, &c2b, start_pos) {
            return Some((c2b[start_pos], c2b[end_pos], captures));
        }
    }

    None
}

/// Match pattern against string, returns (byte_end_pos, captures) on success
pub fn match_pattern(text: &str, pattern: &Pattern) -> Option<(usize, Vec<CaptureValue>)> {
    let text_chars: Vec<char> = text.chars().collect();
    let c2b = char_to_byte_map(text);
    match try_match(pattern, &text_chars, &c2b, 0) {
        Some((end_pos, captures)) => Some((c2b[end_pos], captures)),
        None => None,
    }
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
    let mut pos = 0; // char position
    let text_chars: Vec<char> = text.chars().collect();
    let c2b = char_to_byte_map(text);
    let mut last_was_nonempty = false;

    // Fast path: if replacement doesn't contain %, no substitution needed
    let needs_substitution = replacement.contains('%');

    // We need to try matching one position past the end to handle empty matches at the end
    while pos <= text_chars.len() {
        if let Some(max_count) = max {
            if count >= max_count {
                // Reached max replacements, copy rest via byte slice
                result.push_str(&text[c2b[pos]..]);
                break;
            }
        }

        if let Some((end_pos, captures)) = try_match(pattern, &text_chars, &c2b, pos) {
            // Skip empty match right after non-empty match
            if end_pos == pos && last_was_nonempty {
                // Copy char and continue
                if pos < text_chars.len() {
                    result.push(text_chars[pos]);
                }
                pos += 1;
                last_was_nonempty = false;
                continue;
            }

            // Found match
            count += 1;

            if needs_substitution {
                // Build replacement with capture substitution
                let matched_text = &text[c2b[pos]..c2b[end_pos]];
                let replaced = substitute_captures(replacement, matched_text, &captures)?;
                result.push_str(&replaced);
            } else {
                // Fast path: no % in replacement, just copy it
                result.push_str(replacement);
            }

            // Handle empty vs non-empty match
            if end_pos == pos {
                // Empty match: copy char and advance
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
            // No match, copy char if within bounds
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
    captures: &[CaptureValue],
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
                        match &captures[capture_idx - 1] {
                            CaptureValue::String(s) => result.push_str(s),
                            CaptureValue::Position(p) => result.push_str(&p.to_string()),
                        }
                    } else if captures.is_empty() && capture_idx == 1 {
                        // Lua special case: when no captures, %1 refers to the whole match
                        result.push_str(full_match);
                    } else {
                        // Invalid capture index
                        return Err(format!("invalid capture index %{}", capture_idx));
                    }
                    i += 2;
                } else {
                    // Invalid escape sequence in replacement
                    return Err(format!("invalid use of '%' in replacement string"));
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
pub fn try_match(
    pattern: &Pattern,
    text: &[char],
    c2b: &[usize],
    pos: usize,
) -> Option<(usize, Vec<CaptureValue>)> {
    let mut captures = Vec::new();
    let mut capture_opens: Vec<(usize, usize)> = Vec::new();
    match match_impl(pattern, text, c2b, pos, &mut captures, &mut capture_opens) {
        Some(end_pos) => {
            // Filter out any remaining Open markers (shouldn't happen in valid patterns)
            Some((end_pos, captures))
        }
        None => None,
    }
}

/// Match a sequence of remaining patterns (recursive, enabling full backtracking)
fn match_rest(
    patterns: &[Pattern],
    text: &[char],
    c2b: &[usize],
    pos: usize,
    captures: &mut Vec<CaptureValue>,
    capture_opens: &mut Vec<(usize, usize)>,
) -> Option<usize> {
    if patterns.is_empty() {
        return Some(pos);
    }
    if patterns.len() == 1 {
        return match_impl(&patterns[0], text, c2b, pos, captures, capture_opens);
    }
    match_impl(
        &Pattern::Seq(patterns.to_vec()),
        text,
        c2b,
        pos,
        captures,
        capture_opens,
    )
}

fn match_impl(
    pattern: &Pattern,
    text: &[char],
    c2b: &[usize],
    mut pos: usize,
    captures: &mut Vec<CaptureValue>,
    capture_opens: &mut Vec<(usize, usize)>,
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
        Pattern::InvertedClass(class) => {
            if pos < text.len() && !class.matches(text[pos]) {
                Some(pos + 1)
            } else {
                None
            }
        }
        Pattern::Set { items, negated } => {
            if pos >= text.len() {
                return None;
            }
            let ch = text[pos];
            let found = items.iter().any(|item| item.matches(ch));
            if *negated != found {
                // XOR: match if (negated and not found) or (not negated and found)
                Some(pos + 1)
            } else {
                None
            }
        }
        Pattern::Seq(patterns) => {
            // Helper: extract repeat info from a pattern, including Capture(Repeat{...})
            fn get_repeat_info(pat: &Pattern) -> Option<(&RepeatMode, &Pattern, bool)> {
                match pat {
                    Pattern::Repeat { mode, pattern } => Some((mode, pattern, false)),
                    _ => None,
                }
            }

            let mut i = 0;
            while i < patterns.len() {
                if i + 1 < patterns.len() {
                    if let Some((mode, inner, is_capture)) = get_repeat_info(&patterns[i]) {
                        let rest_patterns = &patterns[i + 1..];
                        let saved_captures_len = captures.len();
                        let saved_opens = capture_opens.clone();
                        let start_pos = pos;

                        match mode {
                            RepeatMode::Lazy => {
                                let mut try_pos = pos;
                                loop {
                                    // For captured lazy repeats, push capture before trying rest
                                    if is_capture {
                                        let captured: String =
                                            text[start_pos..try_pos].iter().collect();
                                        captures.push(CaptureValue::String(captured));
                                    }
                                    if let Some(end) = match_rest(
                                        rest_patterns,
                                        text,
                                        c2b,
                                        try_pos,
                                        captures,
                                        capture_opens,
                                    ) {
                                        return Some(end);
                                    }
                                    captures.truncate(saved_captures_len);
                                    *capture_opens = saved_opens.clone();

                                    if try_pos >= text.len() {
                                        return None;
                                    }

                                    match match_impl(
                                        inner,
                                        text,
                                        c2b,
                                        try_pos,
                                        captures,
                                        capture_opens,
                                    ) {
                                        Some(new_pos) => {
                                            captures.truncate(saved_captures_len);
                                            *capture_opens = saved_opens.clone();
                                            if new_pos == try_pos {
                                                try_pos += 1;
                                            } else {
                                                try_pos = new_pos;
                                            }
                                        }
                                        None => return None,
                                    }
                                }
                            }
                            RepeatMode::ZeroOrMore => {
                                // Collect all possible match positions
                                let mut match_positions = vec![pos];
                                let mut curr_pos = pos;
                                while let Some(new_pos) =
                                    match_impl(inner, text, c2b, curr_pos, captures, capture_opens)
                                {
                                    captures.truncate(saved_captures_len);
                                    *capture_opens = saved_opens.clone();
                                    if new_pos == curr_pos {
                                        break;
                                    }
                                    match_positions.push(new_pos);
                                    curr_pos = new_pos;
                                }
                                captures.truncate(saved_captures_len);
                                *capture_opens = saved_opens.clone();

                                // Try from longest match to shortest
                                for &try_pos in match_positions.iter().rev() {
                                    if is_capture {
                                        let captured: String =
                                            text[start_pos..try_pos].iter().collect();
                                        captures.push(CaptureValue::String(captured));
                                    }
                                    if let Some(end) = match_rest(
                                        rest_patterns,
                                        text,
                                        c2b,
                                        try_pos,
                                        captures,
                                        capture_opens,
                                    ) {
                                        return Some(end);
                                    }
                                    captures.truncate(saved_captures_len);
                                    *capture_opens = saved_opens.clone();
                                }
                                return None;
                            }
                            RepeatMode::OneOrMore => {
                                // Must match at least once
                                let first_pos = match match_impl(
                                    inner,
                                    text,
                                    c2b,
                                    pos,
                                    captures,
                                    capture_opens,
                                ) {
                                    Some(p) => p,
                                    None => return None,
                                };
                                captures.truncate(saved_captures_len);
                                *capture_opens = saved_opens.clone();

                                // Collect all possible match positions
                                let mut match_positions = vec![first_pos];
                                let mut curr_pos = first_pos;
                                while let Some(new_pos) =
                                    match_impl(inner, text, c2b, curr_pos, captures, capture_opens)
                                {
                                    captures.truncate(saved_captures_len);
                                    *capture_opens = saved_opens.clone();
                                    if new_pos == curr_pos {
                                        break;
                                    }
                                    match_positions.push(new_pos);
                                    curr_pos = new_pos;
                                }
                                captures.truncate(saved_captures_len);
                                *capture_opens = saved_opens.clone();

                                // Try from longest match to shortest
                                for &try_pos in match_positions.iter().rev() {
                                    if is_capture {
                                        let captured: String =
                                            text[start_pos..try_pos].iter().collect();
                                        captures.push(CaptureValue::String(captured));
                                    }
                                    if let Some(end) = match_rest(
                                        rest_patterns,
                                        text,
                                        c2b,
                                        try_pos,
                                        captures,
                                        capture_opens,
                                    ) {
                                        return Some(end);
                                    }
                                    captures.truncate(saved_captures_len);
                                    *capture_opens = saved_opens.clone();
                                }
                                return None;
                            }
                            RepeatMode::ZeroOrOne => {
                                // Try matching one
                                if let Some(one_pos) =
                                    match_impl(inner, text, c2b, pos, captures, capture_opens)
                                {
                                    captures.truncate(saved_captures_len);
                                    *capture_opens = saved_opens.clone();
                                    if is_capture {
                                        let captured: String =
                                            text[start_pos..one_pos].iter().collect();
                                        captures.push(CaptureValue::String(captured));
                                    }
                                    if let Some(end) = match_rest(
                                        rest_patterns,
                                        text,
                                        c2b,
                                        one_pos,
                                        captures,
                                        capture_opens,
                                    ) {
                                        return Some(end);
                                    }
                                    captures.truncate(saved_captures_len);
                                    *capture_opens = saved_opens.clone();
                                }

                                // Try matching zero (skip)
                                if is_capture {
                                    captures.push(CaptureValue::String(String::new()));
                                }
                                if let Some(end) = match_rest(
                                    rest_patterns,
                                    text,
                                    c2b,
                                    pos,
                                    captures,
                                    capture_opens,
                                ) {
                                    return Some(end);
                                }
                                captures.truncate(saved_captures_len);
                                *capture_opens = saved_opens.clone();
                                return None;
                            }
                        }
                    }
                }

                // Standalone ZeroOrOne at end of pattern (no rest)
                if let Some((RepeatMode::ZeroOrOne, inner, is_capture)) =
                    get_repeat_info(&patterns[i])
                {
                    let start_pos = pos;
                    if let Some(one_pos) =
                        match_impl(inner, text, c2b, pos, captures, capture_opens)
                    {
                        if is_capture {
                            let captured: String = text[start_pos..one_pos].iter().collect();
                            captures.push(CaptureValue::String(captured));
                        }
                        pos = one_pos;
                    } else if is_capture {
                        captures.push(CaptureValue::String(String::new()));
                    }
                    i += 1;
                    continue;
                }

                // Normal matching for this pattern
                match match_impl(&patterns[i], text, c2b, pos, captures, capture_opens) {
                    Some(new_pos) => pos = new_pos,
                    None => return None,
                }
                i += 1;
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
                    while let Some(new_pos) =
                        match_impl(inner, text, c2b, last_pos, captures, capture_opens)
                    {
                        if new_pos == last_pos {
                            break; // Avoid infinite loop
                        }
                        last_pos = new_pos;
                    }
                    Some(last_pos)
                }
                RepeatMode::OneOrMore => {
                    // Must match at least once
                    let first_pos = match_impl(inner, text, c2b, pos, captures, capture_opens)?;
                    let mut last_pos = first_pos;
                    while let Some(new_pos) =
                        match_impl(inner, text, c2b, last_pos, captures, capture_opens)
                    {
                        if new_pos == last_pos {
                            break;
                        }
                        last_pos = new_pos;
                    }
                    Some(last_pos)
                }
                RepeatMode::ZeroOrOne => {
                    // Optional: try to match once
                    if let Some(new_pos) =
                        match_impl(inner, text, c2b, pos, captures, capture_opens)
                    {
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
        Pattern::CaptureStart => {
            // Pre-allocate a capture slot at the correct position in the captures vec
            let idx = captures.len();
            captures.push(CaptureValue::String(String::new())); // placeholder
            capture_opens.push((idx, pos)); // (capture index, start char position)
            Some(pos)
        }
        Pattern::CaptureEnd => {
            // Close the most recent open capture, fill in the pre-allocated slot
            if let Some((idx, start)) = capture_opens.pop() {
                let captured: String = text[start..pos].iter().collect();
                captures[idx] = CaptureValue::String(captured);
                Some(pos)
            } else {
                None // Unmatched capture end
            }
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
        Pattern::PositionCapture => {
            // Position capture: push current 1-based byte position
            captures.push(CaptureValue::Position(c2b[pos] + 1));
            Some(pos)
        }
        Pattern::Backref(n) => {
            // Backreference to capture N (1-based)
            if *n > captures.len() {
                return None;
            }
            let cap = &captures[*n - 1];
            match cap {
                CaptureValue::String(s) => {
                    let cap_chars: Vec<char> = s.chars().collect();
                    let cap_len = cap_chars.len();
                    if pos + cap_len > text.len() {
                        return None;
                    }
                    for i in 0..cap_len {
                        if text[pos + i] != cap_chars[i] {
                            return None;
                        }
                    }
                    Some(pos + cap_len)
                }
                CaptureValue::Position(_) => None, // Position captures can't be backreferenced
            }
        }
        Pattern::Balanced { open, close } => {
            if pos >= text.len() || text[pos] != *open {
                return None;
            }

            let mut depth = 1;
            let mut curr = pos + 1;

            while curr < text.len() && depth > 0 {
                // Check close BEFORE open (important when open == close)
                if text[curr] == *close {
                    depth -= 1;
                } else if text[curr] == *open {
                    depth += 1;
                }
                curr += 1;
            }

            if depth == 0 { Some(curr) } else { None }
        }
        Pattern::Frontier { items, negated } => {
            // %f[set] matches empty string at transition boundary
            // Previous char must NOT match [set], current char must match [set]
            // At start of string, previous char is '\0'
            // At end of string, current char is '\0'
            let prev_char = if pos > 0 { text[pos - 1] } else { '\0' };
            let curr_char = if pos < text.len() { text[pos] } else { '\0' };

            let matches_set = |c: char| -> bool {
                let mut result = false;
                for item in items {
                    if item.matches(c) {
                        result = true;
                        break;
                    }
                }
                if *negated { !result } else { result }
            };

            if !matches_set(prev_char) && matches_set(curr_char) {
                Some(pos) // Frontier matches an empty string (zero-width)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::stdlib::string::pattern::parse_pattern;

    use super::*;

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

    #[test]
    fn test_inverted_class_s() {
        let pattern = parse_pattern("%S").unwrap();
        eprintln!("Parsed pattern: {:?}", pattern);
        let text = "abc";
        let result = find(text, &pattern, 0);
        eprintln!("Find result for '%S' in 'abc': {:?}", result);
        assert!(result.is_some(), "%S should match 'a' in 'abc'");
        let (start, end, _) = result.unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 1);
    }
}
