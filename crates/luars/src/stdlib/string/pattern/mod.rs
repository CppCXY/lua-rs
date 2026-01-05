// Lua string pattern matching implementation
// This implements Lua 5.4 pattern matching syntax
//
// Pattern syntax:
// - Character classes: ., %a, %c, %d, %g, %l, %p, %s, %u, %w, %x
// - Magic characters: ( ) % . + - * ? [ ] ^ $
// - Character sets: [set], [^set]
// - Repetitions: *, +, -, ?
// - Captures: (pattern)
// - Anchors: ^, $
// - Balanced: %b

mod matcher;
mod parser;

#[allow(unused)]
pub use matcher::{find, gsub, match_pattern, try_match};
#[allow(unused)]
pub use parser::{Pattern, parse_pattern};
