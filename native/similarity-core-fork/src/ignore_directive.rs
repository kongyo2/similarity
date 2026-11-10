//! `// similarity-ignore` — the comment that keeps a declaration out of the
//! report.

use crate::source::LineIndex;

const DIRECTIVE: &str = "similarity-ignore";

/// Whether the comments directly above 1-based line `line` carry the
/// `similarity-ignore` directive.
///
/// The search walks upward from the line above `line` over blank lines,
/// `//` comments and block comments (JSDoc included) and stops at the
/// first line of code, so a directive only ever applies to the declaration
/// right below it.
pub(crate) fn has_ignore_directive_above(source_text: &str, lines: &LineIndex, line: u32) -> bool {
    let mut current = line;
    let mut in_block_comment = false;

    while current > 1 {
        current -= 1;
        // `None`: `line` lies past the end of the text.
        let Some(text) = lines.line_text(source_text, current) else {
            return false;
        };
        let text = text.trim();

        if text.is_empty() {
            continue;
        }

        if in_block_comment {
            if text.contains(DIRECTIVE) {
                return true;
            }
            if text.contains("/*") {
                in_block_comment = false;
            }
            continue;
        }

        if text.starts_with("//") {
            if text.contains(DIRECTIVE) {
                return true;
            }
            continue;
        }

        if text.starts_with("/*") {
            return text.contains(DIRECTIVE);
        }

        if text.ends_with("*/") || text.starts_with("*/") {
            if text.contains(DIRECTIVE) {
                return true;
            }
            if !text.contains("/*") {
                in_block_comment = true;
            }
            continue;
        }

        if text.starts_with('*') {
            if text.contains(DIRECTIVE) {
                return true;
            }
            continue;
        }

        break;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directive_above(source: &str, line: u32) -> bool {
        has_ignore_directive_above(source, &LineIndex::new(source), line)
    }

    #[test]
    fn detects_single_line_directive() {
        let source = r#"
// similarity-ignore
function test() {}
"#;

        assert!(directive_above(source, 3));
    }

    #[test]
    fn detects_block_comment_directive() {
        let source = r#"
/**
 * similarity-ignore
 */
function test() {}
"#;

        assert!(directive_above(source, 5));
    }

    #[test]
    fn stops_at_non_comment_code() {
        let source = r#"
const marker = true;
// similarity-ignore

function test() {}
"#;

        assert!(directive_above(source, 5));
        assert!(!directive_above(source, 2));
    }

    #[test]
    fn a_directive_applies_to_the_next_declaration_only() {
        let source = "// similarity-ignore\nfunction first() {}\nfunction second() {}\n";
        assert!(directive_above(source, 2));
        assert!(!directive_above(source, 3));
        // The first line has nothing above it, and lines past the end have
        // no declaration on them.
        assert!(!directive_above(source, 1));
        assert!(!directive_above(source, 40));
    }

    #[test]
    fn other_comments_between_the_directive_and_the_declaration_are_skipped() {
        let source =
            "// similarity-ignore\n/**\n * Docs.\n */\n// note\nfunction documented() {}\n";
        assert!(directive_above(source, 6));
        let source = "/* similarity-ignore */\r\nfunction windows() {}\r\n";
        assert!(directive_above(source, 2));
    }
}
