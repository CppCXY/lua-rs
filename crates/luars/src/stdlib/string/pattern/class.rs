// Character class matching for Lua patterns
// Handles %a, %d, %l, %u, %w, %s, %p, %c, %g, %x and their uppercase inverses
// Also handles [set] matching

/// Check if a character matches a Lua character class letter.
/// `cl` is the class letter (lowercase): 'a','c','d','g','l','p','s','u','w','x'
#[inline(always)]
pub fn match_class(c: char, cl: char) -> bool {
    match cl {
        'a' => c.is_alphabetic(),
        'c' => c.is_control(),
        'd' => c.is_ascii_digit(),
        'g' => c.is_ascii_graphic(),
        'l' => c.is_lowercase(),
        'p' => c.is_ascii_punctuation(),
        's' => c.is_whitespace(),
        'u' => c.is_uppercase(),
        'w' => c.is_alphanumeric(),
        'x' => c.is_ascii_hexdigit(),
        'z' => c == '\0',
        _ => c == cl, // not a class letter, match literally
    }
}

/// Check if `cl` is a known Lua class letter (used to distinguish %x class vs %x literal).
#[inline(always)]
fn is_class_letter(cl: char) -> bool {
    matches!(
        cl,
        'a' | 'c' | 'd' | 'g' | 'l' | 'p' | 's' | 'u' | 'w' | 'x' | 'z'
            | 'A' | 'C' | 'D' | 'G' | 'L' | 'P' | 'S' | 'U' | 'W' | 'X' | 'Z'
    )
}

/// Match a single character against a single pattern element starting at `pat[pp]`.
/// Returns the pattern index AFTER the element (so caller can continue).
///
/// Pattern elements:
///   - `.`        → any character
///   - `%a`       → class (lowercase = match, uppercase = inverted)
///   - `%x` where x is not a class → literal x
///   - `[set]`    → character set
///   - literal    → exact match
///
/// `None` return means the character did NOT match.
/// `Some(next_pp)` means it matched, and `next_pp` is the index past this element.
pub fn singlematch(c: char, pat: &[char], pp: usize) -> bool {
    match pat[pp] {
        '.' => true,
        '%' => {
            let cl = pat[pp + 1];
            if cl.is_ascii_uppercase() && is_class_letter(cl) {
                // Inverted class: %A matches non-alphabetic
                !match_class(c, cl.to_ascii_lowercase())
            } else {
                match_class(c, cl)
            }
        }
        '[' => matchset(c, pat, pp),
        _ => c == pat[pp],
    }
}

/// Return the pattern index after the current single-element (past `[]`, `%x`, `.`, or literal).
/// This does NOT consume repetition suffixes (*, +, -, ?).
#[inline]
pub fn element_end(pat: &[char], pp: usize) -> usize {
    match pat[pp] {
        '%' => {
            // %bxy is handled separately in engine, not here
            pp + 2
        }
        '[' => {
            let mut i = pp + 1;
            // handle ^
            if i < pat.len() && pat[i] == '^' {
                i += 1;
            }
            // handle ] as first char in set
            if i < pat.len() && pat[i] == ']' {
                i += 1;
            }
            while i < pat.len() && pat[i] != ']' {
                if pat[i] == '%' && i + 1 < pat.len() {
                    i += 1; // skip escaped char
                }
                i += 1;
            }
            i + 1 // past ']'
        }
        _ => pp + 1,
    }
}

/// Match character `c` against a `[set]` starting at `pat[pp]` (pp points to `[`).
fn matchset(c: char, pat: &[char], pp: usize) -> bool {
    let mut i = pp + 1; // skip '['
    let negated = i < pat.len() && pat[i] == '^';
    if negated {
        i += 1;
    }

    let mut matched = false;

    // Handle ']' as first char in set (literal ']')
    if i < pat.len() && pat[i] == ']' {
        if c == ']' {
            matched = true;
        }
        i += 1;
    }

    while i < pat.len() && pat[i] != ']' {
        if pat[i] == '%' && i + 1 < pat.len() {
            i += 1;
            let cl = pat[i];
            if cl.is_ascii_uppercase() && is_class_letter(cl) {
                if !match_class(c, cl.to_ascii_lowercase()) {
                    matched = true;
                }
            } else if match_class(c, cl) {
                matched = true;
            }
            i += 1;
        } else if i + 2 < pat.len() && pat[i + 1] == '-' && pat[i + 2] != ']' {
            // Range: a-z
            if c >= pat[i] && c <= pat[i + 2] {
                matched = true;
            }
            i += 3;
        } else {
            if c == pat[i] {
                matched = true;
            }
            i += 1;
        }
    }

    if negated { !matched } else { matched }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_class() {
        assert!(match_class('a', 'a'));
        assert!(match_class('Z', 'a'));
        assert!(!match_class('1', 'a'));
        assert!(match_class('5', 'd'));
        assert!(!match_class('x', 'd'));
        assert!(match_class(' ', 's'));
        assert!(match_class('\t', 's'));
        assert!(!match_class('a', 's'));
    }

    #[test]
    fn test_singlematch_dot() {
        let p: Vec<char> = ".".chars().collect();
        assert!(singlematch('x', &p, 0));
        assert!(singlematch(' ', &p, 0));
    }

    #[test]
    fn test_singlematch_class() {
        let p: Vec<char> = "%d".chars().collect();
        assert!(singlematch('5', &p, 0));
        assert!(!singlematch('a', &p, 0));
    }

    #[test]
    fn test_singlematch_inverted_class() {
        let p: Vec<char> = "%D".chars().collect();
        assert!(!singlematch('5', &p, 0));
        assert!(singlematch('a', &p, 0));
    }

    #[test]
    fn test_singlematch_set() {
        let p: Vec<char> = "[abc]".chars().collect();
        assert!(singlematch('a', &p, 0));
        assert!(singlematch('c', &p, 0));
        assert!(!singlematch('d', &p, 0));
    }

    #[test]
    fn test_singlematch_negated_set() {
        let p: Vec<char> = "[^abc]".chars().collect();
        assert!(!singlematch('a', &p, 0));
        assert!(singlematch('d', &p, 0));
    }

    #[test]
    fn test_singlematch_range() {
        let p: Vec<char> = "[a-z]".chars().collect();
        assert!(singlematch('m', &p, 0));
        assert!(!singlematch('M', &p, 0));
    }

    #[test]
    fn test_singlematch_set_with_class() {
        let p: Vec<char> = "[%d_]".chars().collect();
        assert!(singlematch('5', &p, 0));
        assert!(singlematch('_', &p, 0));
        assert!(!singlematch('a', &p, 0));
    }

    #[test]
    fn test_element_end() {
        let p: Vec<char> = "a".chars().collect();
        assert_eq!(element_end(&p, 0), 1);

        let p: Vec<char> = "%d".chars().collect();
        assert_eq!(element_end(&p, 0), 2);

        let p: Vec<char> = "[abc]".chars().collect();
        assert_eq!(element_end(&p, 0), 5);

        let p: Vec<char> = "[^a-z%d]".chars().collect();
        assert_eq!(element_end(&p, 0), 8);
    }

    #[test]
    fn test_set_bracket_first() {
        // ] as first char in set
        let p: Vec<char> = "[]abc]".chars().collect();
        assert!(singlematch(']', &p, 0));
        assert!(singlematch('a', &p, 0));
        assert!(!singlematch('x', &p, 0));
    }
}
