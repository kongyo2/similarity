//! The source fragments a function is parsed from for comparison.
//!
//! A function is compared by parsing a small standalone program built from
//! its text: the *normalized* fragment declares every function form the
//! same way, so two functions whose bodies and parameter lists agree
//! convert to the same tree whether they were written as a declaration, an
//! arrow, a method or an object member.

use std::rc::Rc;

use crate::function_extractor::{
    is_plain_identifier, FunctionDefinition, FunctionType, MethodKind, SourceForm,
};
use crate::parser::{
    parse_and_convert_to_tree_canonical, parse_and_convert_to_tree_canonical_with_aliases,
};
use crate::tree::TreeNode;

/// The comparison fragment of a function and the top-level binding it
/// declares, which is what a self-alias resolves to.
pub(crate) struct NormalizedFragment {
    pub(crate) text: String,
    pub(crate) binding: String,
}

/// Name of the synthetic class that wraps method fragments which cannot be
/// parsed on their own (see [`build_normalized_fragment`]).
pub(crate) const CLASS_WRAPPER_NAME: &str = "__C__";

/// Name the standalone `function` fragment of a method is declared under
/// (or, when the method's source spells it, the same name with a suffix;
/// see [`unused_synthetic_name`]). A member key is not a binding inside the
/// member's body: in `visit(node) { return visit(node.left); }` the call
/// goes to an outer `visit`, and recursion goes through `this.visit`.
/// Under the key, such calls would resolve to the fragment's own
/// declaration, and a method delegating to a same-named helper would
/// compare as identical to a recursive function. Under a placeholder, a
/// reference spelled like the key stays a free identifier, as in the
/// class-wrapped shapes, in arrow-valued object members and in class
/// mode's method fingerprints. The declaration name alpha-renames like any
/// other binding, so the comparison does not depend on it. A method's key
/// is a binding in neither fragment shape, so only the own name of a
/// function expression held by the member (`visit: function walk(node)
/// {…}`) resolves to the function.
pub(crate) const METHOD_FRAGMENT_NAME: &str = "__m__";

/// Name a function that has no usable name of its own is declared under.
const ANONYMOUS_FRAGMENT_NAME: &str = "__sim__";

/// Build a normalization-friendly fragment for a function. The goal is that
/// two functions whose bodies and parameter lists agree end up producing
/// identical fragments regardless of whether they were declared as
/// `function foo() {...}`, `const foo = () => {...}`, or
/// `const foo = (x) => x + 1`. Without this layer the wrapping shape
/// (FunctionDeclaration vs ExpressionStatement→ArrowFunctionExpression)
/// dominated the structural distance on short bodies.
pub(crate) fn build_normalized_fragment(
    func: &FunctionDefinition,
    source: &str,
) -> NormalizedFragment {
    let text_of = |span| crate::source::slice(source, span).unwrap_or("");

    let params_text = {
        let raw = text_of(func.params_span).trim();
        if raw.starts_with('(') && raw.ends_with(')') {
            raw.to_string()
        } else if raw.is_empty() {
            "()".to_string()
        } else {
            // Single un-parenthesised arrow parameter, e.g. `x => ...`.
            format!("({raw})")
        }
    };

    let raw_body_text = text_of(func.body_span);
    // Wrap a single-expression arrow body in an explicit `return` so it
    // ends up shaped like a block-bodied function. Without this an
    // `(x) => x + 1` would parse as a top-level `ExpressionStatement →
    // ArrowFunctionExpression`, adding a structural wrapper that an
    // equivalent `function f(x) { return x + 1; }` would not have. Arrow
    // class fields (`handle = (event) => event.id`) take the same wrapping
    // on the method path below.
    let body_text: String = if func.is_arrow_expression {
        format!("{{ return {raw_body_text}; }}")
    } else {
        raw_body_text.to_string()
    };
    // Every source text the fragment splices in; a synthetic name must not
    // occur in any of it (see `unused_synthetic_name`).
    let spliced = format!("{params_text} {body_text} {}", func.display_name);

    // `async function*`, `function*`, `async function`, `function`: the
    // call contract survives the rewrite, so two functions that differ
    // only in `async`-ness (a different runtime return type) or in being a
    // generator don't compare as identical.
    let function_keyword = || {
        let mut keyword = String::new();
        if func.is_async {
            keyword.push_str("async ");
        }
        keyword.push_str("function");
        if func.is_generator {
            keyword.push('*');
        }
        keyword
    };

    match func.function_type {
        FunctionType::Method | FunctionType::Constructor => {
            // Plain instance methods normalize to a standalone `function`
            // fragment so "extract method to function" refactors (and the
            // reverse) compare by body instead of by declaration shape —
            // the class wrapper used to dominate the structural distance
            // for method-vs-function pairs. The fragment is declared under
            // a placeholder, not the key, which binds nothing inside the
            // body (see `METHOD_FRAGMENT_NAME`). Static methods, accessors,
            // constructors and non-identifier keys keep the class wrapper:
            // their runtime contracts are tied to the class shape and the
            // existing relative scoring for those forms is calibrated
            // against it.
            if method_normalizes_to_function(func) {
                let name_text = unused_synthetic_name(METHOD_FRAGMENT_NAME, &spliced);
                return NormalizedFragment {
                    text: format!("{} {name_text}{params_text} {body_text}", function_keyword()),
                    binding: name_text,
                };
            }
            // Remaining method shapes can't be parsed in isolation, so keep
            // the synthetic class wrapper. Preserve `static`, getter/setter,
            // `async` and generator modifiers — runtime-distinct methods
            // like `static foo` vs `foo` or `get foo` vs `foo` must not
            // collapse onto the same tree even when their bodies agree.
            let mut prefix = String::new();
            if func.is_static {
                prefix.push_str("static ");
            }
            match func.method_kind {
                MethodKind::Getter => prefix.push_str("get "),
                MethodKind::Setter => prefix.push_str("set "),
                MethodKind::Normal => {
                    if func.is_async {
                        prefix.push_str("async ");
                    }
                    if func.is_generator {
                        prefix.push('*');
                    }
                }
            }
            // Methods use `display_name` directly: it already carries the
            // exact source text of the key (`"alpha"`, `#load`,
            // `[Symbol.iterator]`, …), all of which are syntactically valid
            // inside a class body, so no sanitization is needed. The
            // sanitizer used for top-level functions would otherwise reject
            // these forms and collapse distinct methods onto one name.
            let method_name_text = if func.display_name.is_empty() {
                ANONYMOUS_FRAGMENT_NAME
            } else {
                func.display_name.as_str()
            };
            let wrapper = unused_synthetic_name(CLASS_WRAPPER_NAME, &spliced);
            NormalizedFragment {
                text: format!(
                    "class {wrapper} {{ {prefix}{method_name_text}{params_text} {body_text} }}"
                ),
                binding: wrapper,
            }
        }
        FunctionType::Function | FunctionType::Arrow => {
            // Top-level functions and arrow declarations always bind to
            // an ordinary identifier, so the sanitizer is enough — there
            // is no private-method / string-literal-key path here.
            // `display_name` is the name the body is bound under: the
            // declaration name, or a named function expression's own
            // name, which its recursive calls refer to. A name the
            // sanitizer made up binds nothing in the source and is kept out
            // of its references like the other synthetic names. (Arrows can
            // be async but never generators, so the generator marker only
            // ever comes from the `function` path.)
            let mut name_text = sanitize_function_name(&func.display_name);
            if name_text != func.display_name {
                name_text = unused_synthetic_name(&name_text, &spliced);
            }
            NormalizedFragment {
                text: format!("{} {name_text}{params_text} {body_text}", function_keyword()),
                binding: name_text,
            }
        }
    }
}

/// `base`, or `base` with the smallest numeric suffix, whichever `text`
/// does not contain. Fragments declare synthetic names (the method
/// placeholder, the class wrapper, a sanitized function name) around
/// source text, and a source reference spelled like one of them would
/// resolve to the synthetic declaration: a call to an outer `__m__`
/// would read as recursion. A name the source never spells captures
/// nothing, and any spelling works because the declaration alpha-renames
/// like every other binding.
pub(crate) fn unused_synthetic_name(base: &str, text: &str) -> String {
    let mut name = base.to_string();
    let mut suffix = 0u32;
    while text.contains(name.as_str()) {
        suffix += 1;
        name = format!("{base}{suffix}");
    }
    name
}

/// Whether a method normalizes to a standalone `function` fragment: plain
/// instance methods with an identifier key. The other shapes keep the
/// synthetic class wrapper — static methods, accessors, constructors, and
/// non-identifier keys (string literals, computed keys, `#private` names),
/// whose key shape stays visible to the comparison.
fn method_normalizes_to_function(func: &FunctionDefinition) -> bool {
    func.function_type == FunctionType::Method
        && func.method_kind == MethodKind::Normal
        && !func.is_static
        && is_plain_identifier(&func.display_name)
}

fn sanitize_function_name(name: &str) -> String {
    if !is_plain_identifier(name) {
        ANONYMOUS_FRAGMENT_NAME.to_string()
    } else if is_reserved_function_name(name) {
        // `export default function () {…}` is recorded as `default`,
        // which cannot head a `function` declaration.
        format!("__f_{name}")
    } else {
        name.to_string()
    }
}

/// Words that cannot appear as a `function <name>` in a strict-mode
/// module even though they are perfectly valid method keys (`delete(key)`
/// on a cache class being the canonical example).
fn is_reserved_function_name(name: &str) -> bool {
    matches!(
        name,
        "await"
            | "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "implements"
            | "import"
            | "in"
            | "instanceof"
            | "interface"
            | "let"
            | "new"
            | "null"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "static"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
    )
}

/// A declaration's own source text in a form that parses by itself.
pub(crate) struct StandaloneSource {
    pub(crate) text: String,
    /// Byte offset of the declaration's text within `text`.
    pub(crate) offset: u32,
}

/// The declaration's own source text, wrapped — on one line, so the line
/// breaks of the text are those of the declaration — in whatever makes it
/// parse by itself: nothing for a `function` statement, parentheses for an
/// expression, a synthetic class or object literal for a member.
pub(crate) fn standalone_source(
    func: &FunctionDefinition,
    source: &str,
) -> Option<StandaloneSource> {
    let text = crate::source::slice(source, func.span)?;
    let (prefix, suffix) = match func.form {
        SourceForm::Statement => (String::new(), ""),
        SourceForm::Expression => ("(".to_string(), ")"),
        SourceForm::ClassMember => {
            (format!("class {} {{ ", unused_synthetic_name(CLASS_WRAPPER_NAME, text)), " }")
        }
        SourceForm::ObjectMember => ("({ ".to_string(), " })"),
    };
    Some(StandaloneSource {
        offset: u32::try_from(prefix.len()).ok()?,
        text: format!("{prefix}{text}{suffix}"),
    })
}

/// Parse a function for structural comparison, using
/// [`build_normalized_fragment`] so arrow vs regular vs method declarations
/// all end up shape-equivalent when their bodies agree, and the canonical
/// parse so style-only rewrites (template literals, `.then` vs `await`,
/// `forEach` vs `for-of`, …) compare as equal trees.
///
/// # Errors
///
/// Returns the parser's messages when neither the normalized fragment nor
/// the declaration's own text parses.
pub(crate) fn parse_function_for_comparison(
    filename: &str,
    func: &FunctionDefinition,
    source: &str,
) -> Result<Rc<TreeNode>, String> {
    let fragment = build_normalized_fragment(func, source);
    let parsed = match &func.self_alias {
        // The fragment declares the function under one binding; the other
        // name it is reachable under resolves to the same declaration.
        Some(alias) => parse_and_convert_to_tree_canonical_with_aliases(
            filename,
            &fragment.text,
            &[(alias.clone(), fragment.binding)],
        ),
        None => parse_and_convert_to_tree_canonical(filename, &fragment.text),
    };
    parsed.or_else(|error| {
        // Fall back to the declaration as written, so an edge case the
        // normalizer breaks still compares.
        let standalone = standalone_source(func, source).ok_or(error)?;
        parse_and_convert_to_tree_canonical(filename, &standalone.text)
    })
}
