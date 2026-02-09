// Lua pattern parser
// Parses Lua pattern strings into a structured representation

#[derive(Debug, Clone)]
pub enum Pattern {
    /// Literal character
    Char(char),
    /// Any character (.)
    Dot,
    /// Character class (%a, %d, etc.)
    Class(CharClass),
    /// Inverted character class (%A, %D, etc.)
    InvertedClass(CharClass),
    /// Character set ([abc], [^abc])
    Set { items: Vec<SetItem>, negated: bool },
    /// Sequence of patterns
    Seq(Vec<Pattern>),
    /// Repetition (*, +, -, ?)
    Repeat {
        pattern: Box<Pattern>,
        mode: RepeatMode,
    },
    /// Position capture () - captures current position as a number
    PositionCapture,
    /// Capture start marker (emitted by parser, tracks capture boundaries in flat Seq)
    CaptureStart,
    /// Capture end marker
    CaptureEnd,
    /// Anchor (^, $)
    Anchor(AnchorType),
    /// Balanced match (%bxy)
    Balanced { open: char, close: char },
    /// Backreference (%1-%9)
    Backref(usize),
    /// Frontier pattern (%f[set]) - matches empty string at transition boundary
    Frontier { items: Vec<SetItem>, negated: bool },
}

/// An item inside a character set [...]
#[derive(Debug, Clone)]
pub enum SetItem {
    Char(char),
    Range(char, char),
    Class(CharClass),
    InvertedClass(CharClass),
}

impl SetItem {
    pub fn matches(&self, c: char) -> bool {
        match self {
            SetItem::Char(ch) => c == *ch,
            SetItem::Range(start, end) => c >= *start && c <= *end,
            SetItem::Class(class) => class.matches(c),
            SetItem::InvertedClass(class) => !class.matches(c),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharClass {
    Letter,   // %a
    Control,  // %c
    Digit,    // %d
    Graph,    // %g
    Lower,    // %l
    Punct,    // %p
    Space,    // %s
    Upper,    // %u
    AlphaNum, // %w
    Hex,      // %x
    Any,      // %z (deprecated, but supported)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    ZeroOrMore, // *
    OneOrMore,  // +
    ZeroOrOne,  // ?
    Lazy,       // - (non-greedy)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorType {
    Start, // ^
    End,   // $
}

impl CharClass {
    pub fn matches(&self, c: char) -> bool {
        match self {
            CharClass::Letter => c.is_alphabetic(),
            CharClass::Control => c.is_control(),
            CharClass::Digit => c.is_ascii_digit(),
            CharClass::Graph => c.is_ascii_graphic(),
            CharClass::Lower => c.is_lowercase(),
            CharClass::Punct => c.is_ascii_punctuation(),
            CharClass::Space => c.is_whitespace(),
            CharClass::Upper => c.is_uppercase(),
            CharClass::AlphaNum => c.is_alphanumeric(),
            CharClass::Hex => c.is_ascii_hexdigit(),
            CharClass::Any => c == '\0',
        }
    }
}

impl Pattern {
    /// Check if pattern is a simple literal string (no special characters)
    /// Returns Some(string) if it's a simple literal, None otherwise
    pub fn as_literal_string(&self) -> Option<String> {
        match self {
            Pattern::Char(c) => Some(c.to_string()),
            Pattern::Seq(patterns) => {
                let mut result = String::new();
                for pat in patterns {
                    match pat.as_literal_string() {
                        Some(s) => result.push_str(&s),
                        None => return None,
                    }
                }
                Some(result)
            }
            _ => None,
        }
    }
}

/// Parse a Lua pattern string
pub fn parse_pattern(pattern: &str) -> Result<Pattern, String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut total_captures: usize = 0;
    let mut open_captures: Vec<usize> = Vec::new();
    let (pat, _) = parse_seq(&chars, 0, false, &mut total_captures, &mut open_captures)?;
    Ok(pat)
}

fn parse_seq(chars: &[char], mut pos: usize, in_capture: bool, total_captures: &mut usize, open_captures: &mut Vec<usize>) -> Result<(Pattern, usize), String> {
    let mut seq = Vec::new();

    while pos < chars.len() {
        let c = chars[pos];

        match c {
            ')' if in_capture => {
                // End of capture group
                break;
            }
            ')' => {
                // Unmatched closing parenthesis
                return Err("invalid pattern capture".to_string());
            }
            '^' if pos == 0 && seq.is_empty() => {
                seq.push(Pattern::Anchor(AnchorType::Start));
                pos += 1;
            }
            '$' if pos == chars.len() - 1 => {
                seq.push(Pattern::Anchor(AnchorType::End));
                pos += 1;
            }
            '.' => {
                seq.push(Pattern::Dot);
                pos += 1;
            }
            '%' => {
                // Escape sequence or character class
                pos += 1;
                if pos >= chars.len() {
                    return Err("malformed pattern (ends with '%%')".to_string());
                }
                let next = chars[pos];
                match next {
                    'a' => seq.push(Pattern::Class(CharClass::Letter)),
                    'c' => seq.push(Pattern::Class(CharClass::Control)),
                    'd' => seq.push(Pattern::Class(CharClass::Digit)),
                    'g' => seq.push(Pattern::Class(CharClass::Graph)),
                    'l' => seq.push(Pattern::Class(CharClass::Lower)),
                    'p' => seq.push(Pattern::Class(CharClass::Punct)),
                    's' => seq.push(Pattern::Class(CharClass::Space)),
                    'u' => seq.push(Pattern::Class(CharClass::Upper)),
                    'w' => seq.push(Pattern::Class(CharClass::AlphaNum)),
                    'x' => seq.push(Pattern::Class(CharClass::Hex)),
                    'z' => seq.push(Pattern::Class(CharClass::Any)),
                    'b' => {
                        // Balanced match %bxy
                        pos += 1;
                        if pos + 1 >= chars.len() {
                            return Err("malformed pattern (missing arguments to '%%b')".to_string());
                        }
                        let open = chars[pos];
                        let close = chars[pos + 1];
                        seq.push(Pattern::Balanced { open, close });
                        pos += 1;
                    }
                    'f' => {
                        // Frontier pattern %f[set]
                        pos += 1;
                        if pos >= chars.len() || chars[pos] != '[' {
                            return Err("missing '[' after '%f' in pattern".to_string());
                        }
                        let (set, new_pos) = parse_set(chars, pos)?;
                        match set {
                            Pattern::Set { items, negated } => {
                                seq.push(Pattern::Frontier { items, negated });
                            }
                            _ => return Err("invalid set after '%f'".to_string()),
                        }
                        pos = new_pos;
                        continue; // pos already advanced by parse_set
                    }
                    // Uppercase inverts the class
                    'A' | 'C' | 'D' | 'G' | 'L' | 'P' | 'S' | 'U' | 'W' | 'X' | 'Z' => {
                        let class = match next {
                            'A' => CharClass::Letter,
                            'C' => CharClass::Control,
                            'D' => CharClass::Digit,
                            'G' => CharClass::Graph,
                            'L' => CharClass::Lower,
                            'P' => CharClass::Punct,
                            'S' => CharClass::Space,
                            'U' => CharClass::Upper,
                            'W' => CharClass::AlphaNum,
                            'X' => CharClass::Hex,
                            'Z' => CharClass::Any,
                            _ => unreachable!(),
                        };
                        seq.push(Pattern::InvertedClass(class));
                    }
                    // Any other character is literal
                    c if c.is_ascii_digit() => {
                        let n = (c as u8 - b'0') as usize;
                        if n == 0 || n > *total_captures || open_captures.contains(&n) {
                            return Err(format!("invalid capture index %{}", n));
                        }
                        seq.push(Pattern::Backref(n));
                    }
                    _ => seq.push(Pattern::Char(next)),
                }
                pos += 1;
            }
            '[' => {
                // Character set
                let (set, new_pos) = parse_set(chars, pos)?;
                seq.push(set);
                pos = new_pos;
            }
            '(' => {
                // Check for position capture ()
                if pos + 1 < chars.len() && chars[pos + 1] == ')' {
                    *total_captures += 1;
                    seq.push(Pattern::PositionCapture);
                    pos += 2; // Skip both ( and )
                } else {
                    // Capture group — emit CaptureStart, inner patterns, CaptureEnd
                    *total_captures += 1;
                    let capture_idx = *total_captures;
                    open_captures.push(capture_idx);
                    let (inner, new_pos) = parse_seq(chars, pos + 1, true, total_captures, open_captures)?;
                    open_captures.pop();
                    seq.push(Pattern::CaptureStart);
                    // Flatten inner Seq patterns into the current sequence
                    match inner {
                        Pattern::Seq(inner_pats) => {
                            for p in inner_pats {
                                seq.push(p);
                            }
                        }
                        other => seq.push(other),
                    }
                    seq.push(Pattern::CaptureEnd);
                    pos = new_pos + 1; // Skip closing )
                }
            }
            '*' | '+' | '?' | '-' => {
                // In standard Lua, quantifiers only apply after a quantifiable
                // pattern item (literal char, '.', '%x', '[set]').
                // If seq is empty or the last element is not quantifiable,
                // treat these as literal characters.
                let can_quantify = if let Some(last) = seq.last() {
                    matches!(
                        last,
                        Pattern::Char(_)
                            | Pattern::Dot
                            | Pattern::Class(_)
                            | Pattern::InvertedClass(_)
                            | Pattern::Set { .. }
                    )
                } else {
                    false
                };

                if !can_quantify {
                    // Treat as literal character
                    seq.push(Pattern::Char(c));
                    pos += 1;
                } else {
                    let mode = match c {
                        '*' => RepeatMode::ZeroOrMore,
                        '+' => RepeatMode::OneOrMore,
                        '?' => RepeatMode::ZeroOrOne,
                        '-' => RepeatMode::Lazy,
                        _ => unreachable!(),
                    };
                    let last = seq.pop().unwrap();
                    seq.push(Pattern::Repeat {
                        pattern: Box::new(last),
                        mode,
                    });
                    pos += 1;
                }
            }
            _ => {
                // Literal character
                seq.push(Pattern::Char(c));
                pos += 1;
            }
        }
    }

    // If we're inside a capture and didn't find closing ')', it's malformed
    if in_capture && (pos >= chars.len() || chars[pos] != ')') {
        return Err("malformed pattern (unfinished capture)".to_string());
    }

    if seq.len() == 1 {
        Ok((seq.into_iter().next().unwrap(), pos))
    } else {
        Ok((Pattern::Seq(seq), pos))
    }
}

fn parse_set(chars: &[char], start: usize) -> Result<(Pattern, usize), String> {
    let mut pos = start + 1; // Skip '['
    if pos >= chars.len() {
        return Err("malformed pattern (missing ']')".to_string());
    }

    let negated = chars[pos] == '^';
    if negated {
        pos += 1;
    }

    // Handle ']' as first char in set (literal ']')
    let mut items = Vec::new();
    if pos < chars.len() && chars[pos] == ']' {
        items.push(SetItem::Char(']'));
        pos += 1;
    }

    while pos < chars.len() && chars[pos] != ']' {
        let c = chars[pos];
        if c == '%' && pos + 1 < chars.len() {
            pos += 1;
            let next = chars[pos];
            // Check if it's a character class
            match next {
                'a' => items.push(SetItem::Class(CharClass::Letter)),
                'c' => items.push(SetItem::Class(CharClass::Control)),
                'd' => items.push(SetItem::Class(CharClass::Digit)),
                'g' => items.push(SetItem::Class(CharClass::Graph)),
                'l' => items.push(SetItem::Class(CharClass::Lower)),
                'p' => items.push(SetItem::Class(CharClass::Punct)),
                's' => items.push(SetItem::Class(CharClass::Space)),
                'u' => items.push(SetItem::Class(CharClass::Upper)),
                'w' => items.push(SetItem::Class(CharClass::AlphaNum)),
                'x' => items.push(SetItem::Class(CharClass::Hex)),
                'z' => items.push(SetItem::Class(CharClass::Any)),
                'A' => items.push(SetItem::InvertedClass(CharClass::Letter)),
                'C' => items.push(SetItem::InvertedClass(CharClass::Control)),
                'D' => items.push(SetItem::InvertedClass(CharClass::Digit)),
                'G' => items.push(SetItem::InvertedClass(CharClass::Graph)),
                'L' => items.push(SetItem::InvertedClass(CharClass::Lower)),
                'P' => items.push(SetItem::InvertedClass(CharClass::Punct)),
                'S' => items.push(SetItem::InvertedClass(CharClass::Space)),
                'U' => items.push(SetItem::InvertedClass(CharClass::Upper)),
                'W' => items.push(SetItem::InvertedClass(CharClass::AlphaNum)),
                'X' => items.push(SetItem::InvertedClass(CharClass::Hex)),
                'Z' => items.push(SetItem::InvertedClass(CharClass::Any)),
                _ => items.push(SetItem::Char(next)), // literal escaped char
            }
        } else if pos + 2 < chars.len() && chars[pos + 1] == '-' && chars[pos + 2] != ']' {
            // Range: a-z
            let start_char = c;
            let end_char = chars[pos + 2];
            items.push(SetItem::Range(start_char, end_char));
            pos += 2;
        } else {
            items.push(SetItem::Char(c));
        }
        pos += 1;
    }

    if pos >= chars.len() {
        return Err("malformed pattern (missing ']')".to_string());
    }

    Ok((
        Pattern::Set {
            items,
            negated,
        },
        pos + 1, // Skip ']'
    ))
}
