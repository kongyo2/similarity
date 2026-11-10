//! Source-text plumbing shared by every extractor: parsing, byte-offset →
//! line lookup, span slicing, and the nesting guard that keeps pathological
//! input from overflowing the stack.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    AssignmentTarget, BindingPattern, Expression, JSXElement, JSXFragment, JSXMemberExpression,
    Program, Statement, TSModuleDeclaration, TSType, TSTypeName,
};
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::{SourceType, Span};

/// Deepest bracket nesting (`(`, `[`, `{`) a file may have before it is
/// skipped. The parser recurses once per level and has no depth limit of
/// its own; under WebAssembly its stack gives out a little past 900 levels,
/// and a stack overflow there takes the whole run down, every other file
/// included. Hand-written code nests a few dozen levels at most.
pub const MAX_BRACKET_DEPTH: usize = 256;

/// Deepest expression / statement / type nesting a parsed file may have
/// before it is skipped. Operator and call chains are parsed with a loop,
/// so `a + a + … + a` or `b.x().x()….x()` of any length parses, into a tree
/// as deep as the chain is long; the conversion, comparison and hashing
/// passes all recurse over that tree. Chains of several hundred links only
/// occur in generated code.
pub const MAX_AST_DEPTH: usize = 512;

/// How a source file is parsed, from its path: `.ts`, `.tsx`, `.mts`,
/// `.cts`, `.d.ts`, or — for anything else — TSX, the superset.
#[must_use]
pub fn source_type_for(path: &str) -> SourceType {
    SourceType::from_path(path).unwrap_or_else(|_| SourceType::tsx())
}

/// Parse `source_text` and hand the program to `visit`.
///
/// # Errors
///
/// Returns the parser's messages when the text has syntax errors.
pub fn with_program<T>(
    path: &str,
    source_text: &str,
    visit: impl FnOnce(&Program<'_>) -> T,
) -> Result<T, String> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source_text, source_type_for(path)).parse();
    if !parsed.errors.is_empty() {
        let messages: Vec<String> =
            parsed.errors.iter().map(|error| error.message.to_string()).collect();
        return Err(format!("Parse errors: {}", messages.join(", ")));
    }
    Ok(visit(&parsed.program))
}

/// [`with_program`] for a whole source file: the text is checked against
/// [`MAX_BRACKET_DEPTH`] before it is parsed and the program against
/// [`MAX_AST_DEPTH`] after, so everything `visit` (and the passes behind
/// it) recurses over is of bounded depth.
///
/// # Errors
///
/// Returns the reason the file cannot be analyzed: syntax errors, or
/// nesting beyond either limit.
pub fn with_checked_program<T>(
    path: &str,
    source_text: &str,
    visit: impl FnOnce(&Program<'_>) -> T,
) -> Result<T, String> {
    if bracket_depth_exceeds(source_text, MAX_BRACKET_DEPTH) {
        return Err(format!("brackets are nested more than {MAX_BRACKET_DEPTH} levels deep"));
    }
    with_program(path, source_text, |program| {
        if ast_depth_exceeds(program, MAX_AST_DEPTH) {
            return Err(format!("code is nested more than {MAX_AST_DEPTH} levels deep"));
        }
        Ok(visit(program))
    })?
}

/// Whether a source file can be analyzed (see [`with_checked_program`]).
///
/// # Errors
///
/// Returns the reason it cannot.
pub fn check_source(path: &str, source_text: &str) -> Result<(), String> {
    with_checked_program(path, source_text, |_| ())
}

/// The text a span covers, or `None` for an empty or out-of-range span.
#[must_use]
pub fn slice(source_text: &str, span: Span) -> Option<&str> {
    let (start, end) = (span.start as usize, span.end as usize);
    if start < end {
        source_text.get(start..end)
    } else {
        None
    }
}

/// Byte-offset → 1-based line lookup for one source text.
#[derive(Debug, Clone)]
pub struct LineIndex {
    /// Byte offset at which each line starts; never empty (line 1 starts
    /// at offset 0).
    line_starts: Vec<u32>,
}

impl LineIndex {
    #[must_use]
    pub fn new(text: &str) -> Self {
        let mut line_starts = Vec::with_capacity(text.len() / 24 + 1);
        line_starts.push(0);
        for (position, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                #[allow(clippy::cast_possible_truncation)]
                line_starts.push(position as u32 + 1);
            }
        }
        Self { line_starts }
    }

    /// The 1-based line a byte offset falls on.
    #[must_use]
    pub fn line_of(&self, byte_offset: u32) -> u32 {
        let index = self.line_starts.partition_point(|&start| start <= byte_offset);
        #[allow(clippy::cast_possible_truncation)]
        let line = index.max(1) as u32;
        line
    }

    /// Number of lines in the text (a trailing newline starts a last,
    /// empty line).
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// The text of a 1-based line, without its line terminator.
    #[must_use]
    pub fn line_text<'a>(&self, text: &'a str, line: u32) -> Option<&'a str> {
        let index = (line as usize).checked_sub(1)?;
        let start = *self.line_starts.get(index)? as usize;
        let end = self.line_starts.get(index + 1).map_or(text.len(), |&next| next as usize);
        let raw = text.get(start..end)?;
        let raw = raw.strip_suffix('\n').unwrap_or(raw);
        Some(raw.strip_suffix('\r').unwrap_or(raw))
    }
}

/// Whether `(`, `[` and `{` nest more than `limit` levels deep anywhere in
/// `source_text`.
///
/// This runs before the parser, so it works on the raw text: comments,
/// string literals and template-literal text are skipped, `${…}` inside a
/// template is code again, and a backslash escapes the character after it
/// (which covers `\(` in a regular expression). What it cannot tell apart —
/// a bracket inside a regular expression's character class, or in JSX
/// text — only moves the count by one, and a closing bracket never takes
/// it below zero.
fn bracket_depth_exceeds(source_text: &str, limit: usize) -> bool {
    let bytes = source_text.as_bytes();
    let mut depth = 0usize;
    // Depth at which each enclosing `${` was opened; the `}` that returns
    // to that depth ends the interpolation and resumes the template text.
    let mut interpolations: Vec<usize> = Vec::new();
    let mut index = 0;

    // Skip template text starting at `start` (just past a backtick or the
    // `}` of an interpolation). Returns the index after the closing
    // backtick, or after a `${`, with `true` in the latter case.
    let skip_template_text = |start: usize| -> (usize, bool) {
        let mut cursor = start;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'\\' => cursor += 2,
                b'`' => return (cursor + 1, false),
                b'$' if bytes.get(cursor + 1) == Some(&b'{') => return (cursor + 2, true),
                _ => cursor += 1,
            }
        }
        (bytes.len(), false)
    };

    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index < bytes.len()
                    && !(bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/'))
                {
                    index += 1;
                }
                index += 2;
            }
            quote @ (b'"' | b'\'') => {
                index += 1;
                // A string literal ends at its quote or, unterminated, at
                // the end of the line.
                while index < bytes.len() && bytes[index] != quote && bytes[index] != b'\n' {
                    index += if bytes[index] == b'\\' { 2 } else { 1 };
                }
                index += 1;
            }
            b'`' => {
                let (next, interpolation) = skip_template_text(index + 1);
                index = next;
                if interpolation {
                    interpolations.push(depth);
                    depth += 1;
                    if depth > limit {
                        return true;
                    }
                }
            }
            b'\\' => index += 2,
            b'(' | b'[' | b'{' => {
                depth += 1;
                if depth > limit {
                    return true;
                }
                index += 1;
            }
            b')' | b']' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                index += 1;
                if interpolations.last() == Some(&depth) {
                    interpolations.pop();
                    // The next interpolation of the same template opens at
                    // the depth this one closed at.
                    let (next, interpolation) = skip_template_text(index);
                    index = next;
                    if interpolation {
                        interpolations.push(depth);
                        depth += 1;
                    }
                }
            }
            _ => index += 1,
        }
    }
    false
}

/// Whether expressions, statements, types or patterns nest more than
/// `limit` levels deep anywhere in `program`.
fn ast_depth_exceeds(program: &Program<'_>, limit: usize) -> bool {
    let mut probe = NestingProbe { depth: 0, limit, exceeded: false };
    probe.visit_program(program);
    probe.exceeded
}

/// Depth-limited walk: every recursive construct of the AST is entered
/// through one of the methods below, and none of them descends past the
/// limit, so the walk itself cannot overflow the stack on the input it is
/// there to reject.
struct NestingProbe {
    depth: usize,
    limit: usize,
    exceeded: bool,
}

impl NestingProbe {
    fn descend(&mut self, walk: impl FnOnce(&mut Self)) {
        if self.exceeded {
            return;
        }
        if self.depth >= self.limit {
            self.exceeded = true;
            return;
        }
        self.depth += 1;
        walk(self);
        self.depth -= 1;
    }
}

impl<'a> Visit<'a> for NestingProbe {
    fn visit_expression(&mut self, it: &Expression<'a>) {
        self.descend(|probe| walk::walk_expression(probe, it));
    }

    fn visit_statement(&mut self, it: &Statement<'a>) {
        self.descend(|probe| walk::walk_statement(probe, it));
    }

    fn visit_ts_type(&mut self, it: &TSType<'a>) {
        self.descend(|probe| walk::walk_ts_type(probe, it));
    }

    fn visit_ts_type_name(&mut self, it: &TSTypeName<'a>) {
        self.descend(|probe| walk::walk_ts_type_name(probe, it));
    }

    fn visit_ts_module_declaration(&mut self, it: &TSModuleDeclaration<'a>) {
        self.descend(|probe| walk::walk_ts_module_declaration(probe, it));
    }

    fn visit_binding_pattern(&mut self, it: &BindingPattern<'a>) {
        self.descend(|probe| walk::walk_binding_pattern(probe, it));
    }

    fn visit_assignment_target(&mut self, it: &AssignmentTarget<'a>) {
        self.descend(|probe| walk::walk_assignment_target(probe, it));
    }

    fn visit_jsx_element(&mut self, it: &JSXElement<'a>) {
        self.descend(|probe| walk::walk_jsx_element(probe, it));
    }

    fn visit_jsx_fragment(&mut self, it: &JSXFragment<'a>) {
        self.descend(|probe| walk::walk_jsx_fragment(probe, it));
    }

    fn visit_jsx_member_expression(&mut self, it: &JSXMemberExpression<'a>) {
        self.descend(|probe| walk::walk_jsx_member_expression(probe, it));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_maps_offsets_to_lines() {
        let text = "first\nsecond\r\n\nfourth";
        let index = LineIndex::new(text);
        assert_eq!(index.line_count(), 4);
        assert_eq!(index.line_of(0), 1);
        assert_eq!(index.line_of(5), 1, "the newline belongs to the line it ends");
        assert_eq!(index.line_of(6), 2);
        assert_eq!(index.line_of(14), 3);
        assert_eq!(index.line_of(15), 4);
        assert_eq!(index.line_of(1_000), 4, "offsets past the end stay on the last line");
        assert_eq!(index.line_text(text, 1), Some("first"));
        assert_eq!(index.line_text(text, 2), Some("second"), "CRLF is stripped");
        assert_eq!(index.line_text(text, 3), Some(""));
        assert_eq!(index.line_text(text, 4), Some("fourth"));
        assert_eq!(index.line_text(text, 0), None);
        assert_eq!(index.line_text(text, 5), None);
    }

    #[test]
    fn slice_rejects_empty_and_out_of_range_spans() {
        let text = "héllo";
        assert_eq!(slice(text, Span::new(0, 1)), Some("h"));
        assert_eq!(slice(text, Span::new(2, 2)), None);
        assert_eq!(slice(text, Span::new(3, 99)), None);
        // An offset inside a multi-byte character is not a boundary.
        assert_eq!(slice(text, Span::new(0, 2)), None);
    }

    #[test]
    fn syntax_errors_are_reported() {
        let error = check_source("broken.ts", "export class {{{ nope").unwrap_err();
        assert!(error.starts_with("Parse errors: "), "{error}");
        assert!(check_source("fine.ts", "export const answer = 42;").is_ok());
    }

    #[test]
    fn bracket_depth_counts_code_only() {
        assert!(!bracket_depth_exceeds("f(a[b]{c})", 2));
        assert!(bracket_depth_exceeds("f(a[b{c}])", 2));
        // Brackets in comments, strings and template text do not count.
        assert!(!bracket_depth_exceeds("// (((((\n/* [[[[[ */ '(((((' \"[[[[[\" `{{{{{`", 0));
        // An escaped quote does not end the string.
        assert!(!bracket_depth_exceeds(r#"'a\'(((' + "b\"((("#, 0));
        // `${…}` is code again, and may itself hold a template.
        assert!(bracket_depth_exceeds("`a${f(g(h()))}b`", 3));
        assert!(!bracket_depth_exceeds("`a${f(g(h()))}b`", 4));
        assert!(!bracket_depth_exceeds("`a${`b${c}((((`}((((` + [0]", 2));
        // Templates nested in interpolations nest like brackets, with or
        // without a bracket of their own.
        let templates = format!("{}a{}", "`x${".repeat(5), "}`".repeat(5));
        assert!(bracket_depth_exceeds(&templates, 4));
        assert!(!bracket_depth_exceeds(&templates, 5));
        // Interpolations side by side do not.
        assert!(!bracket_depth_exceeds("`${a}${b}${c}${d}`", 1));
        // An escaped bracket in a regular expression is skipped; closing
        // brackets never take the count below zero.
        assert!(!bracket_depth_exceeds(r"const open = /\(\[\{/; )))) f(x)", 1));
    }

    #[test]
    fn deep_bracket_nesting_is_rejected_before_parsing() {
        let nested = format!(
            "export const value = {}1{};\n",
            "[".repeat(MAX_BRACKET_DEPTH + 1),
            "]".repeat(MAX_BRACKET_DEPTH + 1)
        );
        let error = check_source("nested.ts", &nested).unwrap_err();
        assert!(error.contains("brackets are nested"), "{error}");

        let allowed = format!(
            "export const value = {}1{};\n",
            "[".repeat(MAX_BRACKET_DEPTH / 2),
            "]".repeat(MAX_BRACKET_DEPTH / 2)
        );
        assert!(check_source("nested.ts", &allowed).is_ok());
    }

    #[test]
    fn long_operator_and_call_chains_are_rejected_after_parsing() {
        // No brackets at all: the parser builds these with a loop, into a
        // tree as deep as the chain is long.
        let terms = vec!["a"; MAX_AST_DEPTH + 2].join(" + ");
        let error =
            check_source("sum.ts", &format!("export const total = {terms};\n")).unwrap_err();
        assert!(error.contains("code is nested"), "{error}");

        let calls = ".next()".repeat(MAX_AST_DEPTH / 2 + 2);
        let error =
            check_source("chain.ts", &format!("export const last = b{calls};\n")).unwrap_err();
        assert!(error.contains("code is nested"), "{error}");

        // The chains real code is made of pass.
        let terms = vec!["a"; 100].join(" + ");
        assert!(check_source("sum.ts", &format!("export const total = {terms};\n")).is_ok());
        let calls = ".option('x', 1)".repeat(60);
        assert!(check_source("chain.ts", &format!("export const cli = yargs{calls};\n")).is_ok());
    }
}
