//! Function discovery: every function-shaped declaration of a source file.

use oxc_ast::ast::{
    ArrowFunctionExpression, BindingPattern, Class, ClassElement, Declaration,
    ExportDefaultDeclarationKind, Expression, FormalParameters, Function, FunctionBody,
    MethodDefinitionKind, ObjectExpression, ObjectProperty, ObjectPropertyKind, Program,
    PropertyKey, PropertyKind, Statement, TSModuleDeclaration, TSModuleDeclarationBody, TSType,
    TSTypeAnnotation, TSTypeName, VariableDeclaration,
};
use oxc_span::{GetSpan, Span};

use crate::ignore_directive::has_ignore_directive_above;
use crate::source::{slice, with_checked_program, LineIndex};

#[derive(Debug, Clone)]
pub struct FunctionDefinition {
    pub name: String,
    pub function_type: FunctionType,
    pub parameters: Vec<String>,
    /// Span of the whole declaration: the function or arrow itself, or the
    /// class or object member that holds it. Its text parses on its own the
    /// way [`Self::form`] says; nested functions sit strictly inside it.
    pub span: Span,
    pub form: SourceForm,
    /// Span of the formal parameter list (`(x, y)` or, for arrow functions
    /// with a single un-parenthesised parameter, just `x`). Used together
    /// with `body_span` to assemble a normalization wrapper that treats
    /// arrow functions and regular function declarations equivalently.
    pub params_span: Span,
    /// Span of just the function body (the `{ ... }` block, or the
    /// expression for expression-bodied arrow functions). Used to extract
    /// a normalized comparison fragment so a `function foo` and a
    /// `const foo = () => ...` with identical bodies look structurally
    /// equivalent to the comparator.
    pub body_span: Span,
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
    /// True when the declared return type is `Promise<…>`/`PromiseLike<…>`:
    /// a sync function with that annotation has the same call contract as
    /// an `async` twin.
    pub declares_promise_return: bool,
    /// True only for class methods declared `static`. Static and
    /// instance methods with identical bodies are runtime-distinct,
    /// so the normalized fragment carries this prefix forward.
    pub is_static: bool,
    /// Method kind: `Normal`, `Getter`, or `Setter`. For functions,
    /// arrows, and constructors this is always `Normal`.
    pub method_kind: MethodKind,
    /// The exact source text of the method key (`"alpha"`, `#load`,
    /// `[Symbol.iterator]`, …) or, for functions and arrows, the name the
    /// body is bound under — the declaration name, or a named function
    /// expression's own name (`recur` in `const f = function recur() {…}`).
    /// Used in the normalization wrapper so a `static "alpha"()` and a
    /// `static "beta"()` do not collapse onto one placeholder, and so
    /// recursive self-references keep resolving. Falls back to `name`
    /// when the underlying span can't be recovered.
    pub display_name: String,
    /// The other name a named function expression is reachable under:
    /// `factorial` in `const factorial = function recur(n) {…}`, where the
    /// body may recurse through either binding, or `walk` in an object
    /// member `visit: function walk(node) {…}`, whose fragment is declared
    /// under a placeholder because the key binds nothing inside the body.
    /// `None` when the function has a single name.
    pub self_alias: Option<String>,
    pub start_line: u32,
    pub end_line: u32,
    pub class_name: Option<String>,
    pub parent_function: Option<String>,
    /// Whether a `// similarity-ignore` comment excludes the function from
    /// the report: one right above it, above the statement that declares
    /// it, or above a declaration it is nested in (its class, the object
    /// literal that holds it, an enclosing function or namespace).
    pub has_ignore_directive: bool,
}

impl FunctionDefinition {
    #[must_use]
    pub fn line_count(&self) -> u32 {
        self.end_line - self.start_line + 1
    }

    /// Whether one of the two functions is nested inside the other.
    #[must_use]
    pub fn is_parent_child_relationship(&self, other: &FunctionDefinition) -> bool {
        self.contains(other) || other.contains(self)
    }

    fn contains(&self, inner: &FunctionDefinition) -> bool {
        self.start_line <= inner.start_line
            && self.end_line >= inner.end_line
            && self.span.start < inner.span.start
            && self.span.end > inner.span.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionType {
    Function,
    Method,
    Arrow,
    Constructor,
}

impl FunctionType {
    /// The name the report uses for this kind of function.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            FunctionType::Function => "function",
            FunctionType::Method => "method",
            FunctionType::Arrow => "arrow",
            FunctionType::Constructor => "constructor",
        }
    }
}

/// Class-method kind preserved across normalization so a `get` accessor
/// and a regular method, or `set` vs `get`, never collapse into the same
/// tree. Non-method functions always carry `Normal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodKind {
    Normal,
    Getter,
    Setter,
}

/// How the text of a declaration's [`FunctionDefinition::span`] parses on
/// its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceForm {
    /// A named `function` declaration: a statement as it stands.
    Statement,
    /// An arrow or an anonymous `function`: an expression.
    Expression,
    /// A method, accessor or constructor: valid inside a class body.
    ClassMember,
    /// A method, accessor or function-valued property: valid inside an
    /// object literal.
    ObjectMember,
}

/// Extract all functions from TypeScript/JavaScript code.
///
/// # Errors
///
/// Returns an error when the file cannot be analyzed: syntax errors, or
/// nesting beyond the limits of [`crate::source`].
pub fn extract_functions(
    filename: &str,
    source_text: &str,
) -> Result<Vec<FunctionDefinition>, String> {
    let lines = LineIndex::new(source_text);
    with_checked_program(filename, source_text, |program| {
        extract_functions_from_program(program, source_text, &lines)
    })
}

/// [`extract_functions`] for an already parsed file.
#[must_use]
pub fn extract_functions_from_program(
    program: &Program,
    source_text: &str,
    lines: &LineIndex,
) -> Vec<FunctionDefinition> {
    let mut context = ExtractionContext {
        functions: Vec::new(),
        source_text,
        lines,
        parent_function: None,
        ignored: false,
    };
    for stmt in &program.body {
        context.statement(stmt);
    }
    context.functions
}

/// The parts every function form shares, whether it is written as a
/// `function` or as an arrow.
struct Callable<'a, 'b> {
    params: &'b FormalParameters<'a>,
    body: &'b FunctionBody<'a>,
    return_type: Option<&'b TSTypeAnnotation<'a>>,
    is_async: bool,
    is_generator: bool,
    /// An arrow whose body is a single expression.
    is_expression: bool,
}

impl<'a, 'b> Callable<'a, 'b> {
    /// `None` for a bodiless declaration — an overload signature, a
    /// `declare function …;`, an abstract or ambient member. There is no
    /// code to deduplicate there, and comparing their (type-less)
    /// parameter lists used to report every overload of the same function
    /// as a 1.0 duplicate of the next one.
    fn of_function(func: &'b Function<'a>) -> Option<Self> {
        Some(Self {
            params: &func.params,
            body: func.body.as_deref()?,
            return_type: func.return_type.as_deref(),
            is_async: func.r#async,
            is_generator: func.generator,
            is_expression: false,
        })
    }

    fn of_arrow(arrow: &'b ArrowFunctionExpression<'a>) -> Self {
        Self {
            params: &arrow.params,
            body: &arrow.body,
            return_type: arrow.return_type.as_deref(),
            is_async: arrow.r#async,
            is_generator: false,
            is_expression: arrow.expression,
        }
    }
}

/// How a [`Callable`] is declared: what it is called and where it sits.
struct DeclarationSite {
    name: String,
    /// The name functions nested in the body record as their parent.
    scope_name: String,
    function_type: FunctionType,
    method_kind: MethodKind,
    is_static: bool,
    display_name: String,
    self_alias: Option<String>,
    class_name: Option<String>,
    span: Span,
    form: SourceForm,
    /// The span whose lines the function is reported at. An arrow-valued
    /// class field is reported at the field, not at the arrow alone.
    reported: Span,
}

struct ExtractionContext<'s> {
    functions: Vec<FunctionDefinition>,
    source_text: &'s str,
    lines: &'s LineIndex,
    /// Name of the function whose body is being walked.
    parent_function: Option<String>,
    /// Whether the scope being walked belongs to a declaration under a
    /// `similarity-ignore` directive.
    ignored: bool,
}

impl ExtractionContext<'_> {
    /// Whether a directive sits right above the line `offset` is on.
    fn directive_above(&self, offset: u32) -> bool {
        has_ignore_directive_above(self.source_text, self.lines, self.lines.line_of(offset))
    }

    /// Run `walk` with the scope marked ignored when `ignored` says so (or
    /// when it already is).
    fn within_scope(&mut self, ignored: bool, walk: impl FnOnce(&mut Self)) {
        let outer = self.ignored;
        self.ignored = outer || ignored;
        walk(self);
        self.ignored = outer;
    }

    fn statement(&mut self, stmt: &Statement) {
        // A directive above a statement covers everything it declares:
        // `export const handler: Handler<…> = async (event) => {…}` may
        // start its arrow lines below the comment.
        let ignored = self.directive_above(stmt.span().start);
        self.within_scope(ignored, |this| this.statement_declarations(stmt));
    }

    fn statement_declarations(&mut self, stmt: &Statement) {
        match stmt {
            Statement::FunctionDeclaration(func) => {
                if let Some(name) = &func.id {
                    self.function(func, name.name.to_string());
                }
            }
            Statement::ClassDeclaration(class) => self.class_members(class),
            Statement::VariableDeclaration(var_decl) => self.variable_functions(var_decl),
            // `namespace`/`module` blocks are ordinary declaration scopes for
            // the functions they contain (`namespace ts { export function
            // isBlock(node) {…} }` is the classic TypeScript-compiler style);
            // skipping them hid every function declared that way.
            Statement::TSModuleDeclaration(module) => self.module_body(module),
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::FunctionDeclaration(func)) => {
                    if let Some(name) = &func.id {
                        self.function(func, name.name.to_string());
                    }
                }
                Some(Declaration::ClassDeclaration(class)) => self.class_members(class),
                Some(Declaration::VariableDeclaration(var_decl)) => {
                    self.variable_functions(var_decl);
                }
                Some(Declaration::TSModuleDeclaration(module)) => self.module_body(module),
                _ => {}
            },
            // `export default` takes a declaration or an expression; the class
            // and expression forms used to contribute nothing, so an arrow
            // field or method in `export default class Relay {…}` never met
            // its twin in a named class.
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                    let name = func
                        .id
                        .as_ref()
                        .map_or_else(|| "default".to_string(), |id| id.name.to_string());
                    self.function(func, name);
                }
                ExportDefaultDeclarationKind::ClassDeclaration(class) => self.class_members(class),
                // Expression forms, looking through transparent wrappers the
                // same way variable initializers do (`export default (() =>
                // {…}) satisfies Handler`).
                other => {
                    if let Some(expression) = other.as_expression() {
                        self.bound_expression(expression, "default");
                    }
                }
            },
            _ => {}
        }
    }

    /// Walk a `namespace`/`module` body (including the nested-name form
    /// `namespace a.b.c { … }`, which oxc represents as nested declarations).
    fn module_body(&mut self, module: &TSModuleDeclaration) {
        match &module.body {
            Some(TSModuleDeclarationBody::TSModuleDeclaration(inner)) => self.module_body(inner),
            Some(TSModuleDeclarationBody::TSModuleBlock(block)) => {
                for stmt in &block.body {
                    self.statement(stmt);
                }
            }
            None => {}
        }
    }

    /// `const f = (x) => …` and `const f = function (x) { … }` both bind a
    /// function to a name; the function-expression spelling used to be
    /// invisible to the scan.
    fn variable_functions(&mut self, var_decl: &VariableDeclaration) {
        for decl in &var_decl.declarations {
            let BindingPattern::BindingIdentifier(ident) = &decl.id else {
                continue;
            };
            if let Some(init) = &decl.init {
                self.bound_expression(init, ident.name.as_str());
            }
        }
    }

    /// The functions an expression bound to `name` contributes: the
    /// function or arrow itself, or the members of an object literal.
    ///
    /// Parentheses and TypeScript-only wrappers around the expression are
    /// transparent at runtime — `const parse = (function (line) {…});`,
    /// `const handle = ((event) => {…}) as Handler;`, `… satisfies Handler`,
    /// `<Handler>(…)` and a trailing `!` all bind the inner function — so
    /// they are looked through instead of hiding the function from the scan.
    fn bound_expression(&mut self, expression: &Expression, name: &str) {
        match expression.get_inner_expression() {
            Expression::ArrowFunctionExpression(arrow) => self.arrow(arrow, name.to_string()),
            Expression::FunctionExpression(func) => self.function(func, name.to_string()),
            Expression::ObjectExpression(object) => self.object_members(object, name),
            _ => {}
        }
    }

    /// Record a `function` (declaration, default export, or a function
    /// expression bound to a variable) under `name`.
    fn function(&mut self, func: &Function, name: String) {
        let Some(callable) = Callable::of_function(func) else {
            return;
        };
        // A named function expression (`const factorial = function recur(n)
        // {…}`) binds its own name inside its body. The comparison fragment
        // is rebuilt under that binding so `recur(n - 1)` keeps resolving to
        // the function itself; rebuilding it under the variable name used to
        // leave `recur` as a free identifier that differed between otherwise
        // identical copies. The variable binding is the same function, so it
        // is kept as an alias: a body recursing through `factorial(n - 1)`
        // resolves to the function too.
        let binding_name = func.id.as_ref().map_or_else(|| name.clone(), |id| id.name.to_string());
        let self_alias = (binding_name != name && is_plain_identifier(&name)).then(|| name.clone());
        self.record(
            DeclarationSite {
                scope_name: name.clone(),
                name,
                function_type: FunctionType::Function,
                method_kind: MethodKind::Normal,
                is_static: false,
                display_name: binding_name,
                self_alias,
                class_name: None,
                span: func.span,
                // A function without a name of its own is no statement.
                form: if func.id.is_some() {
                    SourceForm::Statement
                } else {
                    SourceForm::Expression
                },
                reported: func.span,
            },
            &callable,
        );
    }

    /// Record an arrow function bound to a name (`const f = (x) => …`).
    fn arrow(&mut self, arrow: &ArrowFunctionExpression, name: String) {
        self.record(
            DeclarationSite {
                scope_name: name.clone(),
                display_name: name.clone(),
                name,
                function_type: FunctionType::Arrow,
                method_kind: MethodKind::Normal,
                is_static: false,
                self_alias: None,
                class_name: None,
                span: arrow.span,
                form: SourceForm::Expression,
                reported: arrow.span,
            },
            &Callable::of_arrow(arrow),
        );
    }

    /// Functions held by an object literal bound to a name — `const handlers
    /// = { onSave(doc) {…}, onClose: (doc) => {…} }`, `export default {
    /// transform(code) {…} }`, nested objects included. Dispatch tables,
    /// plugin and handler objects, and test doubles are written this way, and
    /// all of their functions used to be invisible to the scan. Members are
    /// reported under the binding (`handlers.onSave`,
    /// `table[SyntaxKind.Block]`); an object passed as an argument or returned
    /// from a function stays part of the function that builds it.
    fn object_members(&mut self, object: &ObjectExpression, qualifier: &str) {
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(prop) = property else {
                continue;
            };
            let key = member_name(&prop.key, prop.computed, self.source_text);
            let name = if key.starts_with('[') {
                format!("{qualifier}{key}")
            } else {
                format!("{qualifier}.{key}")
            };
            let ignored = self.directive_above(prop.span.start);
            self.within_scope(ignored, |this| match prop.value.get_inner_expression() {
                // `save(doc) {…}`, `get size() {…}` and `save: function (doc)
                // {…}` are methods: `this` is the object, exactly as in a class.
                Expression::FunctionExpression(func) => this.object_method(func, prop, name),
                Expression::ArrowFunctionExpression(arrow) => this.arrow(arrow, name),
                Expression::ObjectExpression(inner) => this.object_members(inner, &name),
                _ => {}
            });
        }
    }

    /// Record a function-valued object member as a method (see
    /// [`Self::object_members`]).
    fn object_method(&mut self, func: &Function, prop: &ObjectProperty, name: String) {
        let Some(callable) = Callable::of_function(func) else {
            return;
        };
        let method_kind = match prop.kind {
            PropertyKind::Get => MethodKind::Getter,
            PropertyKind::Set => MethodKind::Setter,
            PropertyKind::Init => MethodKind::Normal,
        };
        let display_name = member_key_text(&prop.key, prop.computed, self.source_text)
            .unwrap_or_else(|| name.clone());
        self.record(
            DeclarationSite {
                scope_name: name.clone(),
                name,
                function_type: FunctionType::Method,
                method_kind,
                is_static: false,
                display_name,
                // `visit: function walk(node) {… walk(node.left) …}`: the
                // function expression's own name is a binding inside the
                // body, the property key is not. The own name is an alias of
                // the fragment's declaration, so recursion through `walk`
                // alpha-renames with the function instead of staying a free
                // identifier that differs between otherwise identical copies.
                // The fragment is not declared under the key (see
                // `METHOD_FRAGMENT_NAME`), so a call to an outer `visit`
                // stays free.
                self_alias: func.id.as_ref().map(|id| id.name.to_string()),
                class_name: None,
                span: prop.span,
                form: SourceForm::ObjectMember,
                reported: prop.span,
            },
            &callable,
        );
    }

    fn class_members(&mut self, class: &Class) {
        let class_name = class.id.as_ref().map(|id| id.name.to_string());
        let qualified = |member: &str| match &class_name {
            Some(class) => format!("{class}.{member}"),
            None => member.to_string(),
        };
        // Decorators sit above the `class` keyword (and above `export`).
        let ignored =
            class.decorators.first().is_some_and(|first| self.directive_above(first.span.start));
        self.within_scope(ignored, |this| {
            for element in &class.body.body {
                match element {
                    ClassElement::MethodDefinition(method) => {
                        // Abstract methods and overload signatures have no
                        // body — nothing to compare.
                        let Some(callable) = Callable::of_function(&method.value) else {
                            continue;
                        };
                        let name = member_name(&method.key, method.computed, this.source_text);
                        // The key's source text keeps string, number and
                        // computed keys spelled exactly during comparison,
                        // and private `#name` methods apart from each other.
                        let display_name =
                            member_key_text(&method.key, method.computed, this.source_text)
                                .unwrap_or_else(|| name.clone());
                        this.record(
                            DeclarationSite {
                                scope_name: qualified(&name),
                                name,
                                function_type: if method.kind == MethodDefinitionKind::Constructor {
                                    FunctionType::Constructor
                                } else {
                                    FunctionType::Method
                                },
                                method_kind: match method.kind {
                                    MethodDefinitionKind::Get => MethodKind::Getter,
                                    MethodDefinitionKind::Set => MethodKind::Setter,
                                    _ => MethodKind::Normal,
                                },
                                is_static: method.r#static,
                                display_name,
                                self_alias: None,
                                class_name: class_name.clone(),
                                span: method.span,
                                form: SourceForm::ClassMember,
                                reported: method.span,
                            },
                            &callable,
                        );
                    }
                    // `handle = (event) => { … }` class fields are methods in
                    // everything but declaration syntax (the class comparator
                    // already treats them that way); extract them so an
                    // arrow-field method and its `handle(event) { … }` twin
                    // meet in the function scan.
                    ClassElement::PropertyDefinition(prop) => {
                        let Some(Expression::ArrowFunctionExpression(arrow)) =
                            prop.value.as_ref().map(Expression::get_inner_expression)
                        else {
                            continue;
                        };
                        let name = member_name(&prop.key, prop.computed, this.source_text);
                        let display_name =
                            member_key_text(&prop.key, prop.computed, this.source_text)
                                .unwrap_or_else(|| name.clone());
                        this.record(
                            DeclarationSite {
                                scope_name: qualified(&name),
                                name,
                                function_type: FunctionType::Method,
                                method_kind: MethodKind::Normal,
                                is_static: prop.r#static,
                                display_name,
                                self_alias: None,
                                class_name: class_name.clone(),
                                span: arrow.span,
                                form: SourceForm::Expression,
                                reported: prop.span,
                            },
                            &Callable::of_arrow(arrow),
                        );
                    }
                    _ => {}
                }
            }
        });
    }

    /// Record one function and walk its body for the functions declared in
    /// it.
    fn record(&mut self, declaration: DeclarationSite, callable: &Callable) {
        let ignored = self.ignored || self.directive_above(declaration.reported.start);
        self.functions.push(FunctionDefinition {
            name: declaration.name,
            function_type: declaration.function_type,
            parameters: parameter_names(callable.params),
            span: declaration.span,
            form: declaration.form,
            params_span: callable.params.span,
            body_span: callable.body.span,
            is_arrow_expression: callable.is_expression,
            is_async: callable.is_async,
            is_generator: callable.is_generator,
            declares_promise_return: is_promise_annotation(callable.return_type),
            is_static: declaration.is_static,
            method_kind: declaration.method_kind,
            display_name: declaration.display_name,
            self_alias: declaration.self_alias,
            start_line: self.lines.line_of(declaration.reported.start),
            end_line: self.lines.line_of(declaration.reported.end),
            class_name: declaration.class_name,
            parent_function: self.parent_function.clone(),
            has_ignore_directive: ignored,
        });

        // An expression body declares nothing.
        if callable.is_expression {
            return;
        }
        let outer_parent = self.parent_function.replace(declaration.scope_name);
        self.within_scope(ignored, |this| {
            for stmt in &callable.body.statements {
                this.statement(stmt);
            }
        });
        self.parent_function = outer_parent;
    }
}

/// Member name for a class or object member key: identifiers as written,
/// private names with their `#`, string-literal keys by value, numeric and
/// computed keys by their source text (`0`, `[Symbol.iterator]`). Every
/// non-identifier key used to collapse onto `anonymous`, which made the
/// reported name useless even though `display_name` kept such methods
/// apart during comparison.
fn member_name(key: &PropertyKey, computed: bool, source_text: &str) -> String {
    match key {
        PropertyKey::StaticIdentifier(ident) => ident.name.to_string(),
        PropertyKey::PrivateIdentifier(ident) => format!("#{}", ident.name),
        PropertyKey::StringLiteral(literal) => literal.value.to_string(),
        _ => member_key_text(key, computed, source_text).unwrap_or_else(|| "anonymous".to_string()),
    }
}

/// Exact source text of a member key (`"alpha"`, `#load`, `0`), bracketed
/// when the key is computed so the text re-parses as a key inside the
/// synthetic class wrapper. `None` when the span cannot be recovered.
fn member_key_text(key: &PropertyKey, computed: bool, source_text: &str) -> Option<String> {
    let text = slice(source_text, key.span())?;
    Some(if computed && !text.starts_with('[') { format!("[{text}]") } else { text.to_string() })
}

fn parameter_names(params: &FormalParameters) -> Vec<String> {
    params
        .items
        .iter()
        .filter_map(|param| match &param.pattern {
            BindingPattern::BindingIdentifier(ident) => Some(ident.name.to_string()),
            _ => None,
        })
        .collect()
}

/// `Promise<…>` or `PromiseLike<…>` as a declared return type.
fn is_promise_annotation(annotation: Option<&TSTypeAnnotation>) -> bool {
    let Some(TSType::TSTypeReference(reference)) = annotation.map(|a| &a.type_annotation) else {
        return false;
    };
    matches!(
        &reference.type_name,
        TSTypeName::IdentifierReference(ident)
            if matches!(ident.name.as_str(), "Promise" | "PromiseLike")
    )
}

pub(crate) fn is_plain_identifier(name: &str) -> bool {
    !name.is_empty()
        && name.chars().enumerate().all(|(idx, ch)| {
            if idx == 0 {
                ch.is_alphabetic() || ch == '_' || ch == '$'
            } else {
                ch.is_alphanumeric() || ch == '_' || ch == '$'
            }
        })
}
