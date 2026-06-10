use oxc_ast::ast::*;
use oxc_span::{GetSpan, Span};

use crate::ignore_directive::has_similarity_ignore_directive;
use crate::parser::{parse_and_convert_to_tree, parse_and_convert_to_tree_canonical};
use crate::tsed::{calculate_tsed, TSEDOptions};

type CrossFileSimilarityResult = Vec<(String, SimilarityResult, String)>;

#[derive(Debug, Clone)]
pub struct SimilarityResult {
    pub func1: FunctionDefinition,
    pub func2: FunctionDefinition,
    pub similarity: f64,
    pub impact: u32, // Total lines that could be removed
}

impl SimilarityResult {
    pub fn new(func1: FunctionDefinition, func2: FunctionDefinition, similarity: f64) -> Self {
        // Impact is the smaller function's line count (since we'd remove the duplicate)
        let impact = func1.line_count().min(func2.line_count());
        SimilarityResult { func1, func2, similarity, impact }
    }
}

#[derive(Debug, Clone)]
pub struct FunctionDefinition {
    pub name: String,
    pub function_type: FunctionType,
    pub parameters: Vec<String>,
    /// Span of the whole function (header + body). Misleadingly named — kept
    /// for backwards compatibility with the surrounding code that uses it to
    /// detect nested-function containment.
    pub body_span: Span,
    /// Span of the formal parameter list (`(x, y)` or, for arrow functions
    /// with a single un-parenthesised parameter, just `x`). Used together
    /// with `body_block_span` to assemble a normalization wrapper that
    /// treats arrow functions and regular function declarations
    /// equivalently.
    pub params_span: Span,
    /// Span of just the function body (the `{ ... }` block, or the
    /// expression for expression-bodied arrow functions). Used to extract
    /// a normalized comparison fragment so a `function foo` and a
    /// `const foo = () => ...` with identical bodies look structurally
    /// equivalent to the comparator.
    pub body_block_span: Span,
    /// True when this is an arrow function whose body is a single
    /// expression (e.g. `x => x + 1`). The normalization wrapper has to
    /// add an explicit `return` in that case.
    pub is_arrow_expression: bool,
    /// True when the declaration was `async function ...` or
    /// `async (...) => ...`. Preserved through normalization so an
    /// `async` and a sync function with otherwise-identical bodies
    /// don't collapse into the same tree (their runtime contracts
    /// differ — `Promise<T>` vs `T`).
    pub is_async: bool,
    /// True when the declaration was `function* ...` or, for methods,
    /// `*foo() ...`. Generators differ structurally from non-generator
    /// counterparts and have to survive normalization.
    pub is_generator: bool,
    /// True only for class methods declared `static`. Static and
    /// instance methods with identical bodies are runtime-distinct,
    /// so the normalized fragment carries this prefix forward.
    pub is_static: bool,
    /// Method kind: `Normal`, `Getter`, or `Setter`. For functions,
    /// arrows, and constructors this is always `Normal`.
    pub method_kind: MethodKind,
    /// The exact source text of the method key (`"alpha"`, `#load`,
    /// `[Symbol.iterator]`, …) or, for functions and arrows, the
    /// declaration name. Used in the normalization wrapper so a
    /// `static "alpha"()` and a `static "beta"()` do not collapse onto
    /// the same `anonymous` placeholder the simple `name` field uses.
    /// Falls back to `name` when the underlying span can't be recovered.
    pub display_name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub class_name: Option<String>,
    pub parent_function: Option<String>,
    pub node_count: Option<u32>,
    pub has_ignore_directive: bool,
}

impl FunctionDefinition {
    pub fn line_count(&self) -> u32 {
        self.end_line - self.start_line + 1
    }

    /// Check if this function is a parent or child of another function
    pub fn is_parent_child_relationship(&self, other: &FunctionDefinition) -> bool {
        // Check if 'other' is inside 'self' (self is parent of other)
        let other_inside_self = self.start_line <= other.start_line
            && self.end_line >= other.end_line
            && self.body_span.start < other.body_span.start
            && self.body_span.end > other.body_span.end;

        // Check if 'self' is inside 'other' (other is parent of self)
        let self_inside_other = other.start_line <= self.start_line
            && other.end_line >= self.end_line
            && other.body_span.start < self.body_span.start
            && other.body_span.end > self.body_span.end;

        other_inside_self || self_inside_other
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FunctionType {
    Function,
    Method,
    Arrow,
    Constructor,
}

/// Class-method kind preserved across normalization so a `get` accessor
/// and a regular method, or `set` vs `get`, never collapse into the same
/// tree. Non-method functions always carry `Normal`.
#[derive(Debug, Clone, PartialEq)]
pub enum MethodKind {
    Normal,
    Getter,
    Setter,
}

/// Best-effort source text for a method key. For static identifiers and
/// private identifiers we already have a clean string form on the AST node,
/// but for string literals, numeric literals, and computed expressions the
/// original key text (e.g. `"alpha"`, `[Symbol.iterator]`) is the only
/// signal that survives into the normalization wrapper. Returns `None`
/// when the span is empty or out of bounds so callers fall back to the
/// existing name.
fn method_key_source_text(key: &PropertyKey, source: &str) -> Option<String> {
    let span = key.span();
    let s = span.start as usize;
    let e = span.end as usize;
    if s < e && e <= source.len() {
        Some(source[s..e].to_string())
    } else {
        None
    }
}

/// Extract all functions from TypeScript/JavaScript code
pub fn extract_functions(
    filename: &str,
    source_text: &str,
) -> Result<Vec<FunctionDefinition>, String> {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    let allocator = Allocator::default();
    let source_type = SourceType::from_path(filename).unwrap_or(SourceType::tsx());
    let ret = Parser::new(&allocator, source_text, source_type).parse();

    if !ret.errors.is_empty() {
        // Create a more readable error message
        let error_messages: Vec<String> =
            ret.errors.iter().map(|e| e.message.to_string()).collect();
        return Err(format!("Parse errors: {}", error_messages.join(", ")));
    }

    let mut functions = Vec::new();
    let mut context = ExtractionContext {
        functions: &mut functions,
        source_text,
        class_name: None,
        parent_function: None,
    };

    extract_from_program(&ret.program, &mut context);
    Ok(functions)
}

struct ExtractionContext<'a> {
    functions: &'a mut Vec<FunctionDefinition>,
    source_text: &'a str,
    class_name: Option<String>,
    parent_function: Option<String>,
}

fn extract_from_program(program: &Program, ctx: &mut ExtractionContext) {
    for stmt in &program.body {
        extract_from_statement(stmt, ctx);
    }
}

fn extract_from_statement(stmt: &Statement, ctx: &mut ExtractionContext) {
    match stmt {
        Statement::FunctionDeclaration(func) => {
            if let Some(name) = &func.id {
                let func_name = name.name.to_string();
                let params = extract_parameters(&func.params);
                let start_line = get_line_number(func.span.start, ctx.source_text);
                ctx.functions.push(FunctionDefinition {
                    name: func_name.clone(),
                    function_type: FunctionType::Function,
                    parameters: params,
                    body_span: func.span,
                    params_span: func.params.span,
                    body_block_span: func.body.as_ref().map(|b| b.span).unwrap_or(func.span),
                    is_arrow_expression: false,
                    is_async: func.r#async,
                    is_generator: func.generator,
                    is_static: false,
                    method_kind: MethodKind::Normal,
                    display_name: func_name.clone(),
                    start_line,
                    end_line: get_line_number(func.span.end, ctx.source_text),
                    class_name: None,
                    parent_function: ctx.parent_function.clone(),
                    node_count: count_function_nodes(func.span, ctx.source_text),
                    has_ignore_directive: has_similarity_ignore_directive(
                        ctx.source_text,
                        start_line as usize,
                    ),
                });

                // Extract nested functions within the function body
                if let Some(body) = &func.body {
                    let saved_parent = ctx.parent_function.clone();
                    ctx.parent_function = Some(func_name);
                    extract_from_function_body(body, ctx);
                    ctx.parent_function = saved_parent;
                }
            }
        }
        Statement::ClassDeclaration(class) => {
            let class_name = class.id.as_ref().map(|id| id.name.to_string());
            let saved_class_name = ctx.class_name.clone();
            ctx.class_name = class_name.clone();

            for element in &class.body.body {
                if let ClassElement::MethodDefinition(method) = element {
                    let method_name = match &method.key {
                        PropertyKey::StaticIdentifier(ident) => ident.name.to_string(),
                        PropertyKey::PrivateIdentifier(ident) => format!("#{}", ident.name),
                        _ => "anonymous".to_string(),
                    };

                    let params = extract_parameters(&method.value.params);
                    let function_type = if method.kind == MethodDefinitionKind::Constructor {
                        FunctionType::Constructor
                    } else {
                        FunctionType::Method
                    };
                    let method_kind = match method.kind {
                        MethodDefinitionKind::Get => MethodKind::Getter,
                        MethodDefinitionKind::Set => MethodKind::Setter,
                        _ => MethodKind::Normal,
                    };
                    // Capture the original source text of the method key so
                    // string/number literal and computed keys (which the
                    // simple `method_name` resolver flattens to
                    // `"anonymous"`) still differentiate during
                    // comparison, and so private `#name` methods survive
                    // the normalization wrapper instead of collapsing onto
                    // a `__sim__` placeholder.
                    let method_display_name = method_key_source_text(&method.key, ctx.source_text)
                        .unwrap_or_else(|| method_name.clone());

                    let method_full_name = if let Some(ref class) = class_name {
                        format!("{class}.{method_name}")
                    } else {
                        method_name.clone()
                    };
                    let start_line = get_line_number(method.span.start, ctx.source_text);

                    ctx.functions.push(FunctionDefinition {
                        name: method_name.clone(),
                        function_type,
                        parameters: params,
                        body_span: method.span,
                        params_span: method.value.params.span,
                        body_block_span: method
                            .value
                            .body
                            .as_ref()
                            .map(|b| b.span)
                            .unwrap_or(method.span),
                        is_arrow_expression: false,
                        is_async: method.value.r#async,
                        is_generator: method.value.generator,
                        is_static: method.r#static,
                        method_kind,
                        display_name: method_display_name.clone(),
                        start_line,
                        end_line: get_line_number(method.span.end, ctx.source_text),
                        class_name: class_name.clone(),
                        parent_function: ctx.parent_function.clone(),
                        node_count: count_function_nodes(method.span, ctx.source_text),
                        has_ignore_directive: has_similarity_ignore_directive(
                            ctx.source_text,
                            start_line as usize,
                        ),
                    });

                    // Extract nested functions within method body
                    if let Some(body) = &method.value.body {
                        let saved_parent = ctx.parent_function.clone();
                        ctx.parent_function = Some(method_full_name);
                        extract_from_function_body(body, ctx);
                        ctx.parent_function = saved_parent;
                    }
                }
            }

            ctx.class_name = saved_class_name;
        }
        Statement::VariableDeclaration(var_decl) => {
            for decl in &var_decl.declarations {
                if let Some(Expression::ArrowFunctionExpression(arrow)) = &decl.init {
                    if let BindingPattern::BindingIdentifier(ident) = &decl.id {
                        let params = extract_parameters(&arrow.params);
                        let arrow_name = ident.name.to_string();
                        let start_line = get_line_number(arrow.span.start, ctx.source_text);
                        ctx.functions.push(FunctionDefinition {
                            name: arrow_name.clone(),
                            function_type: FunctionType::Arrow,
                            parameters: params,
                            body_span: arrow.span,
                            params_span: arrow.params.span,
                            body_block_span: arrow.body.span,
                            is_arrow_expression: arrow.expression,
                            is_async: arrow.r#async,
                            is_generator: false,
                            is_static: false,
                            method_kind: MethodKind::Normal,
                            display_name: arrow_name.clone(),
                            start_line,
                            end_line: get_line_number(arrow.span.end, ctx.source_text),
                            class_name: None,
                            parent_function: ctx.parent_function.clone(),
                            node_count: count_function_nodes(arrow.span, ctx.source_text),
                            has_ignore_directive: has_similarity_ignore_directive(
                                ctx.source_text,
                                start_line as usize,
                            ),
                        });

                        // Extract nested functions within arrow function body
                        if !arrow.expression {
                            let saved_parent = ctx.parent_function.clone();
                            ctx.parent_function = Some(arrow_name);
                            extract_from_function_body(&arrow.body, ctx);
                            ctx.parent_function = saved_parent;
                        }
                    }
                }
            }
        }
        Statement::ExportNamedDeclaration(export) => {
            if let Some(decl) = &export.declaration {
                extract_from_declaration(decl, ctx);
            }
        }
        Statement::ExportDefaultDeclaration(export) => {
            if let ExportDefaultDeclarationKind::FunctionDeclaration(func) = &export.declaration {
                let name = func
                    .id
                    .as_ref()
                    .map(|id| id.name.to_string())
                    .unwrap_or_else(|| "default".to_string());
                let params = extract_parameters(&func.params);
                let func_name = name.clone();
                let start_line = get_line_number(func.span.start, ctx.source_text);
                ctx.functions.push(FunctionDefinition {
                    name: func_name.clone(),
                    function_type: FunctionType::Function,
                    parameters: params,
                    body_span: func.span,
                    params_span: func.params.span,
                    body_block_span: func.body.as_ref().map(|b| b.span).unwrap_or(func.span),
                    is_arrow_expression: false,
                    is_async: func.r#async,
                    is_generator: func.generator,
                    is_static: false,
                    method_kind: MethodKind::Normal,
                    display_name: func_name.clone(),
                    start_line,
                    end_line: get_line_number(func.span.end, ctx.source_text),
                    class_name: None,
                    parent_function: ctx.parent_function.clone(),
                    node_count: count_function_nodes(func.span, ctx.source_text),
                    has_ignore_directive: has_similarity_ignore_directive(
                        ctx.source_text,
                        start_line as usize,
                    ),
                });

                // Extract nested functions within the function body
                if let Some(body) = &func.body {
                    let saved_parent = ctx.parent_function.clone();
                    ctx.parent_function = Some(func_name);
                    extract_from_function_body(body, ctx);
                    ctx.parent_function = saved_parent;
                }
            }
        }
        _ => {}
    }
}

fn extract_from_declaration(decl: &Declaration, ctx: &mut ExtractionContext) {
    match decl {
        Declaration::FunctionDeclaration(func) => {
            if let Some(name) = &func.id {
                let func_name = name.name.to_string();
                let params = extract_parameters(&func.params);
                let start_line = get_line_number(func.span.start, ctx.source_text);
                ctx.functions.push(FunctionDefinition {
                    name: func_name.clone(),
                    function_type: FunctionType::Function,
                    parameters: params,
                    body_span: func.span,
                    params_span: func.params.span,
                    body_block_span: func.body.as_ref().map(|b| b.span).unwrap_or(func.span),
                    is_arrow_expression: false,
                    is_async: func.r#async,
                    is_generator: func.generator,
                    is_static: false,
                    method_kind: MethodKind::Normal,
                    display_name: func_name.clone(),
                    start_line,
                    end_line: get_line_number(func.span.end, ctx.source_text),
                    class_name: None,
                    parent_function: ctx.parent_function.clone(),
                    node_count: count_function_nodes(func.span, ctx.source_text),
                    has_ignore_directive: has_similarity_ignore_directive(
                        ctx.source_text,
                        start_line as usize,
                    ),
                });

                // Extract nested functions within the function body
                if let Some(body) = &func.body {
                    let saved_parent = ctx.parent_function.clone();
                    ctx.parent_function = Some(func_name);
                    extract_from_function_body(body, ctx);
                    ctx.parent_function = saved_parent;
                }
            }
        }
        Declaration::ClassDeclaration(class) => {
            let class_name = class.id.as_ref().map(|id| id.name.to_string());
            let saved_class_name = ctx.class_name.clone();
            ctx.class_name = class_name.clone();

            for element in &class.body.body {
                if let ClassElement::MethodDefinition(method) = element {
                    let method_name = match &method.key {
                        PropertyKey::StaticIdentifier(ident) => ident.name.to_string(),
                        PropertyKey::PrivateIdentifier(ident) => format!("#{}", ident.name),
                        _ => "anonymous".to_string(),
                    };

                    let params = extract_parameters(&method.value.params);
                    let function_type = if method.kind == MethodDefinitionKind::Constructor {
                        FunctionType::Constructor
                    } else {
                        FunctionType::Method
                    };
                    let method_kind = match method.kind {
                        MethodDefinitionKind::Get => MethodKind::Getter,
                        MethodDefinitionKind::Set => MethodKind::Setter,
                        _ => MethodKind::Normal,
                    };
                    // Capture the original source text of the method key so
                    // string/number literal and computed keys (which the
                    // simple `method_name` resolver flattens to
                    // `"anonymous"`) still differentiate during
                    // comparison, and so private `#name` methods survive
                    // the normalization wrapper instead of collapsing onto
                    // a `__sim__` placeholder.
                    let method_display_name = method_key_source_text(&method.key, ctx.source_text)
                        .unwrap_or_else(|| method_name.clone());

                    let method_full_name = if let Some(ref class) = class_name {
                        format!("{class}.{method_name}")
                    } else {
                        method_name.clone()
                    };
                    let start_line = get_line_number(method.span.start, ctx.source_text);

                    ctx.functions.push(FunctionDefinition {
                        name: method_name.clone(),
                        function_type,
                        parameters: params,
                        body_span: method.span,
                        params_span: method.value.params.span,
                        body_block_span: method
                            .value
                            .body
                            .as_ref()
                            .map(|b| b.span)
                            .unwrap_or(method.span),
                        is_arrow_expression: false,
                        is_async: method.value.r#async,
                        is_generator: method.value.generator,
                        is_static: method.r#static,
                        method_kind,
                        display_name: method_display_name.clone(),
                        start_line,
                        end_line: get_line_number(method.span.end, ctx.source_text),
                        class_name: class_name.clone(),
                        parent_function: ctx.parent_function.clone(),
                        node_count: count_function_nodes(method.span, ctx.source_text),
                        has_ignore_directive: has_similarity_ignore_directive(
                            ctx.source_text,
                            start_line as usize,
                        ),
                    });

                    // Extract nested functions within method body
                    if let Some(body) = &method.value.body {
                        let saved_parent = ctx.parent_function.clone();
                        ctx.parent_function = Some(method_full_name);
                        extract_from_function_body(body, ctx);
                        ctx.parent_function = saved_parent;
                    }
                }
            }

            ctx.class_name = saved_class_name;
        }
        Declaration::VariableDeclaration(var) => {
            for decl in &var.declarations {
                if let Some(Expression::ArrowFunctionExpression(arrow)) = &decl.init {
                    if let BindingPattern::BindingIdentifier(ident) = &decl.id {
                        let params = extract_parameters(&arrow.params);
                        let arrow_name = ident.name.to_string();
                        let start_line = get_line_number(arrow.span.start, ctx.source_text);
                        ctx.functions.push(FunctionDefinition {
                            name: arrow_name.clone(),
                            function_type: FunctionType::Arrow,
                            parameters: params,
                            body_span: arrow.span,
                            params_span: arrow.params.span,
                            body_block_span: arrow.body.span,
                            is_arrow_expression: arrow.expression,
                            is_async: arrow.r#async,
                            is_generator: false,
                            is_static: false,
                            method_kind: MethodKind::Normal,
                            display_name: arrow_name.clone(),
                            start_line,
                            end_line: get_line_number(arrow.span.end, ctx.source_text),
                            class_name: None,
                            parent_function: ctx.parent_function.clone(),
                            node_count: count_function_nodes(arrow.span, ctx.source_text),
                            has_ignore_directive: has_similarity_ignore_directive(
                                ctx.source_text,
                                start_line as usize,
                            ),
                        });

                        // Extract nested functions within arrow function body
                        if !arrow.expression {
                            let saved_parent = ctx.parent_function.clone();
                            ctx.parent_function = Some(arrow_name);
                            extract_from_function_body(&arrow.body, ctx);
                            ctx.parent_function = saved_parent;
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn extract_parameters(params: &oxc_ast::ast::FormalParameters) -> Vec<String> {
    params
        .items
        .iter()
        .filter_map(|param| match &param.pattern {
            BindingPattern::BindingIdentifier(ident) => Some(ident.name.to_string()),
            _ => None,
        })
        .collect()
}

fn extract_from_function_body(body: &FunctionBody, ctx: &mut ExtractionContext) {
    for stmt in &body.statements {
        extract_from_statement(stmt, ctx);
    }
}

fn get_line_number(offset: u32, source_text: &str) -> u32 {
    let mut line = 1;
    let mut current_offset = 0;

    for ch in source_text.chars() {
        if current_offset >= offset as usize {
            break;
        }
        if ch == '\n' {
            line += 1;
        }
        current_offset += ch.len_utf8();
    }

    line
}

/// Compare similarity between two functions
pub fn compare_functions(
    func1: &FunctionDefinition,
    func2: &FunctionDefinition,
    source1: &str,
    source2: &str,
    options: &TSEDOptions,
) -> Result<f64, String> {
    compare_functions_with_threshold(func1, func2, source1, source2, options, 0.0)
}

/// Variant that allows callers to provide a similarity threshold so the
/// inner APTED computation can abort early when the result is provably
/// below `threshold`. Returns `Ok(sim)` either way — for sub-threshold
/// pairs the `sim` value is `0.0` (the threshold guard already filtered
/// them) so callers should still apply their own `>= threshold` check
/// before recording the pair.
pub fn compare_functions_with_threshold(
    func1: &FunctionDefinition,
    func2: &FunctionDefinition,
    source1: &str,
    source2: &str,
    options: &TSEDOptions,
    threshold: f64,
) -> Result<f64, String> {
    // Parse using the normalization-friendly fragments so arrow / regular /
    // method declarations whose bodies agree end up structurally equivalent
    // at the tree level. The old path passed the raw function text to
    // `parse_function_fragment`, which preserved the surrounding declaration
    // shape and made e.g. `(x) => x` vs `function f(x) { return x; }` look
    // ~25% less similar than their bodies actually were.
    let tree1 = parse_function_for_comparison("func1.ts", func1, source1)?;
    let tree2 = parse_function_for_comparison("func2.ts", func2, source2)?;
    compare_function_trees(&tree1, &tree2, func1, func2, options, threshold)
}

/// Tree-driven comparator shared by [`compare_functions`] and the
/// pre-parsed batch path used by `find_similar_functions_*`. Splitting
/// this out avoids re-parsing each function body O(N) times during a
/// pairwise scan.
fn compare_function_trees(
    tree1: &std::rc::Rc<crate::tree::TreeNode>,
    tree2: &std::rc::Rc<crate::tree::TreeNode>,
    func1: &FunctionDefinition,
    func2: &FunctionDefinition,
    options: &TSEDOptions,
    threshold: f64,
) -> Result<f64, String> {
    use crate::tsed::calculate_tsed_with_threshold;

    // Pre-filter by size ratio. If the trees are too lopsided the
    // post-penalty similarity can never reach threshold even at
    // distance 0 — short-circuit here so the expensive APTED pass is
    // skipped entirely.
    let size1 = tree1.get_subtree_size() as f64;
    let size2 = tree2.get_subtree_size() as f64;
    // Pre-filter by size ratio. APTED's minimum edit distance is at least
    // `|size1 - size2| * min(delete_cost, insert_cost)` (one tree has to
    // consume the size gap via deletes or inserts at that per-op rate),
    // so the base TSED is bounded above by
    //   `1 - |size1 - size2| * min_op_cost / max_size`.
    // When `delete_cost = insert_cost = 1.0` (the only configuration the
    // WASM CLI ever sets and the documented default) this collapses to
    // `size_ratio`, which gives us a clean `size_ratio < threshold` gate.
    //
    // Direct callers can override `apted_options` though, and with
    // smaller per-op costs the bound is looser — pairs that the simple
    // `size_ratio` formula would discard might still legitimately clear
    // the threshold. Skip the prune entirely in that case rather than
    // attempt a more elaborate bound; the threshold-aware APTED cutoff
    // below still keeps the worst case bounded.
    if threshold > 0.0 && size1 > 0.0 && size2 > 0.0 {
        let delete_cost = options.apted_options.delete_cost;
        let insert_cost = options.apted_options.insert_cost;
        if delete_cost >= 1.0 && insert_cost >= 1.0 {
            let size_ratio = size1.min(size2) / size1.max(size2);
            if size_ratio < threshold {
                return Ok(0.0);
            }
        }
    }

    // Use threshold-pruned APTED when the caller actually supplied a
    // useful budget; otherwise fall through to the full computation so
    // callers (notably the `compare_classes_*` and test fixtures) that
    // expect the exact distance still get it.
    let mut similarity = if threshold > 0.0 {
        // Reserve some headroom for the line-count penalty that may
        // follow — be conservative so we never under-report a real
        // duplicate. Using `threshold * 0.7` keeps the budget loose
        // enough that the penalty layer can shave the score without
        // pushing it below an "exact-zero" sentinel.
        let apted_threshold = (threshold * 0.7).max(0.0);
        let sim = calculate_tsed_with_threshold(tree1, tree2, options, apted_threshold);
        if sim == 0.0 {
            return Ok(0.0);
        }
        sim
    } else {
        calculate_tsed(tree1, tree2, options)
    };

    // Apply line-count size penalty for short functions if enabled. This is a
    // second, coarser guard on top of the node-count penalty in
    // `calculate_tsed` — the purpose is to filter trivial lookalikes like
    // `() => 0` vs `() => 1`. But the original formulation (`similarity *=
    // avg_lines / 10.0` for functions under 10 lines) was far too aggressive
    // for rename-only duplicates: a genuinely-duplicated 6-line helper with
    // renamed identifiers would have its real ~0.91 TSED reduced to 0.55
    // purely because it happened to be short, hiding obvious duplication from
    // the default 0.8 threshold. We still want to discount pairs where the
    // similarity signal is thin AND the function is short, but we should not
    // punish high-similarity matches on short functions just for being short.
    if options.size_penalty {
        let avg_lines = (func1.line_count() + func2.line_count()) as f64 / 2.0;
        if avg_lines < 10.0 {
            // Confidence softener: scale the base short-function factor back
            // toward 1.0 as the raw similarity approaches 1.0. At sim >= 0.92
            // (essentially identical bodies) we apply no further penalty;
            // below 0.6 we apply the full original short-function discount.
            // The previous curve still pulled identical 3-line functions
            // down by ~30% because the confidence range bottomed out at
            // similarity 0.6 — but identical bodies already register at the
            // node-count layer's softened score (~0.85) so the avg_lines
            // multiplier should let those through.
            let base_factor = (avg_lines / 10.0).max(0.1);
            let confidence = ((similarity - 0.6) / 0.32).clamp(0.0, 1.0);
            let effective_factor = base_factor + (1.0 - base_factor) * confidence;
            similarity *= effective_factor;
        }
    }

    Ok(similarity)
}

fn extract_body_text(func: &FunctionDefinition, source: &str) -> String {
    let start = func.body_span.start as usize;
    let end = func.body_span.end as usize;
    source[start..end].to_string()
}

/// Build a normalization-friendly fragment for a function. The goal is that
/// two functions whose bodies and parameter lists agree end up producing
/// identical fragments regardless of whether they were declared as
/// `function foo() {...}`, `const foo = () => {...}`, or
/// `const foo = (x) => x + 1`. Without this layer the wrapping shape
/// (FunctionDeclaration vs ExpressionStatement→ArrowFunctionExpression)
/// dominated the structural distance on short bodies.
fn build_normalized_fragment(func: &FunctionDefinition, source: &str) -> String {
    let safe_slice = |start: u32, end: u32| -> &str {
        let s = start as usize;
        let e = end as usize;
        if s < e && e <= source.len() {
            &source[s..e]
        } else {
            ""
        }
    };

    let params_text = {
        let raw = safe_slice(func.params_span.start, func.params_span.end).trim();
        if raw.starts_with('(') && raw.ends_with(')') {
            raw.to_string()
        } else if raw.is_empty() {
            "()".to_string()
        } else {
            // Single un-parenthesised arrow parameter, e.g. `x => ...`.
            format!("({raw})")
        }
    };

    let body_text = safe_slice(func.body_block_span.start, func.body_block_span.end);

    match func.function_type {
        FunctionType::Method | FunctionType::Constructor => {
            // Plain instance methods normalize to a standalone `function`
            // fragment so "extract method to function" refactors (and the
            // reverse) compare by body instead of by declaration shape —
            // the class wrapper used to dominate the structural distance
            // for method-vs-function pairs. Static methods, accessors,
            // constructors and non-identifier keys keep the class wrapper:
            // their runtime contracts are tied to the class shape and the
            // existing relative scoring for those forms is calibrated
            // against it.
            if matches!(func.function_type, FunctionType::Method)
                && matches!(func.method_kind, MethodKind::Normal)
                && !func.is_static
            {
                if let Some(name_text) = method_fragment_name(&func.display_name) {
                    let mut prefix = String::new();
                    if func.is_async {
                        prefix.push_str("async ");
                    }
                    prefix.push_str("function");
                    if func.is_generator {
                        prefix.push('*');
                    }
                    return format!("{prefix} {name_text}{params_text} {body_text}");
                }
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
            // these forms and collapse distinct methods onto `__sim__`.
            let method_name_text = if func.display_name.is_empty() {
                "__sim__".to_string()
            } else {
                func.display_name.clone()
            };
            format!(
                "class __C__ {{ {}{}{} {} }}",
                prefix, method_name_text, params_text, body_text
            )
        }
        FunctionType::Function | FunctionType::Arrow => {
            // Carry async / generator flags through the wrapper so two
            // functions that differ only in `async`-ness (different
            // runtime return type) don't compare as identical. Arrows
            // can be async but never generators, so the generator marker
            // is only meaningful for the Function path.
            let mut prefix = String::new();
            if func.is_async {
                prefix.push_str("async ");
            }
            prefix.push_str("function");
            if func.is_generator {
                prefix.push('*');
            }

            // Top-level functions and arrow declarations always bind to
            // an ordinary identifier, so the sanitizer is enough — there
            // is no private-method / string-literal-key path here.
            let name_text = sanitize_function_name(&func.name);
            if func.is_arrow_expression {
                // Wrap a single-expression arrow body in an explicit
                // `return` so it ends up shaped like a block-bodied
                // function. Without this an `(x) => x + 1` would parse as
                // a top-level `ExpressionStatement → ArrowFunctionExpression`,
                // adding a structural wrapper that an equivalent
                // `function f(x) { return x + 1; }` would not have.
                format!(
                    "{} {}{} {{ return {}; }}",
                    prefix, name_text, params_text, body_text
                )
            } else {
                format!("{} {}{} {}", prefix, name_text, params_text, body_text)
            }
        }
    }
}

fn is_plain_identifier(name: &str) -> bool {
    !name.is_empty()
        && name.chars().enumerate().all(|(idx, ch)| {
            if idx == 0 {
                ch.is_alphabetic() || ch == '_' || ch == '$'
            } else {
                ch.is_alphanumeric() || ch == '_' || ch == '$'
            }
        })
}

fn sanitize_function_name(name: &str) -> String {
    if is_plain_identifier(name) {
        name.to_string()
    } else {
        "__sim__".to_string()
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

/// Name to bind a plain instance method to when normalizing it into a
/// standalone `function` fragment. Returns `None` for non-identifier keys
/// (string literals, computed keys, `#private` names) — those keep the
/// class wrapper so their key shape stays visible to the comparison.
/// Reserved words get a stable prefix so e.g. a `delete(key)` method still
/// produces a valid, name-distinct function fragment.
fn method_fragment_name(name: &str) -> Option<String> {
    if !is_plain_identifier(name) {
        return None;
    }
    if is_reserved_function_name(name) {
        Some(format!("__m_{name}"))
    } else {
        Some(name.to_string())
    }
}

/// Parse a function body snippet into a tree, wrapping method-shorthand
/// fragments in a synthetic class so that standalone class methods and
/// constructors (which are not valid top-level TypeScript) can still be
/// structurally compared.
fn parse_function_fragment(
    filename: &str,
    body_text: &str,
    function_type: &FunctionType,
) -> Result<std::rc::Rc<crate::tree::TreeNode>, String> {
    match function_type {
        FunctionType::Method | FunctionType::Constructor => {
            let wrapped = format!("class __C__ {{ {body_text} }}");
            parse_and_convert_to_tree_canonical(filename, &wrapped)
        }
        FunctionType::Function | FunctionType::Arrow => {
            // Try direct parse first; fall back to class-wrapping for method-like
            // snippets that snuck through (e.g. rare extractor edge cases).
            match parse_and_convert_to_tree_canonical(filename, body_text) {
                Ok(tree) => Ok(tree),
                Err(_) => {
                    let wrapped = format!("class __C__ {{ {body_text} }}");
                    parse_and_convert_to_tree_canonical(filename, &wrapped)
                }
            }
        }
    }
}

/// Parse a function for structural comparison, using `build_normalized_fragment`
/// so arrow vs regular vs method declarations all end up shape-equivalent
/// when their bodies agree, and the canonical parse so style-only rewrites
/// (template literals, `.then` vs `await`, `forEach` vs `for-of`, …)
/// compare as equal trees.
fn parse_function_for_comparison(
    filename: &str,
    func: &FunctionDefinition,
    source: &str,
) -> Result<std::rc::Rc<crate::tree::TreeNode>, String> {
    let fragment = build_normalized_fragment(func, source);
    match parse_and_convert_to_tree_canonical(filename, &fragment) {
        Ok(tree) => Ok(tree),
        Err(_) => {
            // Fallback to the legacy whole-function parse path so we don't
            // regress on edge cases the normalizer happens to break.
            let body_text = extract_body_text(func, source);
            parse_function_fragment(filename, &body_text, &func.function_type)
        }
    }
}

/// Count the number of AST nodes in a function body
fn count_function_nodes(body_span: Span, source_text: &str) -> Option<u32> {
    let start = body_span.start as usize;
    let end = body_span.end as usize;
    if start >= end || end > source_text.len() {
        return None;
    }

    let body_text = &source_text[start..end];

    // For now, try to parse the text as-is
    // If it fails, try wrapping it in a way that makes it valid TypeScript
    match parse_and_convert_to_tree("temp.ts", body_text) {
        Ok(tree) => Some(tree.get_subtree_size() as u32),
        Err(_) => {
            // If direct parsing fails, try wrapping in a minimal context
            // This handles cases like "constructor(private x: number) {}" or method definitions
            let wrapped = if body_text.starts_with("constructor") {
                format!("class C {{ {body_text} }}")
            } else if body_text.contains("(") && body_text.contains(")") && body_text.contains("{")
            {
                // Likely a method or function - wrap it appropriately
                if body_text.trim().starts_with(|c: char| c.is_alphabetic() || c == '_' || c == '#')
                {
                    // Method-like syntax
                    format!("class C {{ {body_text} }}")
                } else {
                    // Arrow function or other expression
                    format!("const x = {body_text}")
                }
            } else {
                // Default fallback
                body_text.to_string()
            };

            match parse_and_convert_to_tree("temp.ts", &wrapped) {
                Ok(tree) => {
                    // Subtract nodes added by wrapping (approximation)
                    let base_nodes = if wrapped.starts_with("class C") {
                        3 // class node + body node + wrapping
                    } else if wrapped.starts_with("const x") {
                        2 // const declaration + wrapping
                    } else {
                        0
                    };
                    Some((tree.get_subtree_size().saturating_sub(base_nodes)) as u32)
                }
                Err(_) => {
                    // If all else fails, make a rough estimate based on the text
                    // Count common syntax elements as a fallback
                    let node_count =
                        body_text.matches(['{', '}', '(', ')', ';']).count() as u32 + 1;
                    Some(node_count.max(1))
                }
            }
        }
    }
}

/// Find similar functions within the same file
pub fn find_similar_functions_in_file(
    filename: &str,
    source_text: &str,
    threshold: f64,
    options: &TSEDOptions,
) -> Result<Vec<SimilarityResult>, String> {
    let mut functions = extract_functions(filename, source_text)?;
    functions.retain(|function| !function.has_ignore_directive);

    // Pre-parse each function's body once so the O(N²) pairwise compare
    // doesn't re-parse the same body N times. Parsing dominates the
    // per-pair cost; doing it once per function alone takes the perf
    // sweep on the TypeScript compiler's `src/services` folder from
    // ~4 minutes down to roughly upstream parity.
    let mut trees: Vec<Option<std::rc::Rc<crate::tree::TreeNode>>> =
        Vec::with_capacity(functions.len());
    for func in &functions {
        trees.push(parse_function_for_comparison(filename, func, source_text).ok());
    }

    let mut similar_pairs = Vec::new();

    // Compare all pairs
    for i in 0..functions.len() {
        for j in (i + 1)..functions.len() {
            // Skip if either function is too short
            if let Some(min_tokens) = options.min_tokens {
                // If min_tokens is specified, use token count instead of line count
                let tokens_i = functions[i].node_count.unwrap_or(0);
                let tokens_j = functions[j].node_count.unwrap_or(0);
                if tokens_i < min_tokens || tokens_j < min_tokens {
                    continue;
                }
            } else {
                // Otherwise use line count
                if functions[i].line_count() < options.min_lines
                    || functions[j].line_count() < options.min_lines
                {
                    continue;
                }
            }

            // Skip if functions have parent-child relationship
            if functions[i].is_parent_child_relationship(&functions[j]) {
                continue;
            }

            let (Some(tree_i), Some(tree_j)) = (&trees[i], &trees[j]) else {
                continue;
            };

            let similarity = match compare_function_trees(
                tree_i,
                tree_j,
                &functions[i],
                &functions[j],
                options,
                threshold,
            ) {
                Ok(sim) => sim,
                Err(_) => continue,
            };

            if similarity >= threshold {
                similar_pairs.push(SimilarityResult::new(
                    functions[i].clone(),
                    functions[j].clone(),
                    similarity,
                ));
            }
        }
    }

    // Sort by impact (descending), then by similarity (descending)
    similar_pairs.sort_by(|a, b| {
        b.impact
            .cmp(&a.impact)
            .then(b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal))
    });

    Ok(similar_pairs)
}

/// Find similar functions across multiple files
pub fn find_similar_functions_across_files(
    files: &[(String, String)], // (filename, source_text)
    threshold: f64,
    options: &TSEDOptions,
) -> Result<CrossFileSimilarityResult, String> {
    let mut all_functions = Vec::new();

    // Extract functions from all files, skipping files that fail to parse
    // rather than aborting the whole batch.
    for (filename, source) in files {
        let mut functions = match extract_functions(filename, source) {
            Ok(funcs) => funcs,
            Err(_) => continue,
        };
        functions.retain(|function| !function.has_ignore_directive);
        for func in functions {
            all_functions.push((filename.clone(), source.clone(), func));
        }
    }

    // Pre-parse function bodies once so the pairwise loop avoids the
    // dominant parse-per-pair cost. See the comment on the same-file
    // variant for why this matters — at 1000+ functions the parse work
    // alone dwarfs the APTED computation.
    let trees: Vec<Option<std::rc::Rc<crate::tree::TreeNode>>> = all_functions
        .iter()
        .map(|(filename, source, func)| parse_function_for_comparison(filename, func, source).ok())
        .collect();

    let mut similar_pairs = Vec::new();

    // Compare all pairs across files
    for i in 0..all_functions.len() {
        for j in (i + 1)..all_functions.len() {
            let (first_file, _source1, func1) = &all_functions[i];
            let (second_file, _source2, func2) = &all_functions[j];

            // Skip if same file (already handled by find_similar_functions_in_file)
            if first_file == second_file {
                continue;
            }

            // Skip if either function is too short
            if let Some(min_tokens) = options.min_tokens {
                // If min_tokens is specified, use token count instead of line count
                let tokens1 = func1.node_count.unwrap_or(0);
                let tokens2 = func2.node_count.unwrap_or(0);
                if tokens1 < min_tokens || tokens2 < min_tokens {
                    continue;
                }
            } else {
                // Otherwise use line count
                if func1.line_count() < options.min_lines || func2.line_count() < options.min_lines
                {
                    continue;
                }
            }

            // Skip if functions have parent-child relationship (across files)
            if func1.is_parent_child_relationship(func2) {
                continue;
            }

            let (Some(tree1), Some(tree2)) = (&trees[i], &trees[j]) else {
                continue;
            };

            let similarity = match compare_function_trees(
                tree1, tree2, func1, func2, options, threshold,
            ) {
                Ok(sim) => sim,
                Err(_) => continue,
            };

            if similarity >= threshold {
                similar_pairs.push((
                    first_file.clone(),
                    SimilarityResult::new(func1.clone(), func2.clone(), similarity),
                    second_file.clone(),
                ));
            }
        }
    }

    // Sort by impact (descending), then by similarity (descending)
    similar_pairs.sort_by(|a, b| {
        b.1.impact
            .cmp(&a.1.impact)
            .then(b.1.similarity.partial_cmp(&a.1.similarity).unwrap_or(std::cmp::Ordering::Equal))
    });

    Ok(similar_pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_functions() {
        let code = r"
            function add(a: number, b: number): number {
                return a + b;
            }
            
            const multiply = (x: number, y: number) => x * y;
            
            class Calculator {
                constructor(private initial: number) {}
                
                add(value: number): number {
                    return this.initial + value;
                }
                
                subtract(value: number): number {
                    return this.initial - value;
                }
            }
            
            export function divide(a: number, b: number): number {
                return a / b;
            }
        ";

        let functions = extract_functions("test.ts", code).unwrap();

        assert_eq!(functions.len(), 6);

        // Check function names
        let names: Vec<&str> = functions.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"multiply"));
        assert!(names.contains(&"constructor"));
        assert!(names.contains(&"subtract"));
        assert!(names.contains(&"divide"));

        // Check function types
        let add_func =
            functions.iter().find(|f| f.name == "add" && f.class_name.is_none()).unwrap();
        assert_eq!(add_func.function_type, FunctionType::Function);
        assert_eq!(add_func.parameters, vec!["a", "b"]);

        let multiply_func = functions.iter().find(|f| f.name == "multiply").unwrap();
        assert_eq!(multiply_func.function_type, FunctionType::Arrow);

        let constructor = functions.iter().find(|f| f.name == "constructor").unwrap();
        assert_eq!(constructor.function_type, FunctionType::Constructor);
        assert_eq!(constructor.class_name, Some("Calculator".to_string()));

        // Check that node_count is populated for all functions
        for func in &functions {
            assert!(
                func.node_count.is_some(),
                "Function {} should have node_count populated",
                func.name
            );
            // Node count should be reasonable (greater than 0)
            assert!(
                func.node_count.unwrap() > 0,
                "Function {} should have positive node_count",
                func.name
            );
        }
    }

    #[test]
    fn test_node_count_calculation() {
        let code = r#"
            function simple() {
                return 42;
            }
            
            function complex(a: number, b: number): number {
                if (a > b) {
                    return a - b;
                } else {
                    return a + b;
                }
            }
        "#;

        let functions = extract_functions("test.ts", code).unwrap();

        let simple = functions.iter().find(|f| f.name == "simple").unwrap();
        let complex = functions.iter().find(|f| f.name == "complex").unwrap();

        println!("Simple function node count: {:?}", simple.node_count);
        println!("Complex function node count: {:?}", complex.node_count);

        // Simple function should have fewer nodes than complex
        assert!(simple.node_count.is_some());
        assert!(complex.node_count.is_some());
        assert!(simple.node_count.unwrap() < complex.node_count.unwrap());
    }

    #[test]
    fn test_find_similar_functions_in_file() {
        let code = r"
            function calculateSum(a: number, b: number): number {
                return a + b;
            }
            
            function addNumbers(x: number, y: number): number {
                return x + y;
            }
            
            function multiply(a: number, b: number): number {
                return a * b;
            }
            
            function computeSum(first: number, second: number): number {
                return first + second;
            }
        ";

        let mut options = TSEDOptions::default();
        options.apted_options.rename_cost = 0.3; // Lower rename cost for better similarity detection
        options.size_penalty = false; // Disable for test with small functions
        options.min_lines = 1; // Allow small functions in test

        let similar_pairs = find_similar_functions_in_file("test.ts", code, 0.7, &options).unwrap();

        // Should find that calculateSum, addNumbers, and computeSum are similar
        assert!(
            similar_pairs.len() >= 2,
            "Expected at least 2 similar pairs, found {}",
            similar_pairs.len()
        );

        // Note: multiply IS similar to others because they all have the same structure
        // (two parameters, single return statement). This is expected behavior.
        // Let's check that we found the expected similar pairs
        let sum_pairs = similar_pairs
            .iter()
            .filter(|result| {
                (result.func1.name.contains("Sum") || result.func2.name.contains("Sum"))
                    || (result.func1.name == "addNumbers" || result.func2.name == "addNumbers")
            })
            .count();
        assert!(sum_pairs >= 3, "Expected at least 3 pairs involving sum functions");
    }

    #[test]
    fn test_find_similar_functions_across_files() {
        let file1 = (
            "file1.ts".to_string(),
            r#"
            export function processUser(user: User): void {
                validateUser(user);
                saveUser(user);
                notifyUser(user);
            }
            
            function validateUser(user: User): boolean {
                return user.name.length > 0 && user.email.includes('@');
            }
        "#
            .to_string(),
        );

        let file2 = (
            "file2.ts".to_string(),
            r#"
            export function handleUser(u: User): void {
                checkUser(u);
                storeUser(u);
                alertUser(u);
            }
            
            function checkUser(u: User): boolean {
                return u.name.length > 0 && u.email.includes('@');
            }
        "#
            .to_string(),
        );

        let mut options = TSEDOptions::default();
        options.apted_options.rename_cost = 0.3;
        options.size_penalty = false; // Disable for test with small functions
        options.min_lines = 1; // Allow small functions in test

        let similar_pairs =
            find_similar_functions_across_files(&[file1, file2], 0.7, &options).unwrap();

        // Should find that processUser/handleUser and validateUser/checkUser are similar
        assert!(similar_pairs.len() >= 2);

        // Check specific pairs
        let process_handle = similar_pairs.iter().find(|(_, result, _)| {
            (result.func1.name == "processUser" && result.func2.name == "handleUser")
                || (result.func1.name == "handleUser" && result.func2.name == "processUser")
        });
        assert!(process_handle.is_some());

        let validate_check = similar_pairs.iter().find(|(_, result, _)| {
            (result.func1.name == "validateUser" && result.func2.name == "checkUser")
                || (result.func1.name == "checkUser" && result.func2.name == "validateUser")
        });
        assert!(validate_check.is_some());
    }

    #[test]
    fn test_extract_functions_marks_similarity_ignore_directives() {
        let code = r#"
function keepMe() {
    return 1;
}

// similarity-ignore
function ignoreMe() {
    return 2;
}

/**
 * Keep this duplicated export local for now.
 */
// similarity-ignore
export function ignoredExport() {
    return 3;
}
"#;

        let functions = extract_functions("test.ts", code).unwrap();

        let keep = functions.iter().find(|f| f.name == "keepMe").unwrap();
        assert!(!keep.has_ignore_directive);

        let ignored = functions.iter().find(|f| f.name == "ignoreMe").unwrap();
        assert!(ignored.has_ignore_directive);

        let ignored_export = functions.iter().find(|f| f.name == "ignoredExport").unwrap();
        assert!(ignored_export.has_ignore_directive);
    }
}
