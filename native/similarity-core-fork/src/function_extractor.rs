use oxc_ast::ast::*;
use oxc_span::{GetSpan, Span};

use crate::ignore_directive::has_similarity_ignore_directive;
use crate::parser::parse_and_convert_to_tree_canonical;
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
    let line_offsets = build_line_offsets(source_text);
    let mut context = ExtractionContext {
        functions: &mut functions,
        source_text,
        line_offsets: &line_offsets,
        class_name: None,
        parent_function: None,
    };

    extract_from_program(&ret.program, &mut context);
    Ok(functions)
}

struct ExtractionContext<'a> {
    functions: &'a mut Vec<FunctionDefinition>,
    source_text: &'a str,
    /// Byte offset of the start of each line, so span→line lookups are a
    /// binary search instead of an O(file) rescan per call.
    line_offsets: &'a [u32],
    class_name: Option<String>,
    parent_function: Option<String>,
}

impl ExtractionContext<'_> {
    fn line_number(&self, offset: u32) -> u32 {
        line_number_for_offset(self.line_offsets, offset)
    }
}

/// 1-based line number for a byte offset, given the per-line start
/// offsets (always non-empty; index 0 is line 1 at offset 0).
fn line_number_for_offset(line_offsets: &[u32], offset: u32) -> u32 {
    let index = line_offsets.partition_point(|&start| start <= offset);
    index.max(1) as u32
}

/// Byte offsets at which each line starts.
fn build_line_offsets(source_text: &str) -> Vec<u32> {
    let mut offsets = Vec::with_capacity(source_text.len() / 24 + 1);
    offsets.push(0);
    for (position, byte) in source_text.bytes().enumerate() {
        if byte == b'\n' {
            offsets.push(position as u32 + 1);
        }
    }
    offsets
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
                let start_line = ctx.line_number(func.span.start);
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
                    end_line: ctx.line_number(func.span.end),
                    class_name: None,
                    parent_function: ctx.parent_function.clone(),
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
                    let start_line = ctx.line_number(method.span.start);

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
                        end_line: ctx.line_number(method.span.end),
                        class_name: class_name.clone(),
                        parent_function: ctx.parent_function.clone(),
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
                        let start_line = ctx.line_number(arrow.span.start);
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
                            end_line: ctx.line_number(arrow.span.end),
                            class_name: None,
                            parent_function: ctx.parent_function.clone(),
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
                let start_line = ctx.line_number(func.span.start);
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
                    end_line: ctx.line_number(func.span.end),
                    class_name: None,
                    parent_function: ctx.parent_function.clone(),
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
                let start_line = ctx.line_number(func.span.start);
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
                    end_line: ctx.line_number(func.span.end),
                    class_name: None,
                    parent_function: ctx.parent_function.clone(),
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
                    let start_line = ctx.line_number(method.span.start);

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
                        end_line: ctx.line_number(method.span.end),
                        class_name: class_name.clone(),
                        parent_function: ctx.parent_function.clone(),
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
                        let start_line = ctx.line_number(arrow.span.start);
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
                            end_line: ctx.line_number(arrow.span.end),
                            class_name: None,
                            parent_function: ctx.parent_function.clone(),
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

    // Behavioral-atom twin cap. Alpha-renaming makes true rename clones
    // exactly equal, so a pair that is near-identical EXCEPT for a couple
    // of behavior-carrying atoms — a different callee (`sendEmail` vs
    // `sendSms`), a flipped operator (`*` vs `/`, `&&` vs `||`), a
    // different builtin (`find` vs `findLast`), a for-of turned for-in —
    // is precisely the "twins that do different things" false-positive
    // shape. The direction of the difference matters:
    //
    //   * SUBSTITUTIONS (each side has atoms the other lacks) rewrite
    //     behavior and cap hard;
    //   * one-sided ADDITIONS (one function is the other plus extra
    //     work, e.g. an added logging line) are the classic
    //     copy-paste-and-extend duplicate and stay reportable;
    //   * any CONTROL-flow atom difference (loop kind, `break`,
    //     `continue`) caps hard even one-sided — an added `break`
    //     changes what the loop computes, unlike an added call.
    //
    // Plain string/number/boolean literals are deliberately NOT atoms:
    // twins differing only in data constants are parameterizable
    // duplicates (the corpus's F-P02 precedent).
    if similarity > 0.6 {
        let difference = behavioral_atom_difference(tree1, tree2);
        if difference.control > 0 {
            similarity = similarity.min(0.72);
        } else if difference.missing_in_left > 0 && difference.missing_in_right > 0 {
            let total = difference.missing_in_left + difference.missing_in_right;
            #[allow(clippy::cast_precision_loss)]
            let cap = (0.75 - 0.03 * (total.saturating_sub(2)) as f64).max(0.55);
            similarity = similarity.min(cap);
        } else if difference.missing_in_left + difference.missing_in_right >= 4 {
            similarity = similarity.min(0.85);
        }
    }

    // Statement-order factor: two bodies made of the same statements in a
    // different order share all their atoms and most of their tree. When
    // the reordered statements touch the same identifiers, order is
    // behavior (`total += fee; total *= rate;` vs the reverse) and the
    // pair gets a distinctness multiplier; independent statements (two
    // unrelated `const`s swapped) reorder freely and stay untouched.
    if similarity >= 0.8 && dependent_statements_permuted(tree1, tree2) {
        similarity *= 0.85;
    }

    Ok(similarity)
}

#[derive(Debug, Default)]
struct AtomSets {
    /// Value-level atoms: free identifiers, operators, member/call
    /// optionality markers, null, regexes.
    value_atoms: std::collections::HashSet<String>,
    /// Control-flow atoms: loop kinds and loop jumps.
    control_atoms: std::collections::HashSet<String>,
}

struct AtomDifference {
    missing_in_left: usize,
    missing_in_right: usize,
    control: usize,
}

/// Collect behavior-carrying labels: free identifiers (anything
/// alpha-renaming left intact — call targets, property names, globals),
/// operator labels, optional-chaining markers, `null`, regex literals,
/// plus loop kinds and loop jumps as a separate control class.
///
/// Destructuring-pattern KEYS are skipped: `function f({ x, y })` vs
/// `function f({ left, top })` renames the parameter contract the same
/// way renaming two positional parameters does, and the corpus treats
/// those as duplicates.
fn collect_behavioral_atoms(node: &crate::tree::TreeNode, atoms: &mut AtomSets) {
    let is_value_atom = match node.value.as_str() {
        "Identifier" | "BindingIdentifier" | "PrivateIdentifier" => {
            !node.label.starts_with('§')
        }
        "BinaryExpression" | "LogicalExpression" | "UnaryExpression" | "UpdateExpression"
        | "AssignmentExpression" | "RegExpLiteral" | "StaticMemberExpression"
        | "ComputedMemberExpression" | "PrivateFieldExpression" | "CallExpression"
        | "NullLiteral" => true,
        _ => false,
    };
    if is_value_atom {
        atoms.value_atoms.insert(node.label.clone());
    }
    match node.value.as_str() {
        "ForOfStatement" | "ForInStatement" | "ForStatement" | "WhileStatement"
        | "DoWhileStatement" | "BreakStatement" | "ContinueStatement" => {
            atoms.control_atoms.insert(node.label.clone());
        }
        _ => {}
    }
    collect_boundary_call_atoms(node, atoms);
    collect_fold_direction_atoms(node, atoms);
    let skip_pattern_key = node.value == "BindingProperty";
    for (index, child) in node.children.iter().enumerate() {
        if skip_pattern_key && index == 0 {
            continue;
        }
        collect_behavioral_atoms(child, atoms);
    }
}

/// Positional/slicing builtins whose numeric arguments and arity are
/// boundary semantics, not parameterizable data. `.at(-1)` reads the
/// newest entry while `.at(0)` reads the oldest, and `.slice(0, n)` keeps
/// exactly the head that `.slice(n)` drops — twins one index literal (or
/// one argument) apart do different work, unlike data-literal twins (a
/// table name, a status code) which stay reportable as parameterizable
/// duplicates.
fn collect_boundary_call_atoms(node: &crate::tree::TreeNode, atoms: &mut AtomSets) {
    if node.value != "CallExpression" {
        return;
    }
    let Some(callee) = node.children.first() else {
        return;
    };
    if callee.value != "StaticMemberExpression" || callee.children.len() != 2 {
        return;
    }
    let property = &callee.children[1];
    if property.value != "Identifier" {
        return;
    }
    let name = property.label.as_str();
    if !matches!(
        name,
        "at" | "slice" | "splice" | "charAt" | "charCodeAt" | "codePointAt" | "substring"
            | "substr"
    ) {
        return;
    }
    let arity = node.children.len() - 1;
    atoms.value_atoms.insert(format!("boundary:{name}/{arity}"));
    // `splice` is the one member of the family whose trailing arguments
    // are inserted VALUES, not positions: only `start` and `deleteCount`
    // (positions 0 and 1) are boundaries, and twins differing in an
    // inserted value are ordinary data-literal duplicates.
    let boundary_positions = if name == "splice" { 2 } else { usize::MAX };
    for (position, argument) in node.children.iter().skip(1).enumerate() {
        if position >= boundary_positions {
            break;
        }
        if let Some(index_literal) = static_index_literal(argument) {
            atoms
                .value_atoms
                .insert(format!("boundary:{name}[{position}]={index_literal}"));
        }
    }
}

/// `0`, `1`, `-1`, … — a literal (possibly negated) numeric index.
fn static_index_literal(node: &crate::tree::TreeNode) -> Option<String> {
    if node.value == "NumericLiteral" {
        return Some(node.label.clone());
    }
    if node.value == "UnaryExpression" && node.label == "UnaryNegation" && node.children.len() == 1
    {
        let inner = &node.children[0];
        if inner.value == "NumericLiteral" {
            return Some(format!("-{}", inner.label));
        }
    }
    None
}

/// Fold-direction atoms: an assignment that rebuilds its own target from a
/// `+` chain is order-sensitive because string `+` is not commutative —
/// `trail = trail + seg + "/"` appends while `trail = seg + "/" + trail`
/// prepends, and the two loops produce reversed outputs. The accumulator's
/// position in the chain (head / mid / tail) is therefore a behavioral
/// atom. Two deliberate scope limits:
///
/// * the atom only fires when a chain operand is a string LITERAL —
///   that's the one case where the concatenation (and hence its
///   direction) is provable from syntax. Numeric folds (`sum += n` vs
///   `sum = n + sum`) are commutative and must not be penalized, and an
///   untyped identifier chain could be either, so it gets no atom.
/// * the target may be a local (`trail`) or a member slot
///   (`this.trail`, `state.trail`) — member accumulators compare
///   structurally via the same node hash the statement-permutation
///   check uses.
///
/// The canonicalizer already contracts `x = x + y` onto the `+=` shape
/// (an `AssignmentExpression` labeled `Addition`), which is by
/// construction a head fold; non-`+` operators are left alone — their
/// swapped-operand twins already diverge structurally through the
/// compound-contraction asymmetry.
fn collect_fold_direction_atoms(node: &crate::tree::TreeNode, atoms: &mut AtomSets) {
    if node.value != "AssignmentExpression" || node.children.len() != 2 {
        return;
    }
    let target = &node.children[0];
    if !matches!(
        target.value.as_str(),
        "Identifier"
            | "BindingIdentifier"
            | "StaticMemberExpression"
            | "PrivateFieldExpression"
            | "ComputedMemberExpression"
    ) {
        return;
    }
    let chain = &node.children[1];
    let mut leaves = Vec::new();
    if node.label == "Addition" {
        // Contracted `x += y` / `x = x + y`: the accumulator is the chain
        // head; the flattened RHS supplies the string evidence.
        flatten_addition_chain(chain, &mut leaves);
        if leaves.iter().any(|leaf| leaf.value == "StringLiteral") {
            atoms.value_atoms.insert("fold:head".to_string());
        }
        return;
    }
    if node.label != "Assign" {
        return;
    }
    if chain.value != "BinaryExpression" || chain.label != "Addition" {
        return;
    }
    flatten_addition_chain(chain, &mut leaves);
    if !leaves.iter().any(|leaf| leaf.value == "StringLiteral") {
        return;
    }
    let target_hash = structural_node_hash(target);
    for (position, leaf) in leaves.iter().enumerate() {
        if leaf.value == target.value && structural_node_hash(leaf) == target_hash {
            let marker = if position == 0 {
                "fold:head"
            } else if position == leaves.len() - 1 {
                "fold:tail"
            } else {
                "fold:mid"
            };
            atoms.value_atoms.insert(marker.to_string());
        }
    }
}

/// Collect the operand leaves of a left/right-nested `+` chain in source
/// order.
fn flatten_addition_chain<'a>(
    node: &'a crate::tree::TreeNode,
    leaves: &mut Vec<&'a crate::tree::TreeNode>,
) {
    if node.value == "BinaryExpression" && node.label == "Addition" {
        for child in &node.children {
            flatten_addition_chain(child, leaves);
        }
    } else {
        leaves.push(node);
    }
}

fn behavioral_atom_difference(
    tree1: &std::rc::Rc<crate::tree::TreeNode>,
    tree2: &std::rc::Rc<crate::tree::TreeNode>,
) -> AtomDifference {
    let mut atoms1 = AtomSets::default();
    let mut atoms2 = AtomSets::default();
    collect_behavioral_atoms(tree1, &mut atoms1);
    collect_behavioral_atoms(tree2, &mut atoms2);
    AtomDifference {
        missing_in_right: atoms1.value_atoms.difference(&atoms2.value_atoms).count(),
        missing_in_left: atoms2.value_atoms.difference(&atoms1.value_atoms).count(),
        control: atoms1
            .control_atoms
            .symmetric_difference(&atoms2.control_atoms)
            .count(),
    }
}

fn structural_node_hash(node: &crate::tree::TreeNode) -> u64 {
    use std::hash::{Hash, Hasher};
    fn walk(node: &crate::tree::TreeNode, hasher: &mut impl Hasher) {
        node.label.hash(hasher);
        node.value.hash(hasher);
        node.children.len().hash(hasher);
        for child in &node.children {
            walk(child, hasher);
        }
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    walk(node, &mut hasher);
    hasher.finish()
}

/// Whether the two fragments' top-level body statements are the same
/// multiset in a different order, AND at least one out-of-order pair
/// touches a shared identifier (a data dependency — reordering those is
/// behavioral; reordering independent statements is style).
fn dependent_statements_permuted(
    tree1: &std::rc::Rc<crate::tree::TreeNode>,
    tree2: &std::rc::Rc<crate::tree::TreeNode>,
) -> bool {
    fn body_statements(
        tree: &crate::tree::TreeNode,
    ) -> Option<&Vec<std::rc::Rc<crate::tree::TreeNode>>> {
        let function = tree.children.iter().find(|child| {
            child.value == "FunctionDeclaration" || child.value == "ClassDeclaration"
        })?;
        let block = function.children.iter().find(|child| child.value == "BlockStatement")?;
        Some(&block.children)
    }
    fn identifier_labels(
        node: &crate::tree::TreeNode,
        labels: &mut std::collections::HashSet<String>,
    ) {
        if matches!(node.value.as_str(), "Identifier" | "BindingIdentifier") {
            labels.insert(node.label.clone());
        }
        for child in &node.children {
            identifier_labels(child, labels);
        }
    }

    let (Some(statements1), Some(statements2)) = (body_statements(tree1), body_statements(tree2))
    else {
        return false;
    };
    if statements1.len() < 2 || statements1.len() != statements2.len() {
        return false;
    }
    let hashes1: Vec<u64> = statements1.iter().map(|s| structural_node_hash(s)).collect();
    let hashes2: Vec<u64> = statements2.iter().map(|s| structural_node_hash(s)).collect();
    if hashes1 == hashes2 {
        return false;
    }
    let mut sorted1 = hashes1.clone();
    let mut sorted2 = hashes2.clone();
    sorted1.sort_unstable();
    sorted2.sort_unstable();
    if sorted1 != sorted2 {
        return false;
    }

    // Same multiset, different order: find statement pairs whose relative
    // order flipped and check whether any such pair shares an identifier.
    let position_in_2: std::collections::HashMap<u64, Vec<usize>> = {
        let mut map: std::collections::HashMap<u64, Vec<usize>> = Default::default();
        for (position, hash) in hashes2.iter().enumerate() {
            map.entry(*hash).or_default().push(position);
        }
        map
    };
    // Map each statement of side 1 to a position on side 2 (first unused
    // occurrence for duplicate hashes).
    let mut used: std::collections::HashMap<u64, usize> = Default::default();
    let mapped: Vec<usize> = hashes1
        .iter()
        .map(|hash| {
            let index = used.entry(*hash).or_insert(0);
            let positions = &position_in_2[hash];
            let position = positions[(*index).min(positions.len() - 1)];
            *index += 1;
            position
        })
        .collect();

    let mut labels: Vec<std::collections::HashSet<String>> = Vec::with_capacity(statements1.len());
    for statement in statements1 {
        let mut set = std::collections::HashSet::new();
        identifier_labels(statement, &mut set);
        labels.push(set);
    }

    for a in 0..mapped.len() {
        for b in (a + 1)..mapped.len() {
            if mapped[a] > mapped[b] && !labels[a].is_disjoint(&labels[b]) {
                return true;
            }
        }
    }
    false
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

/// Pair-generation scope for [`find_similar_function_pairs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairScope {
    All,
    SameFileOnly,
    CrossFileOnly,
}

/// A file the unified scan had to skip, with the parse error that caused
/// it.
pub type SkippedFile = (String, String);

/// Unified similar-function scan across a set of files.
///
/// Extracts and pre-parses every function exactly once, then runs a
/// single O(N²) pairwise loop restricted to `scope`. This replaces the
/// previous split into a cross-file pass and a per-file pass, which
/// extracted and parsed every function twice when both scopes were in
/// play (the default).
///
/// Returns the similar pairs (ordered by impact desc, then similarity
/// desc) plus the files that failed to parse and were skipped.
pub fn find_similar_function_pairs(
    files: &[(String, String)],
    threshold: f64,
    options: &TSEDOptions,
    scope: PairScope,
) -> (CrossFileSimilarityResult, Vec<SkippedFile>) {
    struct Candidate {
        file_index: usize,
        func: FunctionDefinition,
        tree: Option<std::rc::Rc<crate::tree::TreeNode>>,
        tree_size: u32,
    }

    let mut skipped = Vec::new();
    let mut candidates: Vec<Candidate> = Vec::new();
    for (file_index, (filename, source)) in files.iter().enumerate() {
        let functions = match extract_functions(filename, source) {
            Ok(functions) => functions,
            Err(error) => {
                skipped.push((filename.clone(), error));
                continue;
            }
        };
        for func in functions {
            if func.has_ignore_directive {
                continue;
            }
            // Pre-parse once per function: parsing dominates the per-pair
            // cost, and the tree also supplies the node count used by the
            // `min_tokens` gate (measured on the same tree that gets
            // compared, so the gate and the score agree).
            let tree = parse_function_for_comparison(filename, &func, source).ok();
            let tree_size = tree
                .as_ref()
                .map_or(0, |tree| u32::try_from(tree.get_subtree_size()).unwrap_or(u32::MAX));
            candidates.push(Candidate { file_index, func, tree, tree_size });
        }
    }

    let mut similar_pairs = Vec::new();
    for i in 0..candidates.len() {
        for j in (i + 1)..candidates.len() {
            let (left, right) = (&candidates[i], &candidates[j]);
            let same_file = left.file_index == right.file_index;
            match scope {
                PairScope::SameFileOnly if !same_file => continue,
                PairScope::CrossFileOnly if same_file => continue,
                _ => {}
            }

            if let Some(min_tokens) = options.min_tokens {
                if left.tree_size < min_tokens || right.tree_size < min_tokens {
                    continue;
                }
            } else if left.func.line_count() < options.min_lines
                || right.func.line_count() < options.min_lines
            {
                continue;
            }

            // Nested-function containment is only meaningful within one
            // file — across files the span/line comparison is between
            // unrelated coordinate spaces and used to spuriously drop
            // pairs whose ranges happened to nest.
            if same_file && left.func.is_parent_child_relationship(&right.func) {
                continue;
            }

            let (Some(left_tree), Some(right_tree)) = (&left.tree, &right.tree) else {
                continue;
            };

            let Ok(similarity) = compare_function_trees(
                left_tree,
                right_tree,
                &left.func,
                &right.func,
                options,
                threshold,
            ) else {
                continue;
            };

            if similarity >= threshold {
                similar_pairs.push((
                    files[left.file_index].0.clone(),
                    SimilarityResult::new(left.func.clone(), right.func.clone(), similarity),
                    files[right.file_index].0.clone(),
                ));
            }
        }
    }

    // Sort by impact (descending), then by similarity (descending).
    similar_pairs.sort_by(|a, b| {
        b.1.impact
            .cmp(&a.1.impact)
            .then(b.1.similarity.total_cmp(&a.1.similarity))
    });

    (similar_pairs, skipped)
}

/// Find similar functions within the same file.
///
/// # Errors
///
/// Returns an error when the file fails to parse.
pub fn find_similar_functions_in_file(
    filename: &str,
    source_text: &str,
    threshold: f64,
    options: &TSEDOptions,
) -> Result<Vec<SimilarityResult>, String> {
    let files = [(filename.to_string(), source_text.to_string())];
    let (pairs, skipped) =
        find_similar_function_pairs(&files, threshold, options, PairScope::SameFileOnly);
    if let Some((_, error)) = skipped.into_iter().next() {
        return Err(error);
    }
    Ok(pairs.into_iter().map(|(_, result, _)| result).collect())
}

/// Find similar functions across multiple files (cross-file pairs only;
/// same-file pairs are the domain of [`find_similar_functions_in_file`]).
///
/// # Errors
///
/// Currently infallible — files that fail to parse are skipped — but the
/// signature keeps `Result` for backwards compatibility.
pub fn find_similar_functions_across_files(
    files: &[(String, String)], // (filename, source_text)
    threshold: f64,
    options: &TSEDOptions,
) -> Result<CrossFileSimilarityResult, String> {
    let (pairs, _skipped) =
        find_similar_function_pairs(files, threshold, options, PairScope::CrossFileOnly);
    Ok(pairs)
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

    }

    #[test]
    fn min_tokens_gate_uses_comparison_tree_size() {
        let code = r"
            function tinyA() {
                return 42;
            }

            function tinyB() {
                return 42;
            }
        ";
        let mut options = TSEDOptions::default();
        options.min_lines = 1;
        options.size_penalty = false;

        // Without a token gate the identical pair is reported…
        let (pairs, _) = find_similar_function_pairs(
            &[("test.ts".to_string(), code.to_string())],
            0.9,
            &options,
            PairScope::SameFileOnly,
        );
        assert_eq!(pairs.len(), 1);

        // …and a high min_tokens threshold filters it out.
        options.min_tokens = Some(500);
        let (pairs, _) = find_similar_function_pairs(
            &[("test.ts".to_string(), code.to_string())],
            0.9,
            &options,
            PairScope::SameFileOnly,
        );
        assert!(pairs.is_empty());
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

        // validateUser/checkUser are genuine rename duplicates (same free
        // atoms, renamed locals) and must be reported.
        let validate_check = similar_pairs.iter().find(|(_, result, _)| {
            (result.func1.name == "validateUser" && result.func2.name == "checkUser")
                || (result.func1.name == "checkUser" && result.func2.name == "validateUser")
        });
        assert!(validate_check.is_some());

        // processUser/handleUser share only the three-call skeleton — the
        // callees are entirely different sets, which the behavioral-atom
        // cap treats as different work (mirroring the corpus's skeleton
        // negatives), so the pair stays below the 0.7 threshold.
        let process_handle = similar_pairs.iter().find(|(_, result, _)| {
            (result.func1.name == "processUser" && result.func2.name == "handleUser")
                || (result.func1.name == "handleUser" && result.func2.name == "processUser")
        });
        assert!(process_handle.is_none());
    }

    #[test]
    fn boundary_index_twins_stay_distinct() {
        // XF-N15 shape: `.at(-1)` picks the newest entry, `.at(0)` the
        // oldest — one literal apart, but reading different elements.
        let code = r#"
export function newestSnapshot(snapshots: string[]): string {
  const picked = snapshots.at(-1);
  if (picked === undefined) {
    return "none";
  }
  return picked;
}

export function oldestSnapshot(snapshots: string[]): string {
  const picked = snapshots.at(0);
  if (picked === undefined) {
    return "none";
  }
  return picked;
}
"#;
        let options = TSEDOptions::default();
        let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
        assert!(
            pairs.is_empty(),
            "boundary-index twins must stay below threshold, got {:?}",
            pairs.iter().map(|p| p.similarity).collect::<Vec<_>>()
        );
    }

    #[test]
    fn slice_arity_twins_stay_distinct() {
        // XF-N17 shape: `slice(0, cut)` keeps the head that `slice(cut)`
        // drops — complementary halves from the same call shape.
        let code = r"
export function takeTopJobs(jobs: string[], cut: number): string[] {
  if (jobs.length === 0) {
    return [];
  }
  return jobs.slice(0, cut);
}

export function dropTopJobs(jobs: string[], cut: number): string[] {
  if (jobs.length === 0) {
    return [];
  }
  return jobs.slice(cut);
}
";
        let options = TSEDOptions::default();
        let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
        assert!(
            pairs.is_empty(),
            "slice head/tail twins must stay below threshold, got {:?}",
            pairs.iter().map(|p| p.similarity).collect::<Vec<_>>()
        );
    }

    #[test]
    fn fold_direction_twins_stay_distinct() {
        // XF-N40 shape: appending builds a/b/c/ while prepending builds
        // c/b/a/ — string + is not commutative.
        let code = r#"
export function buildTrailForward(segments: string[]): string {
  let trail = "";
  for (const segment of segments) {
    trail = trail + segment + "/";
  }
  return trail;
}

export function buildTrailReversed(segments: string[]): string {
  let trail = "";
  for (const segment of segments) {
    trail = segment + "/" + trail;
  }
  return trail;
}
"#;
        let options = TSEDOptions::default();
        let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
        assert!(
            pairs.is_empty(),
            "append vs prepend folds must stay below threshold, got {:?}",
            pairs.iter().map(|p| p.similarity).collect::<Vec<_>>()
        );
    }

    #[test]
    fn splice_inserted_value_twins_stay_duplicates() {
        // Only `start` and `deleteCount` are boundary positions on
        // `splice`; arguments from position 2 onward are inserted VALUES,
        // and twins differing only there are parameterizable data-literal
        // duplicates.
        let code = r"
export function resetLeadingFlag(flags: number[], cursor: number): number[] {
  if (flags.length === 0) {
    return flags;
  }
  flags.splice(cursor, 1, 0);
  return flags;
}

export function raiseLeadingFlag(flags: number[], cursor: number): number[] {
  if (flags.length === 0) {
    return flags;
  }
  flags.splice(cursor, 1, 1);
  return flags;
}
";
        let options = TSEDOptions::default();
        let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
        assert_eq!(
            pairs.len(),
            1,
            "splice twins differing only in the inserted value must stay reportable"
        );
    }

    #[test]
    fn member_target_fold_direction_twins_stay_distinct() {
        // `this.trail` / `state.trail` accumulators carry the same
        // append-vs-prepend behavioral difference as local ones.
        let code = r#"
export function extendAuditTrail(state: { trail: string }, segments: string[]): void {
  for (const segment of segments) {
    state.trail = state.trail + segment + "/";
  }
}

export function rewindAuditTrail(state: { trail: string }, segments: string[]): void {
  for (const segment of segments) {
    state.trail = segment + "/" + state.trail;
  }
}
"#;
        let options = TSEDOptions::default();
        let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
        assert!(
            pairs.is_empty(),
            "member-target append vs prepend folds must stay below threshold, got {:?}",
            pairs.iter().map(|p| p.similarity).collect::<Vec<_>>()
        );
    }

    #[test]
    fn folds_without_string_evidence_get_no_direction_atoms() {
        // `+` over numbers is commutative: `sum += n` vs `sum = n + sum`
        // is not a directional rewrite the analyzer can prove, so without
        // a string literal in the chain the fold-direction atoms must
        // stay silent (the pair keeps whatever score the tree distance
        // gives it, uncapped). With a string literal in the chain the
        // direction IS provable and the atoms fire.
        use crate::parser::parse_and_convert_to_tree_canonical;

        let fold_atoms = |source: &str| -> Vec<String> {
            let tree = parse_and_convert_to_tree_canonical("probe.ts", source).unwrap();
            let mut atoms = AtomSets::default();
            collect_behavioral_atoms(&tree, &mut atoms);
            let mut folds: Vec<String> = atoms
                .value_atoms
                .into_iter()
                .filter(|atom| atom.starts_with("fold:"))
                .collect();
            folds.sort();
            folds
        };

        let numeric_append =
            "function f(xs: number[]) { let sum = 0; for (const x of xs) { sum += x; } return sum; }";
        let numeric_prepend =
            "function f(xs: number[]) { let sum = 0; for (const x of xs) { sum = x + sum; } return sum; }";
        assert!(fold_atoms(numeric_append).is_empty(), "numeric append must not emit fold atoms");
        assert!(fold_atoms(numeric_prepend).is_empty(), "numeric prepend must not emit fold atoms");

        let string_append = r#"function f(xs: string[]) { let out = ""; for (const x of xs) { out = out + x + "/"; } return out; }"#;
        let string_prepend = r#"function f(xs: string[]) { let out = ""; for (const x of xs) { out = x + "/" + out; } return out; }"#;
        assert_eq!(fold_atoms(string_append), vec!["fold:head".to_string()]);
        assert_eq!(fold_atoms(string_prepend), vec!["fold:tail".to_string()]);
    }

    #[test]
    fn same_direction_folds_still_match() {
        // Control for the fold-direction atom: two append folds with
        // renamed identifiers mark the same accumulator position, so the
        // new atoms must not push a true rename-duplicate below
        // threshold.
        let code = r#"
export function joinSegments(pieces: string[]): string {
  let joined = "";
  for (const piece of pieces) {
    joined += piece + "/";
  }
  return joined;
}

export function gluePath(parts: string[]): string {
  let glued = "";
  for (const part of parts) {
    glued += part + "/";
  }
  return glued;
}
"#;
        let options = TSEDOptions::default();
        let pairs = find_similar_functions_in_file("test.ts", code, 0.8, &options).unwrap();
        assert_eq!(
            pairs.len(),
            1,
            "same-direction folds must still report as duplicates"
        );
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
