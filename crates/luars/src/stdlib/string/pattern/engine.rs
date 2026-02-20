// Core pattern matching engine — direct interpretation, no AST
//
// Follows C Lua's lstrlib.c design:
// - MatchState holds text, pattern, captures
// - match_impl recursively walks the pattern with backtracking
// - Fixed capture slots (no heap alloc during matching)

use super::class::{element_end, singlematch};

/// Maximum number of captures (Lua limit)
pub const LUA_MAXCAPTURES: usize = 32;
/// Recursion limit to prevent stack overflow

/// Validate a pattern for common syntax errors before matching.
/// Returns Ok(()) if valid, Err(message) if malformed.
fn validate_pattern(pat: &[char]) -> Result<(), String> {
    let mut i = if !pat.is_empty() && pat[0] == '^' {
        1
    } else {
        0
    };
    while i < pat.len() {
        match pat[i] {
            '%' => {
                if i + 1 >= pat.len() {
                    return Err("malformed pattern (ends with '%')".to_string());
                }
                match pat[i + 1] {
                    'b' => {
                        if i + 3 >= pat.len() {
                            return Err("malformed pattern (missing arguments to '%b')".to_string());
                        }
                        i += 4; // skip %bxy
                    }
                    'f' => {
                        i += 2; // skip %f
                        if i >= pat.len() || pat[i] != '[' {
                            return Err("missing '[' after '%f' in pattern".to_string());
                        }
                        // validate the set
                        i = validate_set(pat, i)?;
                    }
                    _ => {
                        i += 2; // skip %x
                    }
                }
            }
            '[' => {
                i = validate_set(pat, i)?;
            }
            '(' | ')' => {
                i += 1; // capture markers — validated at match time
            }
            _ => {
                i += 1;
            }
        }
        // Skip optional repetition suffix
        if i < pat.len() && matches!(pat[i], '*' | '+' | '-' | '?') {
            i += 1;
        }
    }
    Ok(())
}

/// Validate a [set] starting at pat[i] (i points to '['). Returns index past ']'.
fn validate_set(pat: &[char], i: usize) -> Result<usize, String> {
    let mut j = i + 1; // skip '['
    // handle ^
    if j < pat.len() && pat[j] == '^' {
        j += 1;
    }
    // handle ']' as first char in set (literal)
    if j < pat.len() && pat[j] == ']' {
        j += 1;
    }
    while j < pat.len() && pat[j] != ']' {
        if pat[j] == '%' {
            j += 1; // skip escape
            if j >= pat.len() {
                return Err("malformed pattern (ends with '%')".to_string());
            }
        }
        j += 1;
    }
    if j >= pat.len() {
        return Err("malformed pattern (missing ']')".to_string());
    }
    Ok(j + 1) // past ']'
}
const MAXCCALLS: usize = 200;

/// Capture kind
#[derive(Debug, Clone, Copy)]
pub enum CapKind {
    Unfinished, // capture started but not yet closed
    Position,   // position capture ()
    Closed,     // normal closed capture
}

/// A single capture slot
#[derive(Debug, Clone, Copy)]
pub struct Capture {
    pub start: usize, // start index in text (char index)
    pub len: CaptureLen,
    pub kind: CapKind,
}

/// Capture length — either a char count or a position marker
#[derive(Debug, Clone, Copy)]
pub enum CaptureLen {
    Len(usize),
    Position, // () position capture
    Unfinished,
}

/// Match state — all matching context on the stack
pub struct MatchState<'a> {
    pub text: &'a [char], // source string as chars
    pub pat: &'a [char],  // pattern as chars
    pub captures: [Capture; LUA_MAXCAPTURES],
    pub num_captures: usize,
    pub depth: usize, // recursion counter
    // byte offsets for each char (text_bytes[i] = byte offset of text[i], text_bytes[len] = total bytes)
    pub text_bytes: &'a [usize],
    pub error: Option<String>, // error message if matching fails with a hard error
}

impl<'a> MatchState<'a> {
    pub fn new(text: &'a [char], pat: &'a [char], text_bytes: &'a [usize]) -> Self {
        Self {
            text,
            pat,
            captures: [Capture {
                start: 0,
                len: CaptureLen::Unfinished,
                kind: CapKind::Unfinished,
            }; LUA_MAXCAPTURES],
            num_captures: 0,
            depth: 0,
            text_bytes,
            error: None,
        }
    }

    /// Reset match state for reuse (avoids re-zeroing full capture array)
    #[inline]
    pub fn reset(&mut self) {
        self.num_captures = 0;
        self.depth = 0;
        self.error = None;
    }
}

/// Try to match pattern starting at `pat[pp]` against text starting at `text[si]`.
/// Returns `Some(end_si)` on success (char index past the match), `None` on failure.
///
/// This is the recursive core — equivalent to C Lua's `match` function.
pub fn match_impl(ms: &mut MatchState, si: usize, pp: usize) -> Option<usize> {
    // If an error has been set, bail immediately
    if ms.error.is_some() {
        return None;
    }
    ms.depth += 1;
    if ms.depth > MAXCCALLS {
        ms.error = Some("pattern too complex".to_string());
        ms.depth -= 1;
        return None;
    }

    let result = match_inner(ms, si, pp);
    ms.depth -= 1;
    result
}

fn match_inner(ms: &mut MatchState, mut si: usize, mut pp: usize) -> Option<usize> {
    // Tail-call optimization: loop instead of recursing for sequential elements
    loop {
        if pp >= ms.pat.len() {
            // End of pattern — match succeeded
            return Some(si);
        }

        match ms.pat[pp] {
            '(' => {
                // Start of capture
                if pp + 1 < ms.pat.len() && ms.pat[pp + 1] == ')' {
                    // Position capture ()
                    return match_position_capture(ms, si, pp + 2);
                } else {
                    return match_open_capture(ms, si, pp + 1);
                }
            }
            ')' => {
                // Close capture
                return match_close_capture(ms, si, pp + 1);
            }
            '$' if pp + 1 >= ms.pat.len() => {
                // Anchor at end — succeed only if text exhausted
                return if si == ms.text.len() { Some(si) } else { None };
            }
            '%' if pp + 1 < ms.pat.len() => {
                match ms.pat[pp + 1] {
                    'b' => {
                        // Balanced match %bxy
                        return match_balanced(ms, si, pp);
                    }
                    'f' => {
                        // Frontier %f[set]
                        return match_frontier(ms, si, pp);
                    }
                    c if c.is_ascii_digit() => {
                        // Back reference %0-%9
                        return match_backref(ms, si, pp);
                    }
                    _ => {
                        // Character class %x — fall through to normal match
                    }
                }
            }
            _ => {}
        }

        // Normal pattern element (literal, `.`, `%class`, `[set]`)
        let ep = element_end(ms.pat, pp); // index past the element

        // Check for repetition suffix
        if ep < ms.pat.len() {
            match ms.pat[ep] {
                '*' => return match_greedy(ms, si, pp, ep + 1, 0),
                '+' => return match_greedy(ms, si, pp, ep + 1, 1),
                '-' => return match_lazy(ms, si, pp, ep + 1),
                '?' => return match_optional(ms, si, pp, ep + 1),
                _ => {}
            }
        }

        // No repetition — single match required
        if si < ms.text.len() && singlematch(ms.text[si], ms.pat, pp) {
            // Matched one char. Tail-call: advance both si and pp.
            si += 1;
            pp = ep;
            continue; // loop (tail-call optimization)
        }
        return None;
    }
}

/// Greedy repetition (*, +)
/// `min` is 0 for *, 1 for +
fn match_greedy(
    ms: &mut MatchState,
    si: usize,
    pp: usize, // pattern element start
    rp: usize, // rest of pattern (after repetition char)
    min: usize,
) -> Option<usize> {
    // Count maximum matches
    let mut count = 0;
    while si + count < ms.text.len() && singlematch(ms.text[si + count], ms.pat, pp) {
        count += 1;
    }
    // Try from most to least (greedy)
    while count >= min {
        if let Some(end) = match_impl(ms, si + count, rp) {
            return Some(end);
        }
        if count == 0 {
            break;
        }
        count -= 1;
    }
    None
}

/// Lazy repetition (-)
fn match_lazy(ms: &mut MatchState, si: usize, pp: usize, rp: usize) -> Option<usize> {
    let mut i = si;
    loop {
        if let Some(end) = match_impl(ms, i, rp) {
            return Some(end);
        }
        if i < ms.text.len() && singlematch(ms.text[i], ms.pat, pp) {
            i += 1;
        } else {
            return None;
        }
    }
}

/// Optional repetition (?)
fn match_optional(ms: &mut MatchState, si: usize, pp: usize, rp: usize) -> Option<usize> {
    if si < ms.text.len() && singlematch(ms.text[si], ms.pat, pp) {
        if let Some(end) = match_impl(ms, si + 1, rp) {
            return Some(end);
        }
    }
    match_impl(ms, si, rp)
}

/// Open a new capture
fn match_open_capture(ms: &mut MatchState, si: usize, pp: usize) -> Option<usize> {
    let n = ms.num_captures;
    if n >= LUA_MAXCAPTURES {
        return None; // too many captures
    }
    ms.captures[n] = Capture {
        start: si,
        len: CaptureLen::Unfinished,
        kind: CapKind::Unfinished,
    };
    ms.num_captures = n + 1;
    let result = match_impl(ms, si, pp);
    if result.is_none() {
        ms.num_captures = n; // undo
    }
    result
}

/// Close the most recent unfinished capture
fn match_close_capture(ms: &mut MatchState, si: usize, pp: usize) -> Option<usize> {
    // Find the last unfinished capture
    let mut n = ms.num_captures;
    loop {
        if n == 0 {
            ms.error = Some("invalid pattern capture".to_string());
            return None; // no open capture to close
        }
        n -= 1;
        if let CaptureLen::Unfinished = ms.captures[n].len {
            ms.captures[n].len = CaptureLen::Len(si - ms.captures[n].start);
            ms.captures[n].kind = CapKind::Closed;
            let result = match_impl(ms, si, pp);
            if result.is_none() {
                // Undo close on backtrack
                ms.captures[n].len = CaptureLen::Unfinished;
                ms.captures[n].kind = CapKind::Unfinished;
            }
            return result;
        }
    }
}

/// Position capture ()
fn match_position_capture(ms: &mut MatchState, si: usize, pp: usize) -> Option<usize> {
    let n = ms.num_captures;
    if n >= LUA_MAXCAPTURES {
        return None;
    }
    ms.captures[n] = Capture {
        start: si,
        len: CaptureLen::Position,
        kind: CapKind::Position,
    };
    ms.num_captures = n + 1;
    let result = match_impl(ms, si, pp);
    if result.is_none() {
        ms.num_captures = n;
    }
    result
}

/// Balanced match %bxy
fn match_balanced(ms: &mut MatchState, si: usize, pp: usize) -> Option<usize> {
    if pp + 3 >= ms.pat.len() {
        return None; // malformed %b
    }
    let open = ms.pat[pp + 2];
    let close = ms.pat[pp + 3];

    if si >= ms.text.len() || ms.text[si] != open {
        return None;
    }

    let mut depth = 1i32;
    let mut i = si + 1;
    while i < ms.text.len() && depth > 0 {
        if ms.text[i] == close {
            depth -= 1;
        } else if ms.text[i] == open {
            depth += 1;
        }
        i += 1;
    }

    if depth != 0 {
        return None;
    }
    // pp + 4 = past %bxy
    match_impl(ms, i, pp + 4)
}

/// Frontier pattern %f[set]
fn match_frontier(ms: &mut MatchState, si: usize, pp: usize) -> Option<usize> {
    // pp points to '%', pp+1 is 'f', pp+2 should be '['
    if pp + 2 >= ms.pat.len() || ms.pat[pp + 2] != '[' {
        return None; // malformed %f
    }
    let set_start = pp + 2; // points to '['
    let set_end = element_end(ms.pat, set_start); // past ']'

    let prev_char = if si > 0 { ms.text[si - 1] } else { '\0' };
    let curr_char = if si < ms.text.len() {
        ms.text[si]
    } else {
        '\0'
    };

    let prev_matches = singlematch(prev_char, ms.pat, set_start);
    let curr_matches = singlematch(curr_char, ms.pat, set_start);

    if !prev_matches && curr_matches {
        match_impl(ms, si, set_end)
    } else {
        None
    }
}

/// Back reference %0-%9
fn match_backref(ms: &mut MatchState, si: usize, pp: usize) -> Option<usize> {
    let n = (ms.pat[pp + 1] as u32 - '0' as u32) as usize;
    // %0 is always invalid (captures are 1-indexed)
    if n == 0 || n > ms.num_captures {
        ms.error = Some(format!("invalid capture index %{}", n));
        return None;
    }
    let cap_idx = n - 1;
    let cap_len = match ms.captures[cap_idx].len {
        CaptureLen::Len(l) => l,
        _ => {
            // Unfinished or position capture — invalid backreference
            ms.error = Some(format!("invalid capture index %{}", n));
            return None;
        }
    };
    let cap_start = ms.captures[cap_idx].start;

    if si + cap_len > ms.text.len() {
        return None;
    }

    // Compare chars
    for i in 0..cap_len {
        if ms.text[si + i] != ms.text[cap_start + i] {
            return None;
        }
    }

    match_impl(ms, si + cap_len, pp + 2)
}

// ======================== Public API ========================

/// A capture value returned to callers
#[derive(Debug, Clone)]
pub enum CaptureValue {
    String(String),
    Position(usize), // 1-based byte position
}

/// Information about a single match
#[derive(Debug, Clone)]
pub struct MatchInfo {
    pub start: usize, // byte offset
    pub end: usize,   // byte offset
    pub captures: Vec<CaptureValue>,
}

/// Build char-to-byte offset map. Returns vec of len `chars.len() + 1`.
fn char_to_byte_map(text: &str) -> Vec<usize> {
    if text.is_ascii() {
        // ASCII fast path: byte offset == char index
        (0..=text.len()).collect()
    } else {
        text.char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .collect()
    }
}

/// Convert text to Vec<char>, with ASCII fast path.
#[inline]
fn text_to_chars(text: &str) -> Vec<char> {
    if text.is_ascii() {
        // ASCII fast path: skip UTF-8 decoding
        text.as_bytes().iter().map(|&b| b as char).collect()
    } else {
        text.chars().collect()
    }
}

/// Convert byte offset to char index
#[inline]
fn byte_to_char_index(text: &str, byte_pos: usize) -> usize {
    if text.is_ascii() {
        byte_pos.min(text.len())
    } else {
        let clamped = byte_pos.min(text.len());
        text[..clamped].chars().count()
    }
}

/// Check that all captures in a successful match are finished
fn check_captures(ms: &MatchState) -> Result<(), String> {
    for i in 0..ms.num_captures {
        if let CaptureLen::Unfinished = ms.captures[i].len {
            return Err("unfinished capture".to_string());
        }
    }
    Ok(())
}

/// Extract captures from MatchState into CaptureValue vec
fn extract_captures(ms: &MatchState) -> Vec<CaptureValue> {
    let mut caps = Vec::new();
    for i in 0..ms.num_captures {
        let cap = &ms.captures[i];
        match cap.len {
            CaptureLen::Position => {
                // 1-based byte position
                caps.push(CaptureValue::Position(ms.text_bytes[cap.start] + 1));
            }
            CaptureLen::Len(len) => {
                let s: String = ms.text[cap.start..cap.start + len].iter().collect();
                caps.push(CaptureValue::String(s));
            }
            CaptureLen::Unfinished => {
                // shouldn't happen after check_captures, but handle gracefully
            }
        }
    }
    caps
}

/// Find pattern in text. `init` is a 0-based byte offset.
/// Returns `(byte_start, byte_end, captures)`.
pub fn find(
    text: &str,
    pat_str: &str,
    init: usize,
) -> Result<Option<(usize, usize, Vec<CaptureValue>)>, String> {
    if init > text.len() {
        return Ok(None);
    }

    let text_chars = text_to_chars(text);
    let pat_chars = text_to_chars(pat_str);
    validate_pattern(&pat_chars)?;
    let c2b = char_to_byte_map(text);
    let init_ci = byte_to_char_index(text, init);

    let pp_start = if !pat_chars.is_empty() && pat_chars[0] == '^' {
        1
    } else {
        0
    };
    let anchored = pp_start == 1;

    let mut ms = MatchState::new(&text_chars, &pat_chars, &c2b);
    let mut si = init_ci;
    loop {
        ms.reset();
        if let Some(end_ci) = match_impl(&mut ms, si, pp_start) {
            check_captures(&ms)?;
            let caps = extract_captures(&ms);
            return Ok(Some((c2b[si], c2b[end_ci], caps)));
        }
        if let Some(err) = ms.error {
            return Err(err);
        }
        if anchored || si >= text_chars.len() {
            return Ok(None);
        }
        si += 1;
    }
}

/// Find all matches of pattern in text (for gmatch/gsub).
pub fn find_all_matches(
    text: &str,
    pat_str: &str,
    max: Option<usize>,
) -> Result<Vec<MatchInfo>, String> {
    let text_chars = text_to_chars(text);
    let pat_chars = text_to_chars(pat_str);
    validate_pattern(&pat_chars)?;
    let c2b = char_to_byte_map(text);

    let pp_start = if !pat_chars.is_empty() && pat_chars[0] == '^' {
        1
    } else {
        0
    };
    let anchored = pp_start == 1;

    let mut matches = Vec::new();
    let mut ms = MatchState::new(&text_chars, &pat_chars, &c2b);
    let mut si = 0usize;
    let mut last_was_nonempty = false;

    while si <= text_chars.len() {
        if let Some(max_count) = max {
            if matches.len() >= max_count {
                break;
            }
        }

        ms.reset();
        if let Some(end_ci) = match_impl(&mut ms, si, pp_start) {
            check_captures(&ms)?;
            let is_empty = end_ci == si;

            // Skip empty match right after non-empty match
            if is_empty && last_was_nonempty {
                if si < text_chars.len() {
                    si += 1;
                }
                last_was_nonempty = false;
                continue;
            }

            let caps = extract_captures(&ms);
            matches.push(MatchInfo {
                start: c2b[si],
                end: c2b[end_ci],
                captures: caps,
            });

            if is_empty {
                if si < text_chars.len() {
                    si += 1;
                } else {
                    break;
                }
                last_was_nonempty = false;
            } else {
                si = end_ci;
                last_was_nonempty = true;
            }
        } else {
            if let Some(err) = ms.error {
                return Err(err);
            }
            if anchored || si >= text_chars.len() {
                break;
            }
            si += 1;
            last_was_nonempty = false;
        }
    }

    Ok(matches)
}

/// Global substitution with string replacement.
pub fn gsub(
    text: &str,
    pat_str: &str,
    replacement: &str,
    max: Option<usize>,
) -> Result<(String, usize), String> {
    let text_chars = text_to_chars(text);
    let pat_chars = text_to_chars(pat_str);
    validate_pattern(&pat_chars)?;
    let c2b = char_to_byte_map(text);

    let pp_start = if !pat_chars.is_empty() && pat_chars[0] == '^' {
        1
    } else {
        0
    };
    let anchored = pp_start == 1;
    let needs_substitution = replacement.contains('%');

    let mut result = String::new();
    let mut count = 0usize;
    let mut ms = MatchState::new(&text_chars, &pat_chars, &c2b);
    let mut si = 0usize;
    let mut last_was_nonempty = false;
    // Track last byte position for copying unmatched text
    let mut last_byte_end = 0usize;

    while si <= text_chars.len() {
        if let Some(max_count) = max {
            if count >= max_count {
                break;
            }
        }

        ms.reset();
        if let Some(end_ci) = match_impl(&mut ms, si, pp_start) {
            check_captures(&ms)?;
            let is_empty = end_ci == si;

            if is_empty && last_was_nonempty {
                if si < text_chars.len() {
                    result.push(text_chars[si]);
                    last_byte_end = c2b[si + 1];
                }
                si += 1;
                last_was_nonempty = false;
                continue;
            }

            // Copy text between last match end and this match start
            let match_byte_start = c2b[si];
            let match_byte_end = c2b[end_ci];
            result.push_str(&text[last_byte_end..match_byte_start]);

            count += 1;

            if needs_substitution {
                let matched_text = &text[match_byte_start..match_byte_end];
                let replaced = substitute_captures(replacement, matched_text, &ms)?;
                result.push_str(&replaced);
            } else {
                result.push_str(replacement);
            }

            last_byte_end = match_byte_end;

            if is_empty {
                if si < text_chars.len() {
                    result.push(text_chars[si]);
                    last_byte_end = c2b[si + 1];
                }
                si += 1;
                last_was_nonempty = false;
            } else {
                si = end_ci;
                last_was_nonempty = true;
            }
        } else {
            if let Some(err) = ms.error {
                return Err(err);
            }
            if anchored || si >= text_chars.len() {
                break;
            }
            si += 1;
            last_was_nonempty = false;
        }
    }

    // Copy remaining text
    result.push_str(&text[last_byte_end..]);

    Ok((result, count))
}

/// Substitute %0-%9 and %% in replacement string using MatchState captures
fn substitute_captures(
    replacement: &str,
    full_match: &str,
    ms: &MatchState,
) -> Result<String, String> {
    let mut result = String::new();
    let repl_chars: Vec<char> = replacement.chars().collect();
    let mut i = 0;

    while i < repl_chars.len() {
        if repl_chars[i] == '%' {
            if i + 1 < repl_chars.len() {
                let next = repl_chars[i + 1];
                if next == '%' {
                    result.push('%');
                    i += 2;
                } else if next.is_ascii_digit() {
                    let n = (next as u8 - b'0') as usize;
                    if n == 0 {
                        result.push_str(full_match);
                    } else if n <= ms.num_captures {
                        let cap = &ms.captures[n - 1];
                        match cap.len {
                            CaptureLen::Len(len) => {
                                let s: String =
                                    ms.text[cap.start..cap.start + len].iter().collect();
                                result.push_str(&s);
                            }
                            CaptureLen::Position => {
                                result.push_str(&(ms.text_bytes[cap.start] + 1).to_string());
                            }
                            CaptureLen::Unfinished => {}
                        }
                    } else if ms.num_captures == 0 && n == 1 {
                        // Lua special case: no captures, %1 = whole match
                        result.push_str(full_match);
                    } else {
                        return Err(format!("invalid capture index %{}", n));
                    }
                    i += 2;
                } else {
                    return Err("invalid use of '%' in replacement string".to_string());
                }
            } else {
                result.push('%');
                i += 1;
            }
        } else {
            result.push(repl_chars[i]);
            i += 1;
        }
    }

    Ok(result)
}
