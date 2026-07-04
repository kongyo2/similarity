use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Expression, PropertyKey, Statement, TSInterfaceDeclaration, TSPropertySignature, TSType,
    TSTypeAliasDeclaration, VariableDeclarator,
};
use oxc_parser::Parser;
use oxc_span::SourceType;

/// Render a possibly-qualified type name (`React.FC`, `A.B.C`) as its
/// full dotted path — collapsing to the rightmost segment would let
/// `React.FC` compare equal to any other namespace's `FC`.
fn render_ts_type_name(name: &oxc_ast::ast::TSTypeName) -> String {
    match name {
        oxc_ast::ast::TSTypeName::IdentifierReference(ident) => ident.name.as_str().to_string(),
        oxc_ast::ast::TSTypeName::QualifiedName(qualified) => {
            format!("{}.{}", render_ts_type_name(&qualified.left), qualified.right.name.as_str())
        }
        _ => "unknown".to_string(),
    }
}

/// Return the byte span (start, end) of any `TSType` variant. The variants
/// each carry their own `span` field but there is no shared accessor; pattern
/// matching every variant explicitly keeps us insulated from changes to oxc's
/// `GetSpan` trait surface area.
fn ts_type_span(ts_type: &TSType) -> (u32, u32) {
    let span = match ts_type {
        TSType::TSAnyKeyword(t) => t.span,
        TSType::TSBigIntKeyword(t) => t.span,
        TSType::TSBooleanKeyword(t) => t.span,
        TSType::TSIntrinsicKeyword(t) => t.span,
        TSType::TSNeverKeyword(t) => t.span,
        TSType::TSNullKeyword(t) => t.span,
        TSType::TSNumberKeyword(t) => t.span,
        TSType::TSObjectKeyword(t) => t.span,
        TSType::TSStringKeyword(t) => t.span,
        TSType::TSSymbolKeyword(t) => t.span,
        TSType::TSUndefinedKeyword(t) => t.span,
        TSType::TSUnknownKeyword(t) => t.span,
        TSType::TSVoidKeyword(t) => t.span,
        TSType::TSArrayType(t) => t.span,
        TSType::TSConditionalType(t) => t.span,
        TSType::TSConstructorType(t) => t.span,
        TSType::TSFunctionType(t) => t.span,
        TSType::TSImportType(t) => t.span,
        TSType::TSIndexedAccessType(t) => t.span,
        TSType::TSInferType(t) => t.span,
        TSType::TSIntersectionType(t) => t.span,
        TSType::TSLiteralType(t) => t.span,
        TSType::TSMappedType(t) => t.span,
        TSType::TSNamedTupleMember(t) => t.span,
        TSType::TSTemplateLiteralType(t) => t.span,
        TSType::TSThisType(t) => t.span,
        TSType::TSTupleType(t) => t.span,
        TSType::TSTypeLiteral(t) => t.span,
        TSType::TSTypeOperatorType(t) => t.span,
        TSType::TSTypePredicate(t) => t.span,
        TSType::TSTypeQuery(t) => t.span,
        TSType::TSTypeReference(t) => t.span,
        TSType::TSUnionType(t) => t.span,
        TSType::TSParenthesizedType(t) => t.span,
        TSType::JSDocNullableType(t) => t.span,
        TSType::JSDocNonNullableType(t) => t.span,
        TSType::JSDocUnknownType(t) => t.span,
    };
    (span.start, span.end)
}

/// Collapse runs of whitespace down to a single space, but leave whitespace
/// inside string and template literals untouched. Pure
/// `split_whitespace().join(" ")` would equate `type A = "a b"` with
/// `type A = "a  b"` (different cooked string values) because it ignores
/// literal boundaries, so we walk the bytes by hand and only collapse
/// outside quoted regions.
///
/// Behavior notes:
/// * Single- and double-quoted strings preserve every byte verbatim,
///   honoring `\"`, `\\`, … escapes.
/// * Backtick template literals preserve whitespace inside the literal
///   sections but collapse whitespace inside `${...}` interpolations,
///   because a template type's interpolated expression is a normal type
///   expression where formatting shouldn't change identity.
/// * Line (`// …`) and block (`/* … */`) comments are dropped and
///   replaced with a single space, so a comment-only edit doesn't
///   register as a body-signature difference.
fn collapse_whitespace_outside_strings(text: &str) -> String {
    enum Mode {
        Code,
        // Single / double quoted string: collapse nothing inside.
        QuotedString(char),
        // Template literal text segment between backticks (outside any
        // `${...}` interpolation): preserve whitespace.
        TemplateText,
        // Template literal expression: collapse whitespace, with a depth
        // counter so nested braces don't end the interpolation prematurely.
        TemplateExpr(u32),
    }

    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    // Stack so a template inside `${...}` inside another template is
    // tracked correctly. The bottom of the stack is always `Code`.
    let mut stack: Vec<Mode> = vec![Mode::Code];
    let mut last_was_space = false;

    while let Some(ch) = chars.next() {
        let top = stack.last_mut().expect("stack always has at least one mode");
        match top {
            Mode::QuotedString(q) => {
                out.push(ch);
                if ch == '\\' {
                    if let Some(next) = chars.next() {
                        out.push(next);
                    }
                } else if ch == *q {
                    stack.pop();
                }
                last_was_space = false;
            }
            Mode::TemplateText => {
                out.push(ch);
                if ch == '\\' {
                    if let Some(next) = chars.next() {
                        out.push(next);
                    }
                } else if ch == '`' {
                    stack.pop();
                } else if ch == '$' && chars.peek() == Some(&'{') {
                    let brace = chars.next().unwrap();
                    out.push(brace);
                    stack.push(Mode::TemplateExpr(1));
                }
                last_was_space = false;
            }
            Mode::TemplateExpr(depth) => {
                if ch == '{' {
                    *depth += 1;
                    out.push(ch);
                    last_was_space = false;
                } else if ch == '}' {
                    *depth -= 1;
                    if *depth == 0 {
                        stack.pop();
                    }
                    out.push(ch);
                    last_was_space = false;
                } else if ch == '"' || ch == '\'' {
                    out.push(ch);
                    stack.push(Mode::QuotedString(ch));
                    last_was_space = false;
                } else if ch == '`' {
                    out.push(ch);
                    stack.push(Mode::TemplateText);
                    last_was_space = false;
                } else if ch == '/' && chars.peek() == Some(&'/') {
                    consume_line_comment(&mut chars);
                    push_space_if_needed(&mut out, &mut last_was_space);
                } else if ch == '/' && chars.peek() == Some(&'*') {
                    chars.next();
                    consume_block_comment(&mut chars);
                    push_space_if_needed(&mut out, &mut last_was_space);
                } else if ch.is_whitespace() {
                    push_space_if_needed(&mut out, &mut last_was_space);
                } else {
                    out.push(ch);
                    last_was_space = false;
                }
            }
            Mode::Code => {
                if ch == '"' || ch == '\'' {
                    out.push(ch);
                    stack.push(Mode::QuotedString(ch));
                    last_was_space = false;
                } else if ch == '`' {
                    out.push(ch);
                    stack.push(Mode::TemplateText);
                    last_was_space = false;
                } else if ch == '/' && chars.peek() == Some(&'/') {
                    consume_line_comment(&mut chars);
                    push_space_if_needed(&mut out, &mut last_was_space);
                } else if ch == '/' && chars.peek() == Some(&'*') {
                    chars.next();
                    consume_block_comment(&mut chars);
                    push_space_if_needed(&mut out, &mut last_was_space);
                } else if ch.is_whitespace() {
                    push_space_if_needed(&mut out, &mut last_was_space);
                } else {
                    out.push(ch);
                    last_was_space = false;
                }
            }
        }
    }

    while out.ends_with(' ') {
        out.pop();
    }

    out
}

fn consume_line_comment(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    // Already past the first `/`; consume the second `/` and everything up
    // to the next newline (exclusive — the newline gets folded by the
    // caller's whitespace handling).
    chars.next();
    while let Some(&next) = chars.peek() {
        if next == '\n' {
            break;
        }
        chars.next();
    }
}

fn consume_block_comment(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    // Already past `/`; the `*` is consumed by the caller. Walk forward
    // until we see `*/`. Unterminated comments fall through to EOF — we
    // never look past `text` so the worst case is dropping the trailing
    // partial comment, which matches typical formatter behavior.
    while let Some(c) = chars.next() {
        if c == '*' && chars.peek() == Some(&'/') {
            chars.next();
            break;
        }
    }
}

fn push_space_if_needed(out: &mut String, last_was_space: &mut bool) {
    if !*last_was_space && !out.is_empty() {
        out.push(' ');
        *last_was_space = true;
    }
}
use std::collections::HashMap;

use crate::ignore_directive::has_similarity_ignore_directive;

#[derive(Debug, Clone)]
pub struct TypeDefinition {
    pub name: String,
    pub kind: TypeKind,
    pub properties: Vec<PropertyDefinition>,
    pub generics: Vec<String>,
    pub extends: Vec<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub file_path: String,
    pub has_ignore_directive: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    Interface,
    TypeAlias,
    TypeLiteral,
}

#[derive(Debug, Clone)]
pub struct PropertyDefinition {
    pub name: String,
    pub type_annotation: String,
    pub optional: bool,
    pub readonly: bool,
}

#[derive(Debug, Clone)]
pub struct TypeLiteralDefinition {
    pub name: String, // Function name, variable name, etc.
    pub context: TypeLiteralContext,
    pub properties: Vec<PropertyDefinition>,
    pub start_line: usize,
    pub end_line: usize,
    pub file_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeLiteralContext {
    FunctionReturn(String),            // Function name
    FunctionParameter(String, String), // Function name, parameter name
    VariableDeclaration(String),       // Variable name
    ArrowFunctionReturn(String),       // Variable name for arrow function
}

pub struct TypeExtractor {
    source_text: String,
    file_path: String,
    line_offsets: Vec<usize>,
}

impl TypeExtractor {
    pub fn new(source_text: String, file_path: String) -> Self {
        let line_offsets = Self::calculate_line_offsets(&source_text);
        Self { source_text, file_path, line_offsets }
    }

    fn calculate_line_offsets(source: &str) -> Vec<usize> {
        let mut offsets = vec![0];
        for (i, ch) in source.char_indices() {
            if ch == '\n' {
                offsets.push(i + 1);
            }
        }
        offsets
    }

    fn get_line_number(&self, offset: usize) -> usize {
        match self.line_offsets.binary_search(&offset) {
            Ok(line) => line + 1,
            Err(line) => line,
        }
    }

    pub fn extract_types(&self) -> Result<Vec<TypeDefinition>, String> {
        let allocator = Allocator::default();
        let source_type = SourceType::from_path(&self.file_path).unwrap_or(SourceType::tsx());
        let ret = Parser::new(&allocator, &self.source_text, source_type).parse();

        if !ret.errors.is_empty() {
            // Create a more readable error message
            let error_messages: Vec<String> =
                ret.errors.iter().map(|e| e.message.to_string()).collect();
            return Err(format!("Parse errors: {}", error_messages.join(", ")));
        }

        let mut types = Vec::new();

        for stmt in &ret.program.body {
            match stmt {
                Statement::TSInterfaceDeclaration(interface) => {
                    if let Some(type_def) = self.extract_interface(interface) {
                        types.push(type_def);
                    }
                }
                Statement::TSTypeAliasDeclaration(type_alias) => {
                    if let Some(type_def) = self.extract_type_alias(type_alias) {
                        types.push(type_def);
                    }
                }
                Statement::ExportNamedDeclaration(export) => {
                    if let Some(decl) = &export.declaration {
                        match decl {
                            oxc_ast::ast::Declaration::TSInterfaceDeclaration(interface) => {
                                if let Some(type_def) = self.extract_interface(interface) {
                                    types.push(type_def);
                                }
                            }
                            oxc_ast::ast::Declaration::TSTypeAliasDeclaration(type_alias) => {
                                if let Some(type_def) = self.extract_type_alias(type_alias) {
                                    types.push(type_def);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(types)
    }

    pub fn extract_type_literals(&self) -> Result<Vec<TypeLiteralDefinition>, String> {
        let allocator = Allocator::default();
        let source_type = SourceType::from_path(&self.file_path).unwrap_or(SourceType::tsx());
        let ret = Parser::new(&allocator, &self.source_text, source_type).parse();

        if !ret.errors.is_empty() {
            // Create a more readable error message
            let error_messages: Vec<String> =
                ret.errors.iter().map(|e| e.message.to_string()).collect();
            return Err(format!("Parse errors: {}", error_messages.join(", ")));
        }

        let mut type_literals = Vec::new();

        for stmt in &ret.program.body {
            self.extract_type_literals_from_statement(stmt, &mut type_literals);
        }

        Ok(type_literals)
    }


    /// Replace each generic parameter name with a positional `#N` token
    /// (whole identifiers only) in every property annotation, so two
    /// declarations that differ only in type-parameter names compare as
    /// equal while concrete instantiations (`Array<string>` vs
    /// `Array<number>`) keep their distinct arguments.
    fn substitute_generic_params(properties: &mut [PropertyDefinition], generics: &[String]) {
        if generics.is_empty() {
            return;
        }
        let replace_tokens = |input: &str| -> String {
            let mut result = String::with_capacity(input.len());
            let mut token = String::new();
            let flush = |token: &mut String, result: &mut String| {
                if token.is_empty() {
                    return;
                }
                if let Some(position) = generics.iter().position(|generic| generic == token) {
                    result.push_str(&format!("#{position}"));
                } else {
                    result.push_str(token);
                }
                token.clear();
            };
            for ch in input.chars() {
                if ch.is_alphanumeric() || ch == '_' || ch == '$' {
                    token.push(ch);
                } else {
                    flush(&mut token, &mut result);
                    result.push(ch);
                }
            }
            flush(&mut token, &mut result);
            result
        };
        for property in properties.iter_mut() {
            property.type_annotation = replace_tokens(&property.type_annotation);
        }
    }

    fn extract_interface(&self, interface: &TSInterfaceDeclaration) -> Option<TypeDefinition> {
        let name = interface.id.name.as_str().to_string();
        let start_line = self.get_line_number(interface.span.start as usize);
        let end_line = self.get_line_number(interface.span.end as usize);

        let mut properties = self.extract_interface_properties(&interface.body.body);
        let generics = self.extract_generics(interface.type_parameters.as_ref());
        Self::substitute_generic_params(&mut properties, &generics);
        let extends = self.extract_extends(Some(&interface.extends));

        Some(TypeDefinition {
            name,
            kind: TypeKind::Interface,
            properties,
            generics,
            extends,
            start_line,
            end_line,
            file_path: self.file_path.clone(),
            has_ignore_directive: has_similarity_ignore_directive(&self.source_text, start_line),
        })
    }

    fn extract_type_alias(&self, type_alias: &TSTypeAliasDeclaration) -> Option<TypeDefinition> {
        let name = type_alias.id.name.as_str().to_string();
        let start_line = self.get_line_number(type_alias.span.start as usize);
        let end_line = self.get_line_number(type_alias.span.end as usize);

        let mut properties = self.extract_type_properties(&type_alias.type_annotation);
        let generics = self.extract_generics(type_alias.type_parameters.as_ref());

        // Type aliases whose body is not an object literal (string-literal
        // unions, primitive aliases, function types, Record<…>, …) extract
        // to an empty `properties` Vec. Without a fallback the type comparator
        // sees them as "two empty objects", which collapses every such alias
        // pair onto the same structural-1.0 / naming-0 score and makes the
        // type mode effectively useless for non-object aliases. Synthesize a
        // single virtual property holding the textual type body so identical
        // aliases match exactly and distinct aliases compare by their body.
        //
        // `type X = {}` legitimately is an empty object type — the property
        // vector being empty there is the truth, not a fallback case. Only
        // synthesize the body signature when the underlying type is *not*
        // an object literal; otherwise the cross-kind match between
        // `interface X {}` and `type X = {}` would compare a real empty
        // interface against an alias carrying a `<type-body>` property and
        // fall apart.
        let is_object_literal_body =
            matches!(&type_alias.type_annotation, TSType::TSTypeLiteral(_));
        if properties.is_empty() && !is_object_literal_body {
            let body_signature = self.extract_type_body_signature(&type_alias.type_annotation);
            if !body_signature.is_empty() {
                properties.push(PropertyDefinition {
                    // Use angle brackets so the synthetic name can never
                    // collide with a real TypeScript identifier.
                    name: "<type-body>".to_string(),
                    type_annotation: body_signature,
                    optional: false,
                    readonly: false,
                });
            }
        }
        // Substitute AFTER the fallback body exists so non-object generic
        // aliases participate too: `type Box<T> = T[]` and
        // `type Bag<U> = U[]` must both carry the body `#0[]`.
        Self::substitute_generic_params(&mut properties, &generics);

        Some(TypeDefinition {
            name,
            kind: TypeKind::TypeAlias,
            properties,
            generics,
            extends: Vec::new(), // Type aliases don't have extends
            start_line,
            end_line,
            file_path: self.file_path.clone(),
            has_ignore_directive: has_similarity_ignore_directive(&self.source_text, start_line),
        })
    }

    /// Capture a stable textual signature for the body of a non-object
    /// type alias. Uses the source span when available so the signature
    /// reflects what the user actually wrote (modulo collapsed whitespace
    /// outside of string and template literals); falls back to the
    /// structured `extract_type_string` for shapes whose span we cannot
    /// recover.
    fn extract_type_body_signature(&self, ts_type: &TSType) -> String {
        let (start, end) = ts_type_span(ts_type);
        if end > start {
            let start = start as usize;
            let end = end as usize;
            if start < self.source_text.len() && end <= self.source_text.len() {
                let raw = &self.source_text[start..end];
                let collapsed = collapse_whitespace_outside_strings(raw);
                if !collapsed.is_empty() {
                    return collapsed;
                }
            }
        }
        self.extract_type_string(ts_type)
    }

    fn extract_interface_properties(
        &self,
        signatures: &[oxc_ast::ast::TSSignature],
    ) -> Vec<PropertyDefinition> {
        let mut properties = Vec::new();

        for signature in signatures {
            match signature {
                oxc_ast::ast::TSSignature::TSPropertySignature(prop_sig) => {
                    if let Some(prop_def) = self.extract_property_from_signature(prop_sig) {
                        properties.push(prop_def);
                    }
                }
                oxc_ast::ast::TSSignature::TSMethodSignature(method_sig) => {
                    if let Some(prop_def) = self.extract_method_from_signature(method_sig) {
                        properties.push(prop_def);
                    }
                }
                oxc_ast::ast::TSSignature::TSIndexSignature(index_sig) => {
                    // `{ [key: string]: number }` — previously dropped
                    // entirely, which made every index-signature interface
                    // look empty (and therefore identical to every other
                    // one). Surface it as a synthetic property keyed by
                    // the index kind so the VALUE type participates in
                    // comparison.
                    let key_type = index_sig.parameters.first().map_or_else(
                        || "string".to_string(),
                        |parameter| {
                            self.extract_type_string(&parameter.type_annotation.type_annotation)
                        },
                    );
                    let value_type = self
                        .extract_type_string(&index_sig.type_annotation.type_annotation);
                    properties.push(PropertyDefinition {
                        name: format!("[index: {key_type}]"),
                        type_annotation: value_type,
                        optional: false,
                        readonly: index_sig.readonly,
                    });
                }
                _ => {}
            }
        }

        properties
    }

    fn extract_type_properties(&self, ts_type: &TSType) -> Vec<PropertyDefinition> {
        match ts_type {
            TSType::TSTypeLiteral(type_literal) => {
                self.extract_interface_properties(&type_literal.members)
            }
            _ => Vec::new(), // For non-object types, return empty properties
        }
    }

    fn extract_property_from_signature(
        &self,
        prop_sig: &TSPropertySignature,
    ) -> Option<PropertyDefinition> {
        let name = match &prop_sig.key {
            PropertyKey::StaticIdentifier(ident) => ident.name.as_str().to_string(),
            PropertyKey::StringLiteral(str_lit) => str_lit.value.as_str().to_string(),
            _ => return None,
        };

        let type_annotation = prop_sig
            .type_annotation
            .as_ref()
            .map(|ta| self.extract_type_string(&ta.type_annotation))
            .unwrap_or_else(|| "any".to_string());

        Some(PropertyDefinition {
            name,
            type_annotation,
            optional: prop_sig.optional,
            readonly: prop_sig.readonly,
        })
    }

    fn extract_method_from_signature(
        &self,
        method_sig: &oxc_ast::ast::TSMethodSignature,
    ) -> Option<PropertyDefinition> {
        let name = match &method_sig.key {
            PropertyKey::StaticIdentifier(ident) => ident.name.as_str().to_string(),
            PropertyKey::StringLiteral(str_lit) => str_lit.value.as_str().to_string(),
            _ => return None,
        };

        // Extract method signature as a function type string
        let params = self.extract_function_params(&method_sig.params);
        let return_type = method_sig
            .return_type
            .as_ref()
            .map(|rt| self.extract_type_string(&rt.type_annotation))
            .unwrap_or_else(|| "void".to_string());

        let type_annotation = format!("({}) => {}", params, return_type);

        Some(PropertyDefinition {
            name,
            type_annotation,
            optional: method_sig.optional,
            readonly: false,
        })
    }

    #[allow(clippy::only_used_in_recursion)]
    fn extract_type_string(&self, ts_type: &TSType) -> String {
        match ts_type {
            TSType::TSStringKeyword(_) => "string".to_string(),
            TSType::TSNumberKeyword(_) => "number".to_string(),
            TSType::TSBooleanKeyword(_) => "boolean".to_string(),
            TSType::TSAnyKeyword(_) => "any".to_string(),
            TSType::TSUnknownKeyword(_) => "unknown".to_string(),
            TSType::TSVoidKeyword(_) => "void".to_string(),
            TSType::TSNullKeyword(_) => "null".to_string(),
            TSType::TSUndefinedKeyword(_) => "undefined".to_string(),
            TSType::TSTypeReference(type_ref) => {
                let base = match &type_ref.type_name {
                    oxc_ast::ast::TSTypeName::IdentifierReference(ident) => {
                        ident.name.as_str().to_string()
                    }
                    oxc_ast::ast::TSTypeName::QualifiedName(_) => {
                        // Render the full dotted path — `React.FC` must not
                        // compare equal to a bare `FC` or some other
                        // namespace's `FC`.
                        render_ts_type_name(&type_ref.type_name)
                    }
                    _ => "unknown".to_string(),
                };
                // Type arguments are part of the type's identity —
                // `Array<string>` and `Array<number>` must not compare
                // equal (they previously both rendered as just "Array").
                match &type_ref.type_arguments {
                    Some(args) if !args.params.is_empty() => {
                        let rendered: Vec<String> =
                            args.params.iter().map(|param| self.extract_type_string(param)).collect();
                        format!("{base}<{}>", rendered.join(", "))
                    }
                    _ => base,
                }
            }
            TSType::TSArrayType(array_type) => {
                let element_type = self.extract_type_string(&array_type.element_type);
                format!("{element_type}[]")
            }
            TSType::TSTupleType(tuple) => {
                let rendered: Vec<String> = tuple
                    .element_types
                    .iter()
                    .map(|element| match element {
                        oxc_ast::ast::TSTupleElement::TSOptionalType(optional) => {
                            format!("{}?", self.extract_type_string(&optional.type_annotation))
                        }
                        oxc_ast::ast::TSTupleElement::TSRestType(rest) => {
                            format!("...{}", self.extract_type_string(&rest.type_annotation))
                        }
                        _ => element
                            .as_ts_type()
                            .map_or_else(|| "unknown".to_string(), |t| self.extract_type_string(t)),
                    })
                    .collect();
                format!("[{}]", rendered.join(", "))
            }
            TSType::TSTypeOperatorType(operator) => {
                let operand = self.extract_type_string(&operator.type_annotation);
                match operator.operator {
                    oxc_ast::ast::TSTypeOperatorOperator::Keyof => format!("keyof {operand}"),
                    oxc_ast::ast::TSTypeOperatorOperator::Readonly => {
                        // `readonly T[]` is the same runtime shape as
                        // `ReadonlyArray<T>`; canonicalize the spelling.
                        format!("ReadonlyArray<{}>",
                            operand.strip_suffix("[]").unwrap_or(&operand))
                    }
                    oxc_ast::ast::TSTypeOperatorOperator::Unique => format!("unique {operand}"),
                }
            }
            TSType::TSIndexedAccessType(indexed) => {
                format!(
                    "{}[{}]",
                    self.extract_type_string(&indexed.object_type),
                    self.extract_type_string(&indexed.index_type)
                )
            }
            TSType::TSParenthesizedType(paren) => self.extract_type_string(&paren.type_annotation),
            TSType::TSNeverKeyword(_) => "never".to_string(),
            TSType::TSObjectKeyword(_) => "object".to_string(),
            TSType::TSSymbolKeyword(_) => "symbol".to_string(),
            TSType::TSBigIntKeyword(_) => "bigint".to_string(),
            TSType::TSThisType(_) => "this".to_string(),
            TSType::TSUnionType(union_type) => {
                let types: Vec<String> =
                    union_type.types.iter().map(|t| self.extract_type_string(t)).collect();
                types.join(" | ")
            }
            TSType::TSIntersectionType(intersection_type) => {
                let types: Vec<String> =
                    intersection_type.types.iter().map(|t| self.extract_type_string(t)).collect();
                types.join(" & ")
            }
            TSType::TSLiteralType(literal_type) => match &literal_type.literal {
                oxc_ast::ast::TSLiteral::StringLiteral(str_lit) => {
                    format!("\"{}\"", str_lit.value.as_str())
                }
                oxc_ast::ast::TSLiteral::NumericLiteral(num_lit) => num_lit.value.to_string(),
                oxc_ast::ast::TSLiteral::BooleanLiteral(bool_lit) => bool_lit.value.to_string(),
                _ => "unknown".to_string(),
            },
            TSType::TSFunctionType(func_type) => {
                let params = self.extract_function_params(&func_type.params);
                let return_type = self.extract_type_string(&func_type.return_type.type_annotation);
                format!("({}) => {}", params, return_type)
            }
            TSType::TSTypeLiteral(literal) => {
                // Render nested object types structurally (sorted members
                // so property order never matters) instead of collapsing
                // every one of them onto the same "object" token.
                let mut members: Vec<String> = self
                    .extract_interface_properties(&literal.members)
                    .into_iter()
                    .map(|property| {
                        format!(
                            "{}{}: {}",
                            property.name,
                            if property.optional { "?" } else { "" },
                            property.type_annotation
                        )
                    })
                    .collect();
                members.sort();
                format!("{{ {} }}", members.join("; "))
            }
            _ => "unknown".to_string(),
        }
    }

    fn extract_function_params(&self, params: &oxc_ast::ast::FormalParameters) -> String {
        let param_strings: Vec<String> = params
            .items
            .iter()
            .map(|param| {
                let param_name = match &param.pattern {
                    oxc_ast::ast::BindingPattern::BindingIdentifier(ident) => ident.name.as_str(),
                    _ => "_",
                };

                let param_type = param
                    .type_annotation
                    .as_ref()
                    .map(|ta| self.extract_type_string(&ta.type_annotation))
                    .unwrap_or_else(|| "any".to_string());

                format!("{}: {}", param_name, param_type)
            })
            .collect();

        param_strings.join(", ")
    }

    fn extract_generics(
        &self,
        type_params: Option<&oxc_allocator::Box<oxc_ast::ast::TSTypeParameterDeclaration>>,
    ) -> Vec<String> {
        if let Some(params) = type_params {
            params.params.iter().map(|param| param.name.name.as_str().to_string()).collect()
        } else {
            Vec::new()
        }
    }

    fn extract_extends(
        &self,
        extends: Option<&oxc_allocator::Vec<oxc_ast::ast::TSInterfaceHeritage>>,
    ) -> Vec<String> {
        if let Some(heritage_clauses) = extends {
            heritage_clauses
                .iter()
                .filter_map(|heritage| match &heritage.expression {
                    oxc_ast::ast::Expression::Identifier(ident) => {
                        Some(ident.name.as_str().to_string())
                    }
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    fn extract_type_literals_from_statement(
        &self,
        stmt: &Statement,
        type_literals: &mut Vec<TypeLiteralDefinition>,
    ) {
        match stmt {
            Statement::FunctionDeclaration(func) => {
                self.extract_from_function(func, type_literals);
            }
            Statement::VariableDeclaration(var_decl) => {
                self.extract_from_variable_declaration(var_decl, type_literals);
            }
            // Exports were previously dropped on the floor here, so anonymous
            // `export function foo(arg: { ... })` parameters were invisible to
            // `--type-literals`. Unwrap the inner declaration so they flow
            // through the same handlers as un-exported declarations.
            Statement::ExportNamedDeclaration(export) => {
                if let Some(decl) = &export.declaration {
                    self.extract_type_literals_from_declaration(decl, type_literals);
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                // Default-exported function declarations were already
                // unwrapped, but `export default (arg: { id: string }) => ...`
                // and `export default function (arg: { id: string }) {...}`
                // are expression forms that fell through the old match arm
                // and skipped type-literal extraction. Route any expression
                // default-export through the initializer walker so its
                // parameters, body, and return type still feed the
                // comparison pool.
                match &export.declaration {
                    oxc_ast::ast::ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                        self.extract_from_function(func, type_literals);
                    }
                    other => {
                        if let Some(expr) = other.as_expression() {
                            self.extract_type_literals_from_initializer(
                                expr,
                                "default",
                                type_literals,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn extract_type_literals_from_declaration(
        &self,
        decl: &oxc_ast::ast::Declaration,
        type_literals: &mut Vec<TypeLiteralDefinition>,
    ) {
        match decl {
            oxc_ast::ast::Declaration::FunctionDeclaration(func) => {
                self.extract_from_function(func, type_literals);
            }
            oxc_ast::ast::Declaration::VariableDeclaration(var_decl) => {
                self.extract_from_variable_declaration(var_decl, type_literals);
            }
            _ => {}
        }
    }

    fn extract_from_function(
        &self,
        func: &oxc_ast::ast::Function,
        type_literals: &mut Vec<TypeLiteralDefinition>,
    ) {
        let func_name = func
            .id
            .as_ref()
            .map(|id| id.name.to_string())
            .unwrap_or_else(|| "<anonymous>".to_string());

        // Extract return type literal
        if let Some(return_type) = &func.return_type {
            if let Some(type_literal) = self.extract_type_literal_from_ts_type(
                &return_type.type_annotation,
                TypeLiteralContext::FunctionReturn(func_name.clone()),
            ) {
                type_literals.push(type_literal);
            }
        }

        // Extract parameter type literals
        for param in &func.params.items {
            if let Some(param_name) = self.get_parameter_name(param) {
                if let Some(type_annotation) = &param.type_annotation {
                    if let Some(type_literal) = self.extract_type_literal_from_ts_type(
                        &type_annotation.type_annotation,
                        TypeLiteralContext::FunctionParameter(
                            func_name.clone(),
                            param_name,
                        ),
                    ) {
                        type_literals.push(type_literal);
                    }
                }
            }
        }

        // Recursively walk into the function body so type literals attached
        // to nested declarations don't disappear into the parent scope.
        if let Some(body) = &func.body {
            for stmt in &body.statements {
                self.extract_type_literals_from_statement(stmt, type_literals);
            }
        }
    }

    fn extract_from_variable_declaration(
        &self,
        var_decl: &oxc_ast::ast::VariableDeclaration,
        type_literals: &mut Vec<TypeLiteralDefinition>,
    ) {
        for declarator in &var_decl.declarations {
            if let Some(var_name) = self.get_variable_name(declarator) {
                // Check for variable type annotation
                if let Some(type_annotation) = &declarator.type_annotation {
                    if let Some(type_literal) = self.extract_type_literal_from_ts_type(
                        &type_annotation.type_annotation,
                        TypeLiteralContext::VariableDeclaration(var_name.clone()),
                    ) {
                        type_literals.push(type_literal);
                    }
                }

                // Check for arrow function in variable initialization
                if let Some(init) = &declarator.init {
                    self.extract_type_literals_from_initializer(init, &var_name, type_literals);
                }
            }
        }
    }

    fn extract_type_literals_from_initializer(
        &self,
        expr: &Expression,
        var_name: &str,
        type_literals: &mut Vec<TypeLiteralDefinition>,
    ) {
        if let Some(type_literal) = self.extract_type_literal_from_expression(
            expr,
            TypeLiteralContext::ArrowFunctionReturn(var_name.to_string()),
        ) {
            type_literals.push(type_literal);
        }

        // Also walk parameters and the body of arrow / function expressions
        // assigned to a variable. Previously the only signal we extracted
        // from `const make = (req: { id: string }) => ...` was the return
        // type, so a parameter type literal common to multiple arrow
        // functions was never compared.
        match expr {
            Expression::ArrowFunctionExpression(arrow) => {
                for param in &arrow.params.items {
                    if let Some(param_name) = self.get_parameter_name(param) {
                        if let Some(type_annotation) = &param.type_annotation {
                            if let Some(type_literal) = self.extract_type_literal_from_ts_type(
                                &type_annotation.type_annotation,
                                TypeLiteralContext::FunctionParameter(
                                    var_name.to_string(),
                                    param_name,
                                ),
                            ) {
                                type_literals.push(type_literal);
                            }
                        }
                    }
                }
                if !arrow.expression {
                    for stmt in &arrow.body.statements {
                        self.extract_type_literals_from_statement(stmt, type_literals);
                    }
                }
            }
            Expression::FunctionExpression(func) => {
                // Function expressions assigned to a variable carry the
                // same shape information as a function declaration: an
                // explicit return type literal (`function (): { id: string }
                // { ... }`) needs to feed the comparison pool too. The
                // arrow form already did this through its return type, so
                // catching it here keeps arrow and function-expression
                // coverage symmetric.
                if let Some(return_type) = &func.return_type {
                    if let Some(type_literal) = self.extract_type_literal_from_ts_type(
                        &return_type.type_annotation,
                        TypeLiteralContext::FunctionReturn(var_name.to_string()),
                    ) {
                        type_literals.push(type_literal);
                    }
                }
                for param in &func.params.items {
                    if let Some(param_name) = self.get_parameter_name(param) {
                        if let Some(type_annotation) = &param.type_annotation {
                            if let Some(type_literal) = self.extract_type_literal_from_ts_type(
                                &type_annotation.type_annotation,
                                TypeLiteralContext::FunctionParameter(
                                    var_name.to_string(),
                                    param_name,
                                ),
                            ) {
                                type_literals.push(type_literal);
                            }
                        }
                    }
                }
                if let Some(body) = &func.body {
                    for stmt in &body.statements {
                        self.extract_type_literals_from_statement(stmt, type_literals);
                    }
                }
            }
            _ => {}
        }
    }

    fn extract_type_literal_from_ts_type(
        &self,
        ts_type: &TSType,
        context: TypeLiteralContext,
    ) -> Option<TypeLiteralDefinition> {
        match ts_type {
            TSType::TSTypeLiteral(type_literal) => {
                let properties = self.extract_interface_properties(&type_literal.members);
                let start_line = self.get_line_number(type_literal.span.start as usize);
                let end_line = self.get_line_number(type_literal.span.end as usize);

                Some(TypeLiteralDefinition {
                    name: self.get_context_name(&context),
                    context,
                    properties,
                    start_line,
                    end_line,
                    file_path: self.file_path.clone(),
                })
            }
            _ => None,
        }
    }

    fn extract_type_literal_from_expression(
        &self,
        expr: &Expression,
        context: TypeLiteralContext,
    ) -> Option<TypeLiteralDefinition> {
        match expr {
            Expression::ArrowFunctionExpression(arrow_func) => {
                if let Some(return_type) = &arrow_func.return_type {
                    self.extract_type_literal_from_ts_type(&return_type.type_annotation, context)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn get_context_name(&self, context: &TypeLiteralContext) -> String {
        match context {
            TypeLiteralContext::FunctionReturn(name) => format!("{name} (return type)"),
            TypeLiteralContext::FunctionParameter(func_name, param_name) => {
                format!("{func_name} (parameter: {param_name})")
            }
            TypeLiteralContext::VariableDeclaration(name) => format!("{name} (variable)"),
            TypeLiteralContext::ArrowFunctionReturn(name) => format!("{} (arrow function)", name),
        }
    }

    fn get_parameter_name(&self, param: &oxc_ast::ast::FormalParameter) -> Option<String> {
        match &param.pattern {
            oxc_ast::ast::BindingPattern::BindingIdentifier(ident) => Some(ident.name.to_string()),
            _ => None,
        }
    }

    fn get_variable_name(&self, declarator: &VariableDeclarator) -> Option<String> {
        match &declarator.id {
            oxc_ast::ast::BindingPattern::BindingIdentifier(ident) => Some(ident.name.to_string()),
            _ => None,
        }
    }
}

/// Extract types from source code
pub fn extract_types_from_code(
    source_text: &str,
    file_path: &str,
) -> Result<Vec<TypeDefinition>, String> {
    let extractor = TypeExtractor::new(source_text.to_string(), file_path.to_string());
    extractor.extract_types()
}

/// Extract types from multiple files
pub fn extract_types_from_files(
    files: &[(String, String)], // (file_path, content)
) -> HashMap<String, Vec<TypeDefinition>> {
    let mut results = HashMap::new();

    for (file_path, content) in files {
        match extract_types_from_code(content, file_path) {
            Ok(types) => {
                results.insert(file_path.clone(), types);
            }
            Err(err) => {
                eprintln!("Failed to extract types from {}: {}", file_path, err);
                results.insert(file_path.clone(), Vec::new());
            }
        }
    }

    results
}

/// Extract type literals from source code
pub fn extract_type_literals_from_code(
    source_text: &str,
    file_path: &str,
) -> Result<Vec<TypeLiteralDefinition>, String> {
    let extractor = TypeExtractor::new(source_text.to_string(), file_path.to_string());
    extractor.extract_type_literals()
}

/// Extract type literals from multiple files
pub fn extract_type_literals_from_files(
    files: &[(String, String)], // (file_path, content)
) -> HashMap<String, Vec<TypeLiteralDefinition>> {
    let mut results = HashMap::new();

    for (file_path, content) in files {
        match extract_type_literals_from_code(content, file_path) {
            Ok(type_literals) => {
                results.insert(file_path.clone(), type_literals);
            }
            Err(err) => {
                eprintln!("Failed to extract type literals from {}: {}", file_path, err);
                results.insert(file_path.clone(), Vec::new());
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_interface() {
        let source = r#"
interface User {
    id: string;
    name: string;
    age?: number;
    readonly email: string;
}
"#;

        let types = extract_types_from_code(source, "test.ts").unwrap();
        assert_eq!(types.len(), 1);

        let user_type = &types[0];
        assert_eq!(user_type.name, "User");
        assert_eq!(user_type.kind, TypeKind::Interface);
        assert_eq!(user_type.properties.len(), 4);

        let id_prop = &user_type.properties[0];
        assert_eq!(id_prop.name, "id");
        assert_eq!(id_prop.type_annotation, "string");
        assert!(!id_prop.optional);
        assert!(!id_prop.readonly);

        let age_prop = &user_type.properties[2];
        assert_eq!(age_prop.name, "age");
        assert_eq!(age_prop.type_annotation, "number");
        assert!(age_prop.optional);

        let email_prop = &user_type.properties[3];
        assert_eq!(email_prop.name, "email");
        assert!(email_prop.readonly);
    }

    #[test]
    fn test_extract_type_alias() {
        let source = r#"
type Status = "active" | "inactive" | "pending";

type User = {
    id: string;
    name: string;
};
"#;

        let types = extract_types_from_code(source, "test.ts").unwrap();
        assert_eq!(types.len(), 2);

        let status_type = &types[0];
        assert_eq!(status_type.name, "Status");
        assert_eq!(status_type.kind, TypeKind::TypeAlias);

        let user_type = &types[1];
        assert_eq!(user_type.name, "User");
        assert_eq!(user_type.kind, TypeKind::TypeAlias);
        assert_eq!(user_type.properties.len(), 2);
    }

    #[test]
    fn test_extract_generic_interface() {
        let source = r#"
interface Container<T> {
    value: T;
}
"#;

        let types = extract_types_from_code(source, "test.ts").unwrap();
        assert_eq!(types.len(), 1);

        let container_type = &types[0];
        assert_eq!(container_type.name, "Container");
        assert_eq!(container_type.generics, vec!["T"]);
    }

    #[test]
    fn test_extract_interface_with_extends() {
        let source = r#"
interface BaseUser {
    id: string;
}

interface User extends BaseUser {
    name: string;
}
"#;

        let types = extract_types_from_code(source, "test.ts").unwrap();
        assert_eq!(types.len(), 2);

        let user_type = &types[1];
        assert_eq!(user_type.name, "User");
        assert_eq!(user_type.extends, vec!["BaseUser"]);
    }

    #[test]
    fn test_extract_types_marks_similarity_ignore_directives() {
        let source = r#"
// similarity-ignore
interface IgnoredUser {
    id: string;
}

type ActiveUser = {
    id: string;
};

/* similarity-ignore */
type IgnoredAlias = {
    value: number;
};
"#;

        let types = extract_types_from_code(source, "test.ts").unwrap();

        let ignored_user = types.iter().find(|t| t.name == "IgnoredUser").unwrap();
        assert!(ignored_user.has_ignore_directive);

        let active_user = types.iter().find(|t| t.name == "ActiveUser").unwrap();
        assert!(!active_user.has_ignore_directive);

        let ignored_alias = types.iter().find(|t| t.name == "IgnoredAlias").unwrap();
        assert!(ignored_alias.has_ignore_directive);
    }

    #[test]
    fn qualified_type_references_keep_their_namespace() {
        let code = r"
export interface WithQualified {
  view: React.FC;
}

export interface WithBare {
  view: FC;
}
";
        let types = extract_types_from_code(code, "test.ts").unwrap();
        let qualified = types.iter().find(|t| t.name == "WithQualified").unwrap();
        let bare = types.iter().find(|t| t.name == "WithBare").unwrap();
        assert_eq!(qualified.properties[0].type_annotation, "React.FC");
        assert_eq!(bare.properties[0].type_annotation, "FC");
        assert_ne!(
            qualified.properties[0].type_annotation,
            bare.properties[0].type_annotation
        );
    }

    #[test]
    fn generic_aliases_substitute_params_into_fallback_bodies() {
        // `type Box<T> = T[]` and `type Bag<U> = U[]` must extract the
        // same positional body, not `T[]` vs `U[]`.
        let code = r"
export type Box<T> = T[];
export type Bag<U> = U[];
";
        let types = extract_types_from_code(code, "test.ts").unwrap();
        let bodies: Vec<&str> = types
            .iter()
            .map(|t| t.properties[0].type_annotation.as_str())
            .collect();
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0], bodies[1], "positional substitution must unify {bodies:?}");
        assert!(bodies[0].contains("#0"));
    }
}
