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
    /// Character set ([abc], [^abc])
    Set { chars: Vec<char>, negated: bool },
    /// Sequence of patterns
    Seq(Vec<Pattern>),
    /// Repetition (*, +, -, ?)
    Repeat {
        pattern: Box<Pattern>,
        mode: RepeatMode,
    },
    /// Capture group
    Capture(Box<Pattern>),
    /// Anchor (^, $)
    Anchor(AnchorType),
    /// Balanced match (%bxy)
    Balanced { open: char, close: char },
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

/// Parse a Lua pattern string
pub fn parse_pattern(pattern: &str) -> Result<Pattern, String> {
    let chars: Vec<char> = pattern.chars().collect();
    let (pat, _) = parse_seq(&chars, 0, false)?;
    Ok(pat)
}

fn parse_seq(chars: &[char], mut pos: usize, in_capture: bool) -> Result<(Pattern, usize), String> {
    let mut seq = Vec::new();

    while pos < chars.len() {
        let c = chars[pos];

        match c {
            ')' if in_capture => {
                // End of capture group
                break;
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
                    return Err("incomplete escape at end of pattern".to_string());
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
                            return Err("incomplete %b pattern".to_string());
                        }
                        let open = chars[pos];
                        let close = chars[pos + 1];
                        seq.push(Pattern::Balanced { open, close });
                        pos += 1;
                    }
                    // Uppercase inverts the class
                    'A' | 'C' | 'D' | 'G' | 'L' | 'P' | 'S' | 'U' | 'W' | 'X' | 'Z' => {
                        return Err(format!(
                            "inverted character class %{} not yet supported",
                            next
                        ));
                    }
                    // Any other character is literal
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
                // Capture group
                let (inner, new_pos) = parse_seq(chars, pos + 1, true)?;
                seq.push(Pattern::Capture(Box::new(inner)));
                pos = new_pos + 1; // Skip closing )
            }
            '*' | '+' | '?' | '-' => {
                if seq.is_empty() {
                    return Err(format!("unexpected repetition operator '{}'", c));
                }
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
            _ => {
                // Literal character
                seq.push(Pattern::Char(c));
                pos += 1;
            }
        }
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
        return Err("incomplete character set".to_string());
    }

    let negated = chars[pos] == '^';
    if negated {
        pos += 1;
    }

    let mut set_chars = Vec::new();

    while pos < chars.len() && chars[pos] != ']' {
        let c = chars[pos];
        if c == '%' && pos + 1 < chars.len() {
            // Escape in set
            pos += 1;
            set_chars.push(chars[pos]);
        } else if pos + 2 < chars.len() && chars[pos + 1] == '-' && chars[pos + 2] != ']' {
            // Range: a-z
            let start_char = c;
            let end_char = chars[pos + 2];
            for ch in (start_char as u32)..=(end_char as u32) {
                if let Some(ch) = char::from_u32(ch) {
                    set_chars.push(ch);
                }
            }
            pos += 2;
        } else {
            set_chars.push(c);
        }
        pos += 1;
    }

    if pos >= chars.len() {
        return Err("unclosed character set".to_string());
    }

    Ok((
        Pattern::Set {
            chars: set_chars,
            negated,
        },
        pos + 1, // Skip ']'
    ))
}
