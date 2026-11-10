//! Class discovery: the shape of every class declaration of a source file.

use oxc_ast::ast::{
    ArrowFunctionExpression, AssignmentOperator, AssignmentTarget, BindingPattern, Class,
    ClassElement, Declaration, ExportDefaultDeclarationKind, Expression, FormalParameter,
    FormalParameters, Function, MethodDefinition, MethodDefinitionKind, Program, PropertyKey,
    Statement, TSAccessibility, TSModuleDeclaration, TSModuleDeclarationBody, TSType,
    TSTypeAnnotation, TSTypeParameterDeclaration, UnaryOperator,
};
use oxc_span::{GetSpan, Span};
use std::hash::{Hash, Hasher};

use crate::function_fragment::{unused_synthetic_name, CLASS_WRAPPER_NAME, METHOD_FRAGMENT_NAME};
use crate::ignore_directive::has_ignore_directive_above;
use crate::module_scope::{module_qualifier, qualified_name};
use crate::parser::parse_and_convert_to_tree_canonical;
use crate::source::{slice, with_checked_program, LineIndex};
use crate::tree::TreeNode;
use crate::type_extractor::TypeExtractor;
use crate::type_normalizer::{normalize_type_name, split_top_level};

#[derive(Debug, Clone)]
pub struct ClassDefinition {
    pub name: String,
    pub properties: Vec<ClassProperty>,
    pub methods: Vec<ClassMethod>,
    pub constructor_params: Vec<String>,
    /// Where the constructor stores its parameters: `(parameter, target)`
    /// pairs, the parameter named by position (`#0`, `#1.host` for a
    /// destructured member, `...` for the rest parameter) and the target a
    /// field (`this.x = p` or a parameter property) or a `super(…)`
    /// argument slot (`super[0]`). Two classes with the same fields but
    /// opposite wiring (`this.x = y; this.y = x`) initialize differently.
    pub constructor_wiring: Vec<(String, String)>,
    pub extends: Option<String>,
    pub implements: Vec<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub file_path: String,
    pub is_abstract: bool,
    pub has_ignore_directive: bool,
}

#[derive(Debug, Clone)]
pub struct ClassProperty {
    pub name: String,
    pub type_annotation: String,
    pub is_static: bool,
    pub is_private: bool,
    pub is_readonly: bool,
    pub is_optional: bool,
}

#[derive(Debug, Clone)]
pub struct ClassMethod {
    pub name: String,
    pub parameters: Vec<String>,
    pub return_type: String,
    pub is_static: bool,
    pub is_private: bool,
    pub is_async: bool,
    pub is_generator: bool,
    /// `f?(): T` — an optional method may be absent on an instance.
    pub is_optional: bool,
    pub kind: MethodKind,
    /// Structural hash of the method's canonicalized params+body tree
    /// (name excluded, locals alpha-renamed). Two methods with the same
    /// fingerprint have behaviorally-equivalent bodies up to the
    /// canonicalizer's equivalences; `None` when the body could not be
    /// parsed standalone.
    pub body_fingerprint: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodKind {
    Method,
    Getter,
    Setter,
    Constructor,
}

/// The members collected from one class body.
#[derive(Default)]
struct ClassMembers {
    properties: Vec<ClassProperty>,
    methods: Vec<ClassMethod>,
    constructor_params: Vec<String>,
    constructor_wiring: Vec<(String, String)>,
}

/// Extracts the class declarations of one source file.
pub struct ClassExtractor<'s> {
    source_text: &'s str,
    file_path: &'s str,
    lines: &'s LineIndex,
    /// Type mode's renderer: member annotations are spelled exactly the way
    /// an interface member's would be. The class extractor used to carry a
    /// renderer of its own that collapsed every qualified reference
    /// (`SyntaxKind.Block`, `React.ComponentType<P>`) onto `unknown`,
    /// dropped the type arguments of nested references (`Promise<User>`
    /// rendered as `Promise`) and turned literal, tuple, `keyof`,
    /// `readonly`, `typeof` and every other type onto `any` — so classes
    /// whose members disagreed on all of those compared as identical.
    types: TypeExtractor<'s>,
}

impl<'s> ClassExtractor<'s> {
    #[must_use]
    pub fn new(source_text: &'s str, file_path: &'s str, lines: &'s LineIndex) -> Self {
        let types = TypeExtractor::new(source_text, file_path, lines);
        Self { source_text, file_path, lines, types }
    }

    /// Every class declaration of the file, namespaces included.
    #[must_use]
    pub fn extract_classes(&self, program: &Program) -> Vec<ClassDefinition> {
        let mut classes = Vec::new();
        for statement in &program.body {
            self.extract_classes_from_statement(statement, &mut classes, "", false);
        }
        classes
    }

    fn line_of(&self, offset: u32) -> usize {
        self.lines.line_of(offset) as usize
    }

    /// Whether a `similarity-ignore` directive sits right above the line
    /// `offset` is on.
    fn directive_above(&self, offset: u32) -> bool {
        has_ignore_directive_above(self.source_text, self.lines, self.lines.line_of(offset))
    }

    /// `qualifier` is the dotted path of the enclosing `namespace`/`module`
    /// blocks (`""` at the top level); see [`crate::module_scope`].
    /// `ignored` says whether an enclosing declaration carries the
    /// `similarity-ignore` directive.
    fn extract_classes_from_statement(
        &self,
        statement: &Statement,
        classes: &mut Vec<ClassDefinition>,
        qualifier: &str,
        ignored: bool,
    ) {
        let ignored = ignored || self.directive_above(statement.span().start);
        match statement {
            Statement::ExportDefaultDeclaration(export) => {
                if let ExportDefaultDeclarationKind::ClassDeclaration(class) = &export.declaration {
                    classes.push(self.extract_class(class, qualifier, ignored));
                }
            }
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::ClassDeclaration(class)) => {
                    classes.push(self.extract_class(class, qualifier, ignored));
                }
                Some(Declaration::TSModuleDeclaration(module)) => {
                    self.extract_classes_from_module(module, classes, qualifier, ignored);
                }
                _ => {}
            },
            Statement::ClassDeclaration(class) => {
                classes.push(self.extract_class(class, qualifier, ignored));
            }
            // `namespace`/`module` blocks are ordinary declaration scopes
            // for the classes inside them; they used to be skipped. Their
            // classes are reported under the qualified name
            // (`Legacy.Pager`).
            Statement::TSModuleDeclaration(module) => {
                self.extract_classes_from_module(module, classes, qualifier, ignored);
            }
            _ => {}
        }
    }

    fn extract_classes_from_module(
        &self,
        module: &TSModuleDeclaration,
        classes: &mut Vec<ClassDefinition>,
        qualifier: &str,
        ignored: bool,
    ) {
        let qualifier = module_qualifier(&module.id, qualifier);
        match &module.body {
            Some(TSModuleDeclarationBody::TSModuleDeclaration(inner)) => {
                self.extract_classes_from_module(inner, classes, &qualifier, ignored);
            }
            Some(TSModuleDeclarationBody::TSModuleBlock(block)) => {
                for stmt in &block.body {
                    self.extract_classes_from_statement(stmt, classes, &qualifier, ignored);
                }
            }
            None => {}
        }
    }

    /// A member annotation, rendered by type mode's renderer and normalized
    /// the way type mode compares annotations (`Array<T>` ⇔ `T[]`, sorted
    /// union arms, boxed primitives).
    fn render_type(&self, ts_type: &TSType) -> String {
        normalize_type_name(&self.types.render_type(ts_type))
    }

    fn render_annotation(&self, annotation: Option<&TSTypeAnnotation>) -> String {
        annotation.map_or_else(|| "any".to_string(), |ta| self.render_type(&ta.type_annotation))
    }

    /// The type of a field or parameter property: its annotation when it
    /// has one, else what its initializer pins down, else `any`.
    fn member_type(
        &self,
        annotation: Option<&TSTypeAnnotation>,
        initializer: Option<&Expression>,
    ) -> String {
        match (annotation, initializer) {
            (Some(annotation), _) => self.render_type(&annotation.type_annotation),
            (None, Some(value)) => self.initializer_type(value),
            (None, None) => "any".to_string(),
        }
    }

    /// The type a field's initializer pins down when it carries no
    /// annotation: a literal's primitive (`count = 0`), or the constructed
    /// class (`cache = new Map<string, User>()`). Everything else stays
    /// `any`. Unannotated fields are the norm in TypeScript classes, and
    /// rendering all of them `any` made `enabled = false` and `enabled =
    /// 0` the same member while `count = 0` and `count: number` differed.
    fn initializer_type(&self, initializer: &Expression) -> String {
        match initializer.get_inner_expression() {
            Expression::NumericLiteral(_) => "number".to_string(),
            Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => "string".to_string(),
            Expression::BooleanLiteral(_) => "boolean".to_string(),
            Expression::BigIntLiteral(_) => "bigint".to_string(),
            // A signed literal is still a literal: `offset = -1` is the
            // same `number` field as `offset = 0`.
            Expression::UnaryExpression(unary) => {
                match (unary.operator, unary.argument.get_inner_expression()) {
                    (
                        UnaryOperator::UnaryNegation | UnaryOperator::UnaryPlus,
                        Expression::NumericLiteral(_),
                    ) => "number".to_string(),
                    (UnaryOperator::UnaryNegation, Expression::BigIntLiteral(_)) => {
                        "bigint".to_string()
                    }
                    (UnaryOperator::LogicalNot, _) => "boolean".to_string(),
                    (UnaryOperator::Typeof, _) => "string".to_string(),
                    _ => "any".to_string(),
                }
            }
            Expression::NewExpression(new_expr) => {
                let callee = match new_expr.callee.get_inner_expression() {
                    Expression::Identifier(ident) => ident.name.to_string(),
                    other => match slice(self.source_text, other.span()) {
                        Some(text) => text.chars().filter(|c| !c.is_whitespace()).collect(),
                        None => return "any".to_string(),
                    },
                };
                match &new_expr.type_arguments {
                    Some(args) if !args.params.is_empty() => {
                        let rendered: Vec<String> =
                            args.params.iter().map(|arg| self.types.render_type(arg)).collect();
                        normalize_type_name(&format!("{callee}<{}>", rendered.join(", ")))
                    }
                    _ => callee,
                }
            }
            _ => "any".to_string(),
        }
    }

    /// A parameter list as `name: type` pieces. Optional and defaulted
    /// parameters keep their `?` and the rest parameter its `...`: the
    /// rest parameter used to be dropped, so `log(...args: unknown[])` and
    /// `log()` had the same signature.
    fn extract_function_params(&self, params: &FormalParameters) -> String {
        let mut pieces: Vec<String> = params
            .items
            .iter()
            .map(|param| {
                let optional = param.optional || param.initializer.is_some();
                format!(
                    "{}{}: {}",
                    parameter_name(param),
                    if optional { "?" } else { "" },
                    self.render_annotation(param.type_annotation.as_deref())
                )
            })
            .collect();
        if let Some(rest) = &params.rest {
            let name = match &rest.rest.argument {
                BindingPattern::BindingIdentifier(ident) => ident.name.as_str(),
                _ => "rest",
            };
            pieces.push(format!(
                "...{name}: {}",
                self.render_annotation(rest.type_annotation.as_deref())
            ));
        }
        pieces.join(", ")
    }

    /// `qualifier` is the dotted path of the enclosing `namespace`/`module`
    /// blocks (`""` at the top level); see [`crate::module_scope`].
    fn extract_class(&self, class: &Class, qualifier: &str, ignored: bool) -> ClassDefinition {
        let name = qualified_name(
            qualifier,
            class.id.as_ref().map_or("AnonymousClass", |id| id.name.as_str()),
        );

        // Heritage is recorded as written. Only bare identifiers used to
        // survive, so a qualified base (`React.Component`), a mixin call
        // (`Mixin(Base)`) or type arguments (`Base<string>`) all looked
        // like *no* superclass — invisible to the comparator's heritage
        // check.
        let extends = class.super_class.as_ref().map(|super_class| {
            let mut text = self.heritage_text(super_class.span());
            if let Some(args) = &class.super_type_arguments {
                text.push_str(&self.heritage_text(args.span));
            }
            text
        });
        let implements: Vec<String> =
            class.implements.iter().map(|clause| self.heritage_text(clause.span)).collect();

        let mut members = ClassMembers::default();
        for element in &class.body.body {
            match element {
                ClassElement::PropertyDefinition(prop) => {
                    let Some(name) = self.member_key_name(&prop.key, prop.computed) else {
                        continue;
                    };
                    let is_private = is_private_member(&prop.key, prop.accessibility);
                    // `handle = (event) => { … }` class fields are methods
                    // in everything but declaration syntax; extract them as
                    // methods so the arrow-field and method spellings of the
                    // same class compare member-for-member. Parentheses and
                    // `as`/`satisfies` wrappers are transparent.
                    if let Some(Expression::ArrowFunctionExpression(arrow)) =
                        prop.value.as_ref().map(Expression::get_inner_expression)
                    {
                        let generics = type_parameter_names(arrow.type_parameters.as_deref());
                        members.methods.push(ClassMethod {
                            name,
                            parameters: vec![scoped_parameters(
                                &self.extract_function_params(&arrow.params),
                                &generics,
                            )],
                            return_type: scoped_type(
                                &self.render_return_type(arrow.return_type.as_deref()),
                                &generics,
                            ),
                            is_static: prop.r#static,
                            is_private,
                            is_async: arrow.r#async,
                            is_generator: false,
                            is_optional: prop.optional,
                            kind: MethodKind::Method,
                            body_fingerprint: self.arrow_body_fingerprint(arrow),
                        });
                        continue;
                    }
                    members.properties.push(ClassProperty {
                        name,
                        type_annotation: self
                            .member_type(prop.type_annotation.as_deref(), prop.value.as_ref()),
                        is_static: prop.r#static,
                        is_private,
                        is_readonly: prop.readonly,
                        is_optional: prop.optional,
                    });
                }
                // `accessor count = 0` (an auto-accessor) is a field behind a
                // generated getter/setter pair — a member like any other,
                // which the scan used to skip.
                ClassElement::AccessorProperty(accessor) => {
                    let Some(name) = self.member_key_name(&accessor.key, accessor.computed) else {
                        continue;
                    };
                    members.properties.push(ClassProperty {
                        name,
                        type_annotation: self.member_type(
                            accessor.type_annotation.as_deref(),
                            accessor.value.as_ref(),
                        ),
                        is_static: accessor.r#static,
                        is_private: is_private_member(&accessor.key, accessor.accessibility),
                        is_readonly: false,
                        is_optional: false,
                    });
                }
                ClassElement::MethodDefinition(method) => {
                    let Some(name) = self.member_key_name(&method.key, method.computed) else {
                        continue;
                    };
                    if method.kind == MethodDefinitionKind::Constructor {
                        self.extract_constructor(name, &method.value, &mut members);
                    } else {
                        members.methods.push(self.extract_method(name, method));
                    }
                }
                _ => {}
            }
        }
        let ClassMembers {
            mut properties,
            mut methods,
            mut constructor_params,
            constructor_wiring,
        } = members;

        // Class type parameters are declaration-local names: `Alpha<TItem>
        // extends Base<TItem>` and `Beta<UItem> extends Base<UItem>` are the
        // same contract, so every member annotation and heritage clause
        // spells them positionally (`#0`, `#1`, …) — the substitution type
        // mode applies to interfaces. Method-level parameters were already
        // spelled `#m0`, `#m1`, … above, so they cannot collide.
        //
        // Substitution can undo what normalization sorted: `x: T | U` is
        // `#0 | #1` in `Alpha<T, U>` but `#1 | #0` in `Beta<U, T>`, so a
        // substituted annotation is normalized again on its final tokens.
        // Heritage clauses take the same normalization as annotations, so
        // their type arguments compare like member types.
        let generics = type_parameter_names(class.type_parameters.as_deref());
        let substitute = |text: &str| TypeExtractor::substitute_generic_tokens(text, &generics);
        let positional = |text: &str| normalize_type_name(&substitute(text));
        if !generics.is_empty() {
            for property in &mut properties {
                property.type_annotation = positional(&property.type_annotation);
            }
            for method in &mut methods {
                method.parameters = method
                    .parameters
                    .iter()
                    .map(|p| normalize_parameter_list(&substitute(p)))
                    .collect();
                method.return_type = positional(&method.return_type);
            }
            constructor_params = constructor_params
                .iter()
                .map(|p| normalize_parameter_list(&substitute(p)))
                .collect();
        }

        ClassDefinition {
            name,
            properties,
            methods,
            constructor_params,
            constructor_wiring,
            extends: extends.map(|text| positional(&text)),
            implements: implements.into_iter().map(|text| positional(&text)).collect(),
            start_line: self.line_of(class.span.start),
            end_line: self.line_of(class.span.end),
            file_path: self.file_path.to_string(),
            is_abstract: class.r#abstract,
            // Decorators sit above the `class` keyword, so a directive
            // above them is above the class.
            has_ignore_directive: ignored
                || self.directive_above(class.span.start)
                || class
                    .decorators
                    .first()
                    .is_some_and(|first| self.directive_above(first.span.start)),
        }
    }

    /// A method's return type as type mode renders it; `void` when the
    /// method declares none.
    fn render_return_type(&self, annotation: Option<&TSTypeAnnotation>) -> String {
        annotation.map_or_else(|| "void".to_string(), |rt| self.render_type(&rt.type_annotation))
    }

    /// A method, getter or setter.
    fn extract_method(&self, name: String, method: &MethodDefinition) -> ClassMethod {
        let generics = type_parameter_names(method.value.type_parameters.as_deref());
        ClassMethod {
            name,
            parameters: vec![scoped_parameters(
                &self.extract_function_params(&method.value.params),
                &generics,
            )],
            return_type: scoped_type(
                &self.render_return_type(method.value.return_type.as_deref()),
                &generics,
            ),
            is_static: method.r#static,
            is_private: is_private_member(&method.key, method.accessibility),
            is_async: method.value.r#async,
            is_generator: method.value.generator,
            is_optional: method.optional,
            kind: match method.kind {
                MethodDefinitionKind::Get => MethodKind::Getter,
                MethodDefinitionKind::Set => MethodKind::Setter,
                MethodDefinitionKind::Method | MethodDefinitionKind::Constructor => {
                    MethodKind::Method
                }
            },
            body_fingerprint: self.method_body_fingerprint(&method.value),
        }
    }

    /// What a constructor contributes: its signature and wiring, the fields
    /// its parameter properties declare, and — when it does real work —
    /// itself, as a member.
    fn extract_constructor(&self, name: String, function: &Function, members: &mut ClassMembers) {
        members.constructor_params = function
            .params
            .items
            .iter()
            .map(|param| {
                format!(
                    "{}: {}",
                    parameter_name(param),
                    self.render_annotation(param.type_annotation.as_deref())
                )
            })
            .collect();
        members.constructor_wiring = constructor_wiring(function);

        // Constructor parameter properties (`constructor(private readonly
        // repo: Repo)`) declare real fields; surface them so the shorthand
        // and the explicit field-plus-assignment spelling extract the same
        // shape.
        for param in &function.params.items {
            let (true, BindingPattern::BindingIdentifier(ident)) =
                (is_parameter_property(param), &param.pattern)
            else {
                continue;
            };
            members.properties.push(ClassProperty {
                name: ident.name.as_str().to_string(),
                // An unannotated parameter property with a default
                // (`private indent = "\t"`) is typed by its default,
                // exactly like the field spelling.
                type_annotation: self
                    .member_type(param.type_annotation.as_deref(), param.initializer.as_deref()),
                is_static: false,
                is_private: param.accessibility == Some(TSAccessibility::Private),
                is_readonly: param.readonly,
                is_optional: false,
            });
        }

        // A constructor that does real work is a member like any other:
        // two classes with the same fields and methods but a constructor
        // that spawns a timer, validates, or delegates used to compare as
        // identical because the constructor was skipped. Wiring-only
        // constructors (`super(…)` plus `this.x = x`, or the empty body of
        // a parameter-property constructor) add nothing the field list
        // doesn't already say, so they stay out — otherwise a class that
        // happens to have one would be "missing" it on the other side.
        // What they wire where is still compared, through
        // `constructor_wiring`.
        if !is_wiring_only_constructor(function) {
            members.methods.push(ClassMethod {
                name,
                parameters: vec![self.extract_function_params(&function.params)],
                return_type: "void".to_string(),
                is_static: false,
                is_private: false,
                is_async: false,
                is_generator: false,
                is_optional: false,
                kind: MethodKind::Constructor,
                body_fingerprint: self.constructor_fingerprint(function),
            });
        }
    }

    /// Canonical structural hash of a method's params+body (the method
    /// NAME is deliberately excluded so consistently-renamed methods with
    /// equal bodies fingerprint identically).
    fn method_body_fingerprint(&self, function: &Function) -> Option<u64> {
        let body = function.body.as_ref()?;
        let params_text = slice(self.source_text, function.params.span)?;
        let body_text = slice(self.source_text, body.span)?;
        let mut prefix = String::new();
        if function.r#async {
            prefix.push_str("async ");
        }
        prefix.push_str("function");
        if function.generator {
            prefix.push('*');
        }
        self.function_fingerprint(&prefix, params_text, body_text)
    }

    /// Fingerprint of `<keyword> <name><params> <body>`, the standalone
    /// function a method's parameters and body make. The function is
    /// declared under a name its text does not spell, so no call in the body
    /// resolves to it: a member's key binds nothing inside the member.
    fn function_fingerprint(
        &self,
        keyword: &str,
        params_text: &str,
        body_text: &str,
    ) -> Option<u64> {
        let name =
            unused_synthetic_name(METHOD_FRAGMENT_NAME, &format!("{params_text} {body_text}"));
        self.fingerprint_fragment(&format!("{keyword} {name}{params_text} {body_text}"))
    }

    /// Fingerprint for an arrow-function class field, shaped exactly like
    /// [`Self::method_body_fingerprint`] so `handle = () => {…}` and
    /// `handle() {…}` with equal bodies hash identically.
    fn arrow_body_fingerprint(&self, arrow: &ArrowFunctionExpression) -> Option<u64> {
        let params_text = slice(self.source_text, arrow.params.span)?;
        let params_text = if params_text.starts_with('(') {
            params_text.to_string()
        } else {
            format!("({params_text})")
        };
        let body_text = slice(self.source_text, arrow.body.span)?;
        let body_text = if arrow.expression {
            format!("{{ return {body_text}; }}")
        } else {
            body_text.to_string()
        };
        let keyword = if arrow.r#async { "async function" } else { "function" };
        self.function_fingerprint(keyword, &params_text, &body_text)
    }

    /// Fingerprint for a constructor. Parsed inside a class wrapper so the
    /// canonicalizer's parameter-property desugaring applies:
    /// `constructor(private readonly repo: Repo) {}` and
    /// `constructor(repo: Repo) { this.repo = repo; }` hash identically.
    fn constructor_fingerprint(&self, function: &Function) -> Option<u64> {
        let body = function.body.as_ref()?;
        let params_text = slice(self.source_text, function.params.span)?;
        let body_text = slice(self.source_text, body.span)?;
        let wrapper =
            unused_synthetic_name(CLASS_WRAPPER_NAME, &format!("{params_text} {body_text}"));
        self.fingerprint_fragment(&format!(
            "class {wrapper} {{ constructor{params_text} {body_text} }}"
        ))
    }

    /// Parse a synthesized standalone function (or class-wrapped
    /// constructor) and hash its canonical tree with `this.<field>` names
    /// positionally normalized. `None` when the fragment does not parse.
    ///
    /// The fragment is parsed as the file is — a method of a `.tsx` file
    /// may hold JSX, which does not parse as plain TypeScript, and a body
    /// that does not parse has no fingerprint to compare.
    fn fingerprint_fragment(&self, fragment: &str) -> Option<u64> {
        let tree = parse_and_convert_to_tree_canonical(self.file_path, fragment).ok()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hash_method_tree(&tree, &mut hasher);
        Some(hasher.finish())
    }

    /// A heritage clause as written, minus whitespace (`React.Component`,
    /// `Base<string>`, `Mixin(Base)`). Compared only for equality, so a
    /// stable spelling is all that matters.
    fn heritage_text(&self, span: Span) -> String {
        slice(self.source_text, span).map_or_else(
            || "unknown".to_string(),
            |text| text.chars().filter(|c| !c.is_whitespace()).collect(),
        )
    }

    /// Member name for a class element key. Private names keep their `#`,
    /// computed keys keep their bracketed source (`[Symbol.iterator]`) —
    /// both used to be dropped, which left classes made of private fields
    /// or well-known-symbol methods looking member-less.
    fn member_key_name(&self, key: &PropertyKey, computed: bool) -> Option<String> {
        Some(match key {
            PropertyKey::StaticIdentifier(ident) => ident.name.as_str().to_string(),
            PropertyKey::StringLiteral(str_lit) => str_lit.value.as_str().to_string(),
            PropertyKey::PrivateIdentifier(ident) => format!("#{}", ident.name.as_str()),
            other => {
                let text = slice(self.source_text, other.span())?;
                if computed {
                    format!("[{text}]")
                } else {
                    text.to_string()
                }
            }
        })
    }
}

/// A parameter's name, or `param` for a destructuring pattern.
fn parameter_name<'a>(param: &'a FormalParameter) -> &'a str {
    match &param.pattern {
        BindingPattern::BindingIdentifier(ident) => ident.name.as_str(),
        _ => "param",
    }
}

/// Whether a constructor parameter declares a field (`private x`,
/// `readonly x`, `override x`).
fn is_parameter_property(param: &FormalParameter) -> bool {
    param.accessibility.is_some() || param.readonly || param.r#override
}

/// `private x` or `#x`.
fn is_private_member(key: &PropertyKey, accessibility: Option<TSAccessibility>) -> bool {
    accessibility == Some(TSAccessibility::Private)
        || matches!(key, PropertyKey::PrivateIdentifier(_))
}

/// Type-parameter names of a declaration, in order.
fn type_parameter_names(params: Option<&TSTypeParameterDeclaration>) -> Vec<String> {
    params.map_or_else(Vec::new, |params| {
        params.params.iter().map(|param| param.name.name.to_string()).collect()
    })
}

/// Spell a method's own type parameters positionally (`#m0`, `#m1`, …):
/// `map<U>(fn: (item: T) => U): U[]` and `map<R>(fn: (item: T) => R):
/// R[]` are the same signature. The result is normalized again, as the
/// class-level substitution is (see `ClassExtractor::extract_class`).
fn scoped_type(text: &str, method_generics: &[String]) -> String {
    if method_generics.is_empty() {
        return text.to_string();
    }
    normalize_type_name(&TypeExtractor::substitute_generic_tokens_with(text, method_generics, "#m"))
}

/// [`scoped_type`] for a `name: type, …` parameter list.
fn scoped_parameters(text: &str, method_generics: &[String]) -> String {
    if method_generics.is_empty() {
        return text.to_string();
    }
    normalize_parameter_list(&TypeExtractor::substitute_generic_tokens_with(
        text,
        method_generics,
        "#m",
    ))
}

/// Normalize the type of every `name: type` piece of a parameter list (see
/// `ClassExtractor::extract_function_params`); the list as a whole is not
/// a type.
fn normalize_parameter_list(text: &str) -> String {
    split_top_level(text, ',')
        .iter()
        .filter(|piece| !piece.is_empty())
        .map(|piece| match piece.split_once(": ") {
            Some((name, annotation)) => format!("{name}: {}", normalize_type_name(annotation)),
            None => piece.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Where a constructor stores its parameters — see
/// [`ClassDefinition::constructor_wiring`]. Parameter properties wire
/// their parameter into the same-named field; top-level `this.f = p`
/// statements and `super(…)` arguments that are bare parameter
/// references wire `p` into `f` / the argument slot.
fn constructor_wiring(function: &Function) -> Vec<(String, String)> {
    let mut keys: Vec<(String, String)> = Vec::new();
    for (index, param) in function.params.items.iter().enumerate() {
        binding_keys(&param.pattern, &format!("#{index}"), &mut keys);
    }
    if let Some(rest) = &function.params.rest {
        binding_keys(&rest.rest.argument, "...", &mut keys);
    }
    let key_of =
        |name: &str| keys.iter().find(|(binding, _)| binding == name).map(|(_, key)| key.clone());

    let mut wiring = Vec::new();
    for param in &function.params.items {
        if let (true, BindingPattern::BindingIdentifier(ident)) =
            (is_parameter_property(param), &param.pattern)
        {
            if let Some(key) = key_of(ident.name.as_str()) {
                wiring.push((key, ident.name.to_string()));
            }
        }
    }
    let Some(body) = &function.body else {
        return wiring;
    };
    for stmt in &body.statements {
        let Statement::ExpressionStatement(expr_stmt) = stmt else {
            continue;
        };
        match &expr_stmt.expression {
            Expression::CallExpression(call) if matches!(call.callee, Expression::Super(_)) => {
                for (slot, argument) in call.arguments.iter().enumerate() {
                    let Some(Expression::Identifier(ident)) =
                        argument.as_expression().map(Expression::get_inner_expression)
                    else {
                        continue;
                    };
                    if let Some(key) = key_of(ident.name.as_str()) {
                        wiring.push((key, format!("super[{slot}]")));
                    }
                }
            }
            Expression::AssignmentExpression(assign)
                if assign.operator == AssignmentOperator::Assign =>
            {
                let AssignmentTarget::StaticMemberExpression(member) = &assign.left else {
                    continue;
                };
                if !matches!(member.object, Expression::ThisExpression(_)) {
                    continue;
                }
                let Expression::Identifier(ident) = assign.right.get_inner_expression() else {
                    continue;
                };
                if let Some(key) = key_of(ident.name.as_str()) {
                    wiring.push((key, member.property.name.to_string()));
                }
            }
            _ => {}
        }
    }
    wiring
}

/// Positional keys for every binding a parameter pattern introduces:
/// `#0` for a plain parameter, `#1.host` for a destructured member,
/// `#2[0]` for an array element.
fn binding_keys(pattern: &BindingPattern, key: &str, out: &mut Vec<(String, String)>) {
    match pattern {
        BindingPattern::BindingIdentifier(ident) => {
            out.push((ident.name.to_string(), key.to_string()));
        }
        BindingPattern::ObjectPattern(object) => {
            for property in &object.properties {
                let member = match &property.key {
                    PropertyKey::StaticIdentifier(ident) => ident.name.to_string(),
                    PropertyKey::StringLiteral(literal) => literal.value.to_string(),
                    _ => "?".to_string(),
                };
                binding_keys(&property.value, &format!("{key}.{member}"), out);
            }
            if let Some(rest) = &object.rest {
                binding_keys(&rest.argument, &format!("{key}..."), out);
            }
        }
        BindingPattern::ArrayPattern(array) => {
            for (index, element) in array.elements.iter().enumerate() {
                if let Some(element) = element {
                    binding_keys(element, &format!("{key}[{index}]"), out);
                }
            }
            if let Some(rest) = &array.rest {
                binding_keys(&rest.argument, &format!("{key}[...]"), out);
            }
        }
        BindingPattern::AssignmentPattern(assignment) => {
            binding_keys(&assignment.left, key, out);
        }
    }
}

/// Whether a constructor body only forwards to `super(…)` and stores
/// its own parameters into `this` fields (or is empty). Storing
/// anything else — a module-level singleton, a computed value — is
/// work the class does, and counts as a member. See
/// `ClassExtractor::extract_constructor`.
fn is_wiring_only_constructor(function: &Function) -> bool {
    let Some(body) = &function.body else {
        return true;
    };
    let parameters: std::collections::HashSet<&str> = function
        .params
        .items
        .iter()
        .map(|param| &param.pattern)
        .chain(function.params.rest.as_ref().map(|rest| &rest.rest.argument))
        .flat_map(|pattern| pattern.get_binding_identifiers())
        .map(|ident| ident.name.as_str())
        .collect();
    body.statements.iter().all(|stmt| {
        let Statement::ExpressionStatement(expr_stmt) = stmt else {
            return false;
        };
        match &expr_stmt.expression {
            Expression::CallExpression(call) => matches!(call.callee, Expression::Super(_)),
            // Parentheses and `as`/`satisfies`/`!` around the stored
            // parameter are transparent, as in `constructor_wiring`:
            // `this.x = (x)` and `this.x = x as T` are wiring too.
            Expression::AssignmentExpression(assign) => {
                assign.operator == AssignmentOperator::Assign
                    && matches!(
                        &assign.left,
                        AssignmentTarget::StaticMemberExpression(member)
                            if matches!(member.object, Expression::ThisExpression(_))
                    )
                    && matches!(
                        assign.right.get_inner_expression(),
                        Expression::Identifier(ident)
                            if parameters.contains(ident.name.as_str())
                    )
            }
            _ => false,
        }
    })
}

/// Feed a method's canonical tree into `hasher`.
///
/// `this.<field>` / `this.#<field>` accesses hash the field position as a
/// placeholder: renaming private storage fields is the class-level
/// analogue of renaming locals, and two methods that differ only in field
/// names must fingerprint identically. Field names still matter for the
/// property-set comparison, so the signal isn't lost — it's just not
/// double-counted against the method bodies.
fn hash_method_tree(node: &TreeNode, hasher: &mut impl Hasher) {
    node.label.hash(hasher);
    node.kind.hash(hasher);
    node.children.len().hash(hasher);
    let this_member = matches!(node.kind, "StaticMemberExpression" | "PrivateFieldExpression")
        && node.children.first().is_some_and(|obj| obj.kind == "ThisExpression");
    for (index, child) in node.children.iter().enumerate() {
        if this_member && index == 1 && matches!(child.kind, "Identifier" | "PrivateIdentifier") {
            "§field".hash(hasher);
            child.kind.hash(hasher);
            0usize.hash(hasher);
        } else {
            hash_method_tree(child, hasher);
        }
    }
}

/// Extract the classes of a source file.
///
/// # Errors
///
/// Returns an error when the file cannot be analyzed: syntax errors, or
/// nesting beyond the limits of [`crate::source`].
pub fn extract_classes_from_code(
    code: &str,
    file_path: &str,
) -> Result<Vec<ClassDefinition>, String> {
    let lines = LineIndex::new(code);
    with_checked_program(file_path, code, |program| {
        ClassExtractor::new(code, file_path, &lines).extract_classes(program)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_classes_marks_similarity_ignore_directives() {
        let source = r#"
class ActiveService {
    run(): void {}
}

// similarity-ignore
class IgnoredService {
    run(): void {}
}
"#;

        let classes = extract_classes_from_code(source, "test.ts").unwrap();

        let active = classes.iter().find(|class| class.name == "ActiveService").unwrap();
        assert!(!active.has_ignore_directive);

        let ignored = classes.iter().find(|class| class.name == "IgnoredService").unwrap();
        assert!(ignored.has_ignore_directive);
    }

    #[test]
    fn methods_of_tsx_files_are_fingerprinted() {
        // Method bodies used to be parsed as plain TypeScript whatever the
        // file was: a body holding JSX did not parse and had no
        // fingerprint, so `render` methods never compared by body.
        let class = |name: &str, body: &str| {
            let source = format!(
                "export class {name} extends Component {{ render() {{ {body} }} handle = () => {{ {body} }}; }}"
            );
            extract_classes_from_code(&source, "view.tsx").unwrap().remove(0)
        };
        let card =
            class("Card", "return <section className=\"card\">{this.props.title}</section>;");
        let panel =
            class("Panel", "return <section className=\"card\">{this.props.title}</section>;");
        let list = class("List", "return <ul className=\"list\">{this.props.items}</ul>;");
        for method in card.methods.iter().chain(&list.methods) {
            assert!(method.body_fingerprint.is_some(), "{}", method.name);
        }
        assert_eq!(card.methods[0].body_fingerprint, panel.methods[0].body_fingerprint);
        assert_ne!(card.methods[0].body_fingerprint, list.methods[0].body_fingerprint);
        // The arrow field and the method with the same body agree.
        assert_eq!(card.methods[0].body_fingerprint, card.methods[1].body_fingerprint);
    }

    #[test]
    fn a_call_spelled_like_the_fragment_name_is_not_recursion() {
        // A method is fingerprinted as a standalone function. Declared
        // under a fixed name, a call to an outer function of that name
        // resolved to the function itself.
        let source = "class A {}";
        let lines = LineIndex::new(source);
        let extractor = ClassExtractor::new(source, "test.ts", &lines);
        let calls_outer = extractor
            .function_fingerprint("function", "(n: number)", "{ return __m__(n - 1); }")
            .unwrap();
        let recursive = extractor
            .fingerprint_fragment("function __m__(n: number) { return __m__(n - 1); }")
            .unwrap();
        assert_ne!(calls_outer, recursive);
        // The name the fragment is declared under takes no part.
        let renamed = extractor
            .fingerprint_fragment("function other(n: number) { return __m__(n - 1); }")
            .unwrap();
        assert_eq!(calls_outer, renamed);
    }

    #[test]
    fn working_constructors_are_members_with_desugared_parameter_properties() {
        let shorthand = extract_classes_from_code(
            "export class A extends B { constructor(private readonly repo: Repo) { super(); this.repo.warm(); } }",
            "a.ts",
        )
        .unwrap();
        let explicit = extract_classes_from_code(
            "export class C extends B { private readonly repo: Repo; constructor(repo: Repo) { super(); this.repo = repo; this.repo.warm(); } }",
            "b.ts",
        )
        .unwrap();
        let ctor = |class: &ClassDefinition| {
            class
                .methods
                .iter()
                .find(|m| m.kind == MethodKind::Constructor)
                .cloned()
                .expect("constructor member")
        };
        let (a, c) = (ctor(&shorthand[0]), ctor(&explicit[0]));
        assert_eq!(a.name, "constructor");
        assert!(a.body_fingerprint.is_some());
        assert_eq!(
            a.body_fingerprint, c.body_fingerprint,
            "shorthand and explicit wiring must fingerprint identically"
        );

        let divergent = extract_classes_from_code(
            "export class D extends B { private readonly repo: Repo; constructor(repo: Repo) { super(); this.repo = repo; this.repo.warm(); this.repo.prime(); } }",
            "d.ts",
        )
        .unwrap();
        assert_ne!(ctor(&divergent[0]).body_fingerprint, c.body_fingerprint);
    }

    #[test]
    fn wiring_only_constructors_are_not_members() {
        for source in [
            "export class A { constructor(private readonly repo: Repo) {} }",
            "export class B extends Base { constructor(private readonly repo: Repo) { super(); } }",
            "export class C { private repo: Repo; constructor(repo: Repo) { this.repo = repo; } }",
        ] {
            let classes = extract_classes_from_code(source, "wiring.ts").unwrap();
            assert!(
                classes[0].methods.iter().all(|m| m.kind != MethodKind::Constructor),
                "wiring-only constructor must not be a member: {source}"
            );
        }
    }

    #[test]
    fn computed_and_private_member_keys_are_kept() {
        let classes = extract_classes_from_code(
            "export class C { #count = 0; [Symbol.iterator]() { return this; } get [Symbol.toStringTag]() { return 'C'; } }",
            "c.ts",
        )
        .unwrap();
        let props: Vec<&str> = classes[0].properties.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(props, vec!["#count"]);
        let methods: Vec<&str> = classes[0].methods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(methods, vec!["[Symbol.iterator]", "[Symbol.toStringTag]"]);
    }

    #[test]
    fn heritage_is_recorded_as_written() {
        let classes = extract_classes_from_code(
            r"
export class Widget extends React.Component<Props, State> implements ts.Node, Disposable {}
export class Mixed extends Mixin( Base ) {}
export class Plain extends Base {}
",
            "h.ts",
        )
        .unwrap();
        // Type arguments are normalized like annotations (`Props, State`).
        assert_eq!(classes[0].extends.as_deref(), Some("React.Component<Props, State>"));
        assert_eq!(classes[0].implements, vec!["ts.Node", "Disposable"]);
        assert_eq!(classes[1].extends.as_deref(), Some("Mixin(Base)"));
        assert_eq!(classes[2].extends.as_deref(), Some("Base"));
    }

    #[test]
    fn transparent_wrappers_around_stored_parameters_are_wiring() {
        // `this.x = (x)` and `this.x = x as T` store the parameter like
        // `this.x = x`: wiring, not constructor work.
        for source in [
            "export class A { x: number; y: number; constructor(x: number, y: number) { this.x = (x); this.y = y; } }",
            "export class B { x: number; y: number; constructor(x: number, y: number) { this.x = x as number; this.y = y!; } }",
            "export class C { x: number; y: number; constructor(x: number, y: number) { this.x = x satisfies number; this.y = <number>y; } }",
        ] {
            let classes = extract_classes_from_code(source, "wrap.ts").unwrap();
            assert!(
                classes[0].methods.iter().all(|m| m.kind != MethodKind::Constructor),
                "{source}: a wiring-only constructor is not a member"
            );
            assert_eq!(
                classes[0].constructor_wiring,
                vec![("#0".to_string(), "x".to_string()), ("#1".to_string(), "y".to_string())],
                "{source}"
            );
        }
    }

    #[test]
    fn constructors_storing_non_parameters_are_work() {
        // `this.registry = globalRegistry` stores something the parameter
        // list never mentions; that is work, not wiring.
        let classes = extract_classes_from_code(
            "export class A { private registry: Registry; constructor() { this.registry = globalRegistry; } }",
            "work.ts",
        )
        .unwrap();
        assert!(classes[0].methods.iter().any(|m| m.kind == MethodKind::Constructor));

        // Destructured and rest parameters are still parameters.
        for source in [
            "export class B { constructor({ host, port }: Opts) { this.host = host; this.port = port; } }",
            "export class C { constructor(...items: string[]) { this.items = items; } }",
        ] {
            let classes = extract_classes_from_code(source, "wiring.ts").unwrap();
            assert!(
                classes[0].methods.iter().all(|m| m.kind != MethodKind::Constructor),
                "parameter wiring must not be a member: {source}"
            );
        }
    }

    #[test]
    fn member_types_render_like_type_mode() {
        let classes = extract_classes_from_code(
            r#"
export class Widget {
  status: "open" | "closed" = "open";
  kind: SyntaxKind.Block;
  render: React.ComponentType<Props>;
  ids: readonly string[] = [];
  pair: [string, number];
  key: keyof Options;
  count = 0;
  cache = new Map<string, Array<User>>();
  label = `w`;
  load(): Promise<User> { return this.repo.load(); }
  log(prefix?: string, ...args: unknown[]): void {}
}
"#,
            "w.ts",
        )
        .unwrap();
        let types: Vec<(&str, &str)> = classes[0]
            .properties
            .iter()
            .map(|p| (p.name.as_str(), p.type_annotation.as_str()))
            .collect();
        assert_eq!(
            types,
            vec![
                ("status", "\"closed\" | \"open\""),
                ("kind", "SyntaxKind.Block"),
                ("render", "React.ComponentType<Props>"),
                ("ids", "ReadonlyArray<string>"),
                ("pair", "[string, number]"),
                ("key", "keyof Options"),
                ("count", "number"),
                ("cache", "Map<string, User[]>"),
                ("label", "string"),
            ]
        );
        let load = &classes[0].methods[0];
        assert_eq!(load.return_type, "Promise<User>");
        let signed = extract_classes_from_code(
            "export class Cursor { offset = -1; step = +2; big = -1n; open = !0; kind = typeof window; }",
            "s.ts",
        )
        .unwrap();
        let types: Vec<&str> =
            signed[0].properties.iter().map(|p| p.type_annotation.as_str()).collect();
        assert_eq!(types, vec!["number", "number", "bigint", "boolean", "string"]);
        let accessors = extract_classes_from_code(
            "export class Counter { accessor count = 0; static accessor label: string; }",
            "a.ts",
        )
        .unwrap();
        let members: Vec<(&str, &str, bool)> = accessors[0]
            .properties
            .iter()
            .map(|p| (p.name.as_str(), p.type_annotation.as_str(), p.is_static))
            .collect();
        assert_eq!(members, vec![("count", "number", false), ("label", "string", true)]);
        let log = &classes[0].methods[1];
        assert_eq!(log.parameters, vec!["prefix?: string, ...args: unknown[]"]);
    }

    #[test]
    fn class_type_parameters_are_positional() {
        let classes = extract_classes_from_code(
            r"
export class Alpha<TItem> extends Base<TItem> implements Store<TItem> {
  private items: TItem[] = [];
  add(item: TItem): void { this.items.push(item); }
  map<R>(fn: (item: TItem) => R): R[] { return this.items.map(fn); }
}
export class Beta<U> extends Base<U> implements Store<U> {
  private items: U[] = [];
  add(item: U): void { this.items.push(item); }
  map<S>(fn: (item: U) => S): S[] { return this.items.map(fn); }
}
",
            "g.ts",
        )
        .unwrap();
        // Literal text and qualified-name segments that happen to spell a
        // type parameter are not references to it.
        let spelled = extract_classes_from_code(
            r#"
export class Walker<Node> extends ts.Visitor<Node> {
  root: ts.Node;
  kind: "Node" = "Node";
  label: `${Node}-walker`;
  current: Node;
}
"#,
            "w.ts",
        )
        .unwrap();
        assert_eq!(spelled[0].extends.as_deref(), Some("ts.Visitor<#0>"));
        let annotations: Vec<&str> =
            spelled[0].properties.iter().map(|p| p.type_annotation.as_str()).collect();
        assert_eq!(annotations, vec!["ts.Node", "\"Node\"", "`${#0}-walker`", "#0"]);
        for class in &classes {
            assert_eq!(class.extends.as_deref(), Some("Base<#0>"));
            assert_eq!(class.implements, vec!["Store<#0>"]);
            assert_eq!(class.properties[0].type_annotation, "#0[]");
            // Function-type parameter names are local to the signature and
            // drop out, as in type mode.
            assert_eq!(class.methods[1].parameters, vec!["fn: (#0) => #m0"]);
            assert_eq!(class.methods[1].return_type, "#m0[]");
        }
    }

    #[test]
    fn constructor_wiring_records_parameter_targets() {
        let wiring = |source: &str| {
            extract_classes_from_code(source, "c.ts").unwrap()[0].constructor_wiring.clone()
        };
        let pairs = |list: &[(&str, &str)]| -> Vec<(String, String)> {
            list.iter().map(|(a, b)| ((*a).to_string(), (*b).to_string())).collect()
        };
        assert_eq!(
            wiring("export class P { x: number; y: number; constructor(x: number, y: number) { this.x = y; this.y = x; } }"),
            pairs(&[("#1", "x"), ("#0", "y")])
        );
        assert_eq!(
            wiring("export class P { constructor(public x: number, readonly y: number) {} }"),
            pairs(&[("#0", "x"), ("#1", "y")])
        );
        assert_eq!(
            wiring("export class S extends B { constructor(name: string, id: string) { super(id, name); } }"),
            pairs(&[("#1", "super[0]"), ("#0", "super[1]")])
        );
        assert_eq!(
            wiring("export class D { constructor({ host, port }: Opts, ...rest: string[]) { this.host = host; this.port = port; this.rest = rest; } }"),
            pairs(&[("#0.host", "host"), ("#0.port", "port"), ("...", "rest")])
        );
    }

    #[test]
    fn namespace_scoped_classes_are_extracted() {
        let source = r"
export namespace Legacy {
    export class Pager {
        advance(): void {}
    }
    namespace Inner {
        class Hidden {}
    }
}
";
        let classes = extract_classes_from_code(source, "test.ts").unwrap();
        let names: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["Legacy.Pager", "Legacy.Inner.Hidden"]);
    }
}
