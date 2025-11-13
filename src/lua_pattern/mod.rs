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

pub use matcher::{find, gsub, match_pattern};
pub use parser::{parse_pattern, Pattern};
