// Character class matching for Lua patterns
// Handles %a, %d, %l, %u, %w, %s, %p, %c, %g, %x and their uppercase inverses
// Also handles [set] matching

/// Check if a byte matches a Lua character class letter.
/// `cl` is the class letter byte (lowercase): 'a','c','d','g','l','p','s','u','w','x'
/// Uses ASCII-only classification to match C Lua's C-locale behavior.
#[inline(always)]
pub fn match_class(c: u8, cl: u8) -> bool {
    match cl {
        b'a' => c.is_ascii_alphabetic(),
        b'c' => c.is_ascii_control(),
        b'd' => c.is_ascii_digit(),
        b'g' => c.is_ascii_graphic(),
        b'l' => c.is_ascii_lowercase(),
        b'p' => c.is_ascii_punctuation(),
        b's' => c.is_ascii_whitespace(),
        b'u' => c.is_ascii_uppercase(),
        b'w' => c.is_ascii_alphanumeric(),
        b'x' => c.is_ascii_hexdigit(),
        b'z' => c == 0,
        _ => c == cl, // not a class letter, match literally
    }
}

/// Check if `cl` is a known Lua class letter (used to distinguish %x class vs %x literal).
#[inline(always)]
pub(crate) fn is_class_letter(cl: u8) -> bool {
    matches!(
        cl,
        b'a' | b'c'
            | b'd'
            | b'g'
            | b'l'
            | b'p'
            | b's'
            | b'u'
            | b'w'
            | b'x'
            | b'z'
            | b'A'
            | b'C'
            | b'D'
            | b'G'
            | b'L'
            | b'P'
            | b'S'
            | b'U'
            | b'W'
            | b'X'
            | b'Z'
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
#[inline]
pub fn singlematch(c: u8, pat: &[u8], pp: usize) -> bool {
    match pat[pp] {
        b'.' => true,
        b'%' => {
            let cl = pat[pp + 1];
            if cl.is_ascii_uppercase() && is_class_letter(cl) {
                // Inverted class: %A matches non-alphabetic
                !match_class(c, cl.to_ascii_lowercase())
            } else {
                match_class(c, cl)
            }
        }
        b'[' => matchset(c, pat, pp),
        _ => c == pat[pp],
    }
}

/// Return the pattern index after the current single-element (past `[]`, `%x`, `.`, or literal).
/// This does NOT consume repetition suffixes (*, +, -, ?).
#[inline]
pub fn element_end(pat: &[u8], pp: usize) -> usize {
    match pat[pp] {
        b'%' => {
            // %bxy is handled separately in engine, not here
            pp + 2
        }
        b'[' => {
            let mut i = pp + 1;
            // handle ^
            if i < pat.len() && pat[i] == b'^' {
                i += 1;
            }
            // handle ] as first char in set
            if i < pat.len() && pat[i] == b']' {
                i += 1;
            }
            while i < pat.len() && pat[i] != b']' {
                if pat[i] == b'%' && i + 1 < pat.len() {
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
#[inline]
fn matchset(c: u8, pat: &[u8], pp: usize) -> bool {
    let mut i = pp + 1; // skip '['
    let negated = i < pat.len() && pat[i] == b'^';
    if negated {
        i += 1;
    }

    let mut matched = false;

    // Handle ']' as first char in set (literal ']')
    if i < pat.len() && pat[i] == b']' {
        if c == b']' {
            matched = true;
        }
        i += 1;
    }

    while i < pat.len() && pat[i] != b']' {
        if pat[i] == b'%' && i + 1 < pat.len() {
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
        } else if i + 2 < pat.len() && pat[i + 1] == b'-' && pat[i + 2] != b']' {
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
        assert!(match_class(b'a', b'a'));
        assert!(match_class(b'Z', b'a'));
        assert!(!match_class(b'1', b'a'));
        assert!(match_class(b'5', b'd'));
        assert!(!match_class(b'x', b'd'));
        assert!(match_class(b' ', b's'));
        assert!(match_class(b'\t', b's'));
        assert!(!match_class(b'a', b's'));
    }

    #[test]
    fn test_singlematch_dot() {
        let p = b".";
        assert!(singlematch(b'x', p, 0));
        assert!(singlematch(b' ', p, 0));
    }

    #[test]
    fn test_singlematch_class() {
        let p = b"%d";
        assert!(singlematch(b'5', p, 0));
        assert!(!singlematch(b'a', p, 0));
    }

    #[test]
    fn test_singlematch_inverted_class() {
        let p = b"%D";
        assert!(!singlematch(b'5', p, 0));
        assert!(singlematch(b'a', p, 0));
    }

    #[test]
    fn test_singlematch_set() {
        let p = b"[abc]";
        assert!(singlematch(b'a', p, 0));
        assert!(singlematch(b'c', p, 0));
        assert!(!singlematch(b'd', p, 0));
    }

    #[test]
    fn test_singlematch_negated_set() {
        let p = b"[^abc]";
        assert!(!singlematch(b'a', p, 0));
        assert!(singlematch(b'd', p, 0));
    }

    #[test]
    fn test_singlematch_range() {
        let p = b"[a-z]";
        assert!(singlematch(b'm', p, 0));
        assert!(!singlematch(b'M', p, 0));
    }

    #[test]
    fn test_singlematch_set_with_class() {
        let p = b"[%d_]";
        assert!(singlematch(b'5', p, 0));
        assert!(singlematch(b'_', p, 0));
        assert!(!singlematch(b'a', p, 0));
    }

    #[test]
    fn test_element_end() {
        let p = b"a";
        assert_eq!(element_end(p, 0), 1);

        let p = b"%d";
        assert_eq!(element_end(p, 0), 2);

        let p = b"[abc]";
        assert_eq!(element_end(p, 0), 5);

        let p = b"[^a-z%d]";
        assert_eq!(element_end(p, 0), 8);
    }

    #[test]
    fn test_set_bracket_first() {
        // ] as first char in set
        let p = b"[]abc]";
        assert!(singlematch(b']', p, 0));
        assert!(singlematch(b'a', p, 0));
        assert!(!singlematch(b'x', p, 0));
    }
}
