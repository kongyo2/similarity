//! oxc AST → [`TreeNode`] conversion.
//!
//! The label set is intentionally rich: every distinct syntactic role gets a
//! distinct label so the structural comparison can tell apart constructs
//! that share a parent kind (`+=` vs `=`, `for-of` vs `while`, `obj.prop` vs
//! `obj[expr]`). Coarser labels were the dominant source of false-positive
//! matches between semantically different functions.
//!
//! # Refactor-equivalence canonicalization
//!
//! The function comparator wants two snippets a developer would call "the
//! same code written differently" to convert into the same tree. In
//! canonical mode the conversion rewrites common style alternatives onto
//! one shape:
//!
//!   * `` `a ${x}` `` → `"a " + x` (template literal as concatenation)
//!   * `x = x + y` → `x += y` (compound assignment), `i++` → `i += 1` in
//!     statement and `for`-update position
//!   * `return c ? a : b` → `if (c) { return a } return b`, and the
//!     assignment and declaration ternary forms accordingly
//!   * `xs.forEach((x) => {…})` → `for (const x of xs) {…}`
//!   * `return p.then((v) => {…})` → `const v = await p; …` (single level)
//!   * `for (; cond; )` → `while (cond)`
//!   * `switch` with jump-terminated cases → `if`/`else if` chain
//!   * `Object.assign({}, a, {k: v})` → `{ ...a, k: v }`
//!   * single-statement bodies of `if` and loops → block-wrapped bodies
//!
//! plus alpha-renaming of every symbol the fragment declares (see
//! [`rename`]) and the statement-list peepholes of [`peephole`].
//!
//! The literal (non-canonical) conversion mirrors what the user wrote. The
//! overlap detector reports source windows and needs exactly that.
//!
//! Which of the two a conversion is — and, for the canonical one, the
//! rename map of the fragment — travels through the conversion functions as
//! an explicit [`ConversionContext`]. It used to live in thread-local
//! state, which every nested or aborted parse had to restore by hand.

mod classes;
mod expressions;
mod jsx;
mod lowering;
mod patterns;
mod peephole;
mod predicates;
mod rename;
mod statements;

#[cfg(test)]
mod tests;

use oxc_ast::ast::{Expression, Program};
use oxc_span::Span;
use std::collections::HashMap;
use std::rc::Rc;

use crate::source::with_program;
use crate::tree::TreeNode;

/// Span → canonical name for every symbol bound inside a fragment.
type RenameMap = HashMap<(u32, u32), Rc<str>>;

/// What one AST → tree conversion needs to know besides the AST.
pub(crate) struct ConversionContext {
    /// Whether the refactor-equivalence canonicalization applies.
    canonical: bool,
    /// The fragment's alpha-renaming; empty for a literal conversion.
    renames: RenameMap,
}

impl ConversionContext {
    /// A conversion that mirrors the source.
    fn literal() -> Self {
        Self { canonical: false, renames: RenameMap::new() }
    }

    /// A canonicalizing conversion of `program`. Each `(alias, binding)`
    /// pair makes free references spelled `alias` resolve to the program's
    /// top-level declaration `binding`.
    fn canonical(program: &Program, aliases: &[(String, String)]) -> Self {
        Self { canonical: true, renames: rename::build_rename_map(program, aliases) }
    }

    /// The label of an identifier occurrence: the canonical `§N` name when
    /// the span belongs to a symbol the fragment declares, the original
    /// name otherwise (free identifiers, property names, and every
    /// identifier of a literal conversion).
    fn identifier_label(&self, span: Span, original: &str) -> String {
        self.renames
            .get(&(span.start, span.end))
            .map_or_else(|| original.to_string(), |name| name.to_string())
    }

    /// Whether the identifier at `span` resolves to a binding the fragment
    /// declares. Guards the rewrites of builtin globals (`Boolean`,
    /// `Object.assign`, `undefined`) against local shadowing.
    fn is_locally_bound(&self, span: Span) -> bool {
        self.renames.contains_key(&(span.start, span.end))
    }
}

/// Parse TypeScript code and convert it to a tree that mirrors the source.
///
/// # Errors
///
/// Returns an error if the code has syntax errors.
pub fn parse_and_convert_to_tree(
    filename: &str,
    source_text: &str,
) -> Result<Rc<TreeNode>, String> {
    with_program(filename, source_text, |program| {
        program_to_tree(program, &ConversionContext::literal())
    })
}

/// Variant of [`parse_and_convert_to_tree`] that applies the
/// refactor-equivalence canonicalization described in the module
/// documentation. Used by the function comparator so style-only rewrites
/// (template literals, `.then()` vs `await`, `forEach` vs `for-of`, …)
/// compare as equal trees.
///
/// # Errors
///
/// Returns an error if the code has syntax errors.
pub fn parse_and_convert_to_tree_canonical(
    filename: &str,
    source_text: &str,
) -> Result<Rc<TreeNode>, String> {
    parse_and_convert_to_tree_canonical_with_aliases(filename, source_text, &[])
}

/// [`parse_and_convert_to_tree_canonical`] for a fragment whose top-level
/// function is also reachable under other names: each `(alias, binding)`
/// pair makes free references spelled `alias` resolve to the fragment's
/// top-level declaration `binding`, so they alpha-rename together with it.
///
/// # Errors
///
/// Returns an error if the code has syntax errors.
pub fn parse_and_convert_to_tree_canonical_with_aliases(
    filename: &str,
    source_text: &str,
    aliases: &[(String, String)],
) -> Result<Rc<TreeNode>, String> {
    with_program(filename, source_text, |program| {
        let tree = program_to_tree(program, &ConversionContext::canonical(program, aliases));
        // Structural rewrites (push-loop → map, temp-return elimination,
        // index-loop → for-of) can drop symbols entirely, which would
        // leave gaps in the declaration-ordered `§N` numbering and make
        // otherwise-identical trees differ by ordinal. Renumber by first
        // occurrence in the FINAL tree so the numbering only depends on
        // the shape being compared.
        rename::relabel_canonical_ordinals(&tree)
    })
}

fn program_to_tree(program: &Program, cx: &ConversionContext) -> Rc<TreeNode> {
    let mut root = TreeNode::new("Program", "Program");
    for stmt in &program.body {
        statements::append_statement_to_list(&mut root, stmt, cx);
    }
    Rc::new(root)
}

/// A node without children (yet) and without a source position.
fn make_node(label: &str, kind: &'static str) -> TreeNode {
    TreeNode::new(label, kind)
}

/// Record the source range `node` was converted from, unless it already
/// has one. The overlap detector reports the lines of the subtrees it
/// matches, so a node that stands for a piece of source knows which piece.
fn stamp(mut node: Rc<TreeNode>, span: Span) -> Rc<TreeNode> {
    if !node.has_source_span() {
        // Freshly converted nodes are not shared yet.
        if let Some(unique) = Rc::get_mut(&mut node) {
            unique.set_source_span(span.start, span.end);
        }
    }
    node
}

fn leaf(label: &str, kind: &'static str) -> Rc<TreeNode> {
    Rc::new(make_node(label, kind))
}

fn strip_parentheses<'a, 'b>(expr: &'b Expression<'a>) -> &'b Expression<'a> {
    let mut current = expr;
    while let Expression::ParenthesizedExpression(paren) = current {
        current = &paren.expression;
    }
    current
}
