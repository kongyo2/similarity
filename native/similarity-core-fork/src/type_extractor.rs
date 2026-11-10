//! Type discovery: interfaces, type aliases and, on request, the anonymous
//! object types of function signatures and variable annotations.

use oxc_ast::ast::{
    BindingPattern, Declaration, ExportDefaultDeclarationKind, Expression, FormalParameter,
    FormalParameters, Function, Program, PropertyKey, Statement, TSInterfaceDeclaration,
    TSInterfaceHeritage, TSLiteral, TSMethodSignature, TSMethodSignatureKind, TSModuleDeclaration,
    TSModuleDeclarationBody, TSPropertySignature, TSSignature, TSTupleElement, TSType,
    TSTypeAliasDeclaration, TSTypeAnnotation, TSTypeName, TSTypeOperatorOperator,
    TSTypeParameterDeclaration, VariableDeclaration, VariableDeclarator,
};
use oxc_span::GetSpan;
use std::collections::HashMap;

use crate::ignore_directive::has_ignore_directive_above;
use crate::module_scope::{module_qualifier, qualified_name};
use crate::source::{slice, with_checked_program, LineIndex};

/// Render a possibly-qualified type name (`React.FC`, `A.B.C`) as its
/// full dotted path — collapsing to the rightmost segment would let
/// `React.FC` compare equal to any other namespace's `FC`.
fn render_ts_type_name(name: &TSTypeName) -> String {
    match name {
        TSTypeName::IdentifierReference(ident) => ident.name.as_str().to_string(),
        TSTypeName::QualifiedName(qualified) => {
            format!("{}.{}", render_ts_type_name(&qualified.left), qualified.right.name.as_str())
        }
        TSTypeName::ThisExpression(_) => "this".to_string(),
    }
}

/// The canonical spelling of a string-literal type: its cooked value in
/// double quotes, with `"` and `\` escaped (and line breaks and tabs
/// spelled as escapes). Escaping keeps a literal's boundaries unambiguous:
/// `'say "yes"'` spells `"say \"yes\""`, one literal, while a literal union
/// such as `"a" | "b"`, which also starts and ends with a quote, is not.
pub(crate) fn quote_string_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Collapse runs of whitespace down to a single space, but leave whitespace
/// inside string and template literals untouched. Pure
/// `split_whitespace().join(" ")` would equate `type A = "a b"` with
/// `type A = "a  b"` (different cooked string values) because it ignores
/// literal boundaries, so we walk the bytes by hand and only collapse
/// outside quoted regions.
///
/// Behavior notes:
/// * Single- and double-quoted strings keep their content verbatim,
///   honoring `\"`, `\\`, … escapes, but are spelled the way
///   [`quote_string_literal`] spells a string-literal type: in double
///   quotes, with an inner `"` escaped and `\'` unescaped. Quote style is
///   formatting: `type A = '1'` and `type A = "1"` are the same literal.
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
                if ch == '\\' {
                    match chars.next() {
                        // `'` needs no escape inside double quotes.
                        Some('\'') => out.push('\''),
                        Some(next) => {
                            out.push('\\');
                            out.push(next);
                        }
                        None => out.push('\\'),
                    }
                } else if ch == *q {
                    out.push('"');
                    stack.pop();
                } else if ch == '"' {
                    // An unescaped `"` inside a single-quoted string.
                    out.push_str("\\\"");
                } else {
                    out.push(ch);
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
                    out.push('"');
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
                    out.push('"');
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// An anonymous object type found in a signature or a variable annotation.
#[derive(Debug, Clone)]
pub struct TypeLiteralDefinition {
    /// Where the literal sits, spelled for the report (see
    /// [`TypeLiteralContext::display_name`]).
    pub name: String,
    pub context: TypeLiteralContext,
    pub properties: Vec<PropertyDefinition>,
    pub start_line: usize,
    pub end_line: usize,
    pub file_path: String,
    /// Whether the declaration the literal belongs to is under a
    /// `similarity-ignore` directive.
    pub has_ignore_directive: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeLiteralContext {
    /// Return type of the named function.
    FunctionReturn(String),
    /// Parameter of a function: function name, parameter name.
    FunctionParameter(String, String),
    /// Annotation of the named variable.
    VariableDeclaration(String),
    /// Return type of the arrow function bound to the named variable.
    ArrowFunctionReturn(String),
}

impl TypeLiteralContext {
    /// The name a literal in this position is reported under.
    #[must_use]
    pub fn display_name(&self) -> String {
        match self {
            TypeLiteralContext::FunctionReturn(name) => format!("{name} (return type)"),
            TypeLiteralContext::FunctionParameter(func_name, param_name) => {
                format!("{func_name} (parameter: {param_name})")
            }
            TypeLiteralContext::VariableDeclaration(name) => format!("{name} (variable)"),
            TypeLiteralContext::ArrowFunctionReturn(name) => format!("{name} (arrow function)"),
        }
    }
}

/// Extracts the type declarations of one source file, and renders type
/// annotations the way type mode compares them.
pub struct TypeExtractor<'s> {
    source_text: &'s str,
    file_path: &'s str,
    lines: &'s LineIndex,
}

impl<'s> TypeExtractor<'s> {
    #[must_use]
    pub fn new(source_text: &'s str, file_path: &'s str, lines: &'s LineIndex) -> Self {
        Self { source_text, file_path, lines }
    }

    fn line_of(&self, offset: u32) -> usize {
        self.lines.line_of(offset) as usize
    }

    /// Whether a `similarity-ignore` directive sits right above the line
    /// `offset` is on.
    fn directive_above(&self, offset: u32) -> bool {
        has_ignore_directive_above(self.source_text, self.lines, self.lines.line_of(offset))
    }

    /// Every interface and type alias of the file, namespaces included.
    #[must_use]
    pub fn extract_types(&self, program: &Program) -> Vec<TypeDefinition> {
        let mut types = Vec::new();
        for stmt in &program.body {
            self.extract_types_from_statement(stmt, &mut types, "", false);
        }
        types
    }

    /// `qualifier` is the dotted path of the enclosing `namespace`/`module`
    /// blocks (`""` at the top level); see [`crate::module_scope`].
    /// `ignored` says whether an enclosing declaration carries the
    /// `similarity-ignore` directive.
    fn extract_types_from_statement(
        &self,
        stmt: &Statement,
        types: &mut Vec<TypeDefinition>,
        qualifier: &str,
        ignored: bool,
    ) {
        // A directive above the statement covers what it declares, the
        // contents of a namespace included.
        let ignored = ignored || self.directive_above(stmt.span().start);
        match stmt {
            Statement::TSInterfaceDeclaration(interface) => {
                types.push(self.extract_interface(interface, qualifier, ignored));
            }
            Statement::TSTypeAliasDeclaration(type_alias) => {
                types.push(self.extract_type_alias(type_alias, qualifier, ignored));
            }
            Statement::TSModuleDeclaration(module) => {
                self.extract_types_from_module(module, types, qualifier, ignored);
            }
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::TSInterfaceDeclaration(interface)) => {
                    types.push(self.extract_interface(interface, qualifier, ignored));
                }
                Some(Declaration::TSTypeAliasDeclaration(type_alias)) => {
                    types.push(self.extract_type_alias(type_alias, qualifier, ignored));
                }
                Some(Declaration::TSModuleDeclaration(module)) => {
                    self.extract_types_from_module(module, types, qualifier, ignored);
                }
                _ => {}
            },
            // `export default interface Foo { … }` is a declaration like
            // any other.
            Statement::ExportDefaultDeclaration(export) => {
                if let ExportDefaultDeclarationKind::TSInterfaceDeclaration(interface) =
                    &export.declaration
                {
                    types.push(self.extract_interface(interface, qualifier, ignored));
                }
            }
            _ => {}
        }
    }

    /// `namespace`/`module` blocks (including `declare namespace` in
    /// declaration files) are ordinary declaration scopes for the
    /// interfaces and aliases inside them; they used to be skipped. Their
    /// members are reported under the qualified name (`Legacy.Options`).
    fn extract_types_from_module(
        &self,
        module: &TSModuleDeclaration,
        types: &mut Vec<TypeDefinition>,
        qualifier: &str,
        ignored: bool,
    ) {
        let qualifier = module_qualifier(&module.id, qualifier);
        match &module.body {
            Some(TSModuleDeclarationBody::TSModuleDeclaration(inner)) => {
                self.extract_types_from_module(inner, types, &qualifier, ignored);
            }
            Some(TSModuleDeclarationBody::TSModuleBlock(block)) => {
                for stmt in &block.body {
                    self.extract_types_from_statement(stmt, types, &qualifier, ignored);
                }
            }
            None => {}
        }
    }

    /// The anonymous object types of the file's function parameters, return
    /// types and annotated variables.
    #[must_use]
    pub fn extract_type_literals(&self, program: &Program) -> Vec<TypeLiteralDefinition> {
        let mut type_literals = Vec::new();
        for stmt in &program.body {
            self.extract_type_literals_from_statement(stmt, &mut type_literals, false);
        }
        type_literals
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
        for property in properties.iter_mut() {
            property.type_annotation =
                Self::substitute_generic_tokens(&property.type_annotation, generics);
        }
    }

    /// The token replacement behind [`Self::substitute_generic_params`],
    /// for one rendered type.
    pub(crate) fn substitute_generic_tokens(input: &str, generics: &[String]) -> String {
        Self::substitute_generic_tokens_with(input, generics, "#")
    }

    /// [`Self::substitute_generic_tokens`] with a caller-chosen marker, so
    /// nested generic scopes (a generic method inside a generic class) get
    /// distinct positional tokens.
    ///
    /// Only type references are substituted. The text of string and
    /// template literals (`"Node"`, `` `on${T}` `` outside its `${…}`) is
    /// data, and a segment of a qualified name (`ts.Node`, `Node.Kind`)
    /// names a namespace member: with a type parameter `Node`, rewriting
    /// those made `kind: "Node"` and `root: ts.Node` differ from the same
    /// members under a type parameter of another name.
    pub(crate) fn substitute_generic_tokens_with(
        input: &str,
        generics: &[String],
        marker: &str,
    ) -> String {
        #[derive(Clone, Copy)]
        enum Mode {
            Code,
            Quote(char),
            Template,
            /// Inside `${…}` of a template literal, with its brace depth.
            TemplateExpression(u32),
        }

        if generics.is_empty() {
            return input.to_string();
        }
        let chars: Vec<char> = input.chars().collect();
        let is_token_char = |ch: char| ch.is_alphanumeric() || ch == '_' || ch == '$';
        // A `.` joining two name segments, as opposed to a spread `...`.
        let is_qualifier_dot = |position: usize| {
            chars.get(position) == Some(&'.')
                && (position == 0 || chars[position - 1] != '.')
                && chars.get(position + 1) != Some(&'.')
        };
        let mut modes = vec![Mode::Code];
        let mut result = String::with_capacity(input.len());
        let mut index = 0;
        while index < chars.len() {
            let ch = chars[index];
            match modes.last().copied().unwrap_or(Mode::Code) {
                Mode::Quote(quote) => {
                    result.push(ch);
                    if ch == '\\' {
                        if let Some(&escaped) = chars.get(index + 1) {
                            result.push(escaped);
                            index += 1;
                        }
                    } else if ch == quote {
                        modes.pop();
                    }
                    index += 1;
                }
                Mode::Template => {
                    result.push(ch);
                    if ch == '\\' {
                        if let Some(&escaped) = chars.get(index + 1) {
                            result.push(escaped);
                            index += 1;
                        }
                    } else if ch == '`' {
                        modes.pop();
                    } else if ch == '$' && chars.get(index + 1) == Some(&'{') {
                        result.push('{');
                        index += 1;
                        modes.push(Mode::TemplateExpression(1));
                    }
                    index += 1;
                }
                Mode::Code | Mode::TemplateExpression(_) => {
                    if is_token_char(ch) {
                        let start = index;
                        while index < chars.len() && is_token_char(chars[index]) {
                            index += 1;
                        }
                        let token: String = chars[start..index].iter().collect();
                        let qualified =
                            (start > 0 && is_qualifier_dot(start - 1)) || is_qualifier_dot(index);
                        match generics.iter().position(|generic| *generic == token) {
                            Some(position) if !qualified => {
                                result.push_str(&format!("{marker}{position}"));
                            }
                            _ => result.push_str(&token),
                        }
                        continue;
                    }
                    match ch {
                        '"' | '\'' => modes.push(Mode::Quote(ch)),
                        '`' => modes.push(Mode::Template),
                        '{' => {
                            if let Some(Mode::TemplateExpression(depth)) = modes.last_mut() {
                                *depth += 1;
                            }
                        }
                        '}' => {
                            if let Some(Mode::TemplateExpression(depth)) = modes.last_mut() {
                                *depth -= 1;
                                if *depth == 0 {
                                    modes.pop();
                                }
                            }
                        }
                        _ => {}
                    }
                    result.push(ch);
                    index += 1;
                }
            }
        }
        result
    }

    /// `qualifier` is the dotted path of the enclosing `namespace`/`module`
    /// blocks (`""` at the top level); see [`crate::module_scope`].
    fn extract_interface(
        &self,
        interface: &TSInterfaceDeclaration,
        qualifier: &str,
        ignored: bool,
    ) -> TypeDefinition {
        let name = qualified_name(qualifier, interface.id.name.as_str());
        let start_line = self.line_of(interface.span.start);
        let end_line = self.line_of(interface.span.end);

        let mut properties = self.extract_interface_properties(&interface.body.body);
        let generics = self.extract_generics(interface.type_parameters.as_ref());
        Self::substitute_generic_params(&mut properties, &generics);
        let extends = self.extract_extends(&interface.extends, &generics);

        TypeDefinition {
            name,
            kind: TypeKind::Interface,
            properties,
            generics,
            extends,
            start_line,
            end_line,
            file_path: self.file_path.to_string(),
            has_ignore_directive: ignored || self.directive_above(interface.span.start),
        }
    }

    fn extract_type_alias(
        &self,
        type_alias: &TSTypeAliasDeclaration,
        qualifier: &str,
        ignored: bool,
    ) -> TypeDefinition {
        let name = qualified_name(qualifier, type_alias.id.name.as_str());
        let start_line = self.line_of(type_alias.span.start);
        let end_line = self.line_of(type_alias.span.end);

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

        TypeDefinition {
            name,
            kind: TypeKind::TypeAlias,
            properties,
            generics,
            extends: Vec::new(), // Type aliases don't have extends
            start_line,
            end_line,
            file_path: self.file_path.to_string(),
            has_ignore_directive: ignored || self.directive_above(type_alias.span.start),
        }
    }

    /// A stable textual signature for the body of a non-object type alias:
    /// what the user wrote, modulo whitespace and comments outside of
    /// string and template literals. Falls back to the structural renderer
    /// for a body whose text cannot be recovered.
    fn extract_type_body_signature(&self, ts_type: &TSType) -> String {
        match self.collapsed_source(ts_type) {
            Some(collapsed) => collapsed,
            None => self.extract_type_string(ts_type),
        }
    }

    /// The source text of a type node with whitespace and comments
    /// normalized; `None` when the span is empty or collapses to nothing.
    fn collapsed_source(&self, ts_type: &TSType) -> Option<String> {
        let collapsed =
            collapse_whitespace_outside_strings(slice(self.source_text, ts_type.span())?);
        (!collapsed.is_empty()).then_some(collapsed)
    }

    /// The members of an interface or object type, in declaration order.
    ///
    /// A `get`/`set` pair is one writable property. Overloads — a method
    /// declared with several signatures, or several call signatures —
    /// share a name; each keeps a member of its own, numbered in
    /// declaration order (`on`, `on#2`, …). Members are compared by name,
    /// and of several members with one name only the last used to count.
    fn extract_interface_properties(&self, signatures: &[TSSignature]) -> Vec<PropertyDefinition> {
        let mut properties: Vec<PropertyDefinition> = Vec::new();
        // Where the property of each accessor seen so far sits.
        let mut accessors: HashMap<String, usize> = HashMap::new();
        let mut declarations: HashMap<String, usize> = HashMap::new();
        for signature in signatures {
            let Some(mut property) = self.extract_member(signature) else {
                continue;
            };
            let is_accessor = matches!(
                signature,
                TSSignature::TSMethodSignature(method)
                    if method.kind != TSMethodSignatureKind::Method
            );
            if is_accessor {
                if let Some(&position) = accessors.get(&property.name) {
                    properties[position].readonly = false;
                    continue;
                }
                accessors.insert(property.name.clone(), properties.len());
            }
            let count = declarations.entry(property.name.clone()).or_insert(0);
            *count += 1;
            if *count > 1 {
                property.name = format!("{}#{count}", property.name);
            }
            properties.push(property);
        }
        properties
    }

    /// One member of an interface or object type, as a named property.
    /// Members without a property name get a synthetic one that no
    /// identifier can spell: `[index: K]` for an index signature, `<call>`
    /// and `<new>` for call and construct signatures.
    fn extract_member(&self, signature: &TSSignature) -> Option<PropertyDefinition> {
        match signature {
            TSSignature::TSPropertySignature(prop_sig) => {
                self.extract_property_from_signature(prop_sig)
            }
            TSSignature::TSMethodSignature(method_sig) => {
                self.extract_method_from_signature(method_sig)
            }
            // `{ [key: string]: number }`: keyed by the index kind, so the
            // VALUE type participates in comparison.
            TSSignature::TSIndexSignature(index_sig) => {
                let key_type = index_sig.parameters.first().map_or_else(
                    || "string".to_string(),
                    |parameter| {
                        self.extract_type_string(&parameter.type_annotation.type_annotation)
                    },
                );
                Some(PropertyDefinition {
                    name: format!("[index: {key_type}]"),
                    type_annotation: self
                        .extract_type_string(&index_sig.type_annotation.type_annotation),
                    optional: false,
                    readonly: index_sig.readonly,
                })
            }
            // `interface Handler { (request: Request): Response }` and
            // `interface Factory { new (options: Options): Widget }` are all
            // signature. Both used to be dropped, which left such an
            // interface looking empty: any two of them compared as two
            // empty shapes, on their names alone.
            TSSignature::TSCallSignatureDeclaration(call_sig) => Some(PropertyDefinition {
                name: "<call>".to_string(),
                type_annotation: self
                    .function_type_string(&call_sig.params, call_sig.return_type.as_deref()),
                optional: false,
                readonly: false,
            }),
            TSSignature::TSConstructSignatureDeclaration(construct_sig) => {
                Some(PropertyDefinition {
                    name: "<new>".to_string(),
                    type_annotation: self.function_type_string(
                        &construct_sig.params,
                        construct_sig.return_type.as_deref(),
                    ),
                    optional: false,
                    readonly: false,
                })
            }
        }
    }

    /// The name of a member key: an identifier or string literal by value,
    /// a number by its source text, a computed key (`[Symbol.iterator]`) by
    /// its bracketed source text. Numeric and computed keys used to drop
    /// their member.
    fn member_key_name(&self, key: &PropertyKey, computed: bool) -> Option<String> {
        Some(match key {
            PropertyKey::StaticIdentifier(ident) => ident.name.as_str().to_string(),
            PropertyKey::StringLiteral(str_lit) => str_lit.value.as_str().to_string(),
            other => {
                let text =
                    collapse_whitespace_outside_strings(slice(self.source_text, other.span())?);
                if computed {
                    format!("[{text}]")
                } else {
                    text
                }
            }
        })
    }

    /// `(params) => return type`, the spelling of a method or call
    /// signature; a signature without a return type returns `void`.
    fn function_type_string(
        &self,
        params: &FormalParameters,
        return_type: Option<&TSTypeAnnotation>,
    ) -> String {
        let return_type = return_type
            .map_or_else(|| "void".to_string(), |rt| self.extract_type_string(&rt.type_annotation));
        format!("({}) => {return_type}", self.extract_function_params(params))
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
        Some(PropertyDefinition {
            name: self.member_key_name(&prop_sig.key, prop_sig.computed)?,
            type_annotation: prop_sig.type_annotation.as_ref().map_or_else(
                || "any".to_string(),
                |ta| self.extract_type_string(&ta.type_annotation),
            ),
            optional: prop_sig.optional,
            readonly: prop_sig.readonly,
        })
    }

    fn extract_method_from_signature(
        &self,
        method_sig: &TSMethodSignature,
    ) -> Option<PropertyDefinition> {
        let name = self.member_key_name(&method_sig.key, method_sig.computed)?;
        let type_annotation = match method_sig.kind {
            TSMethodSignatureKind::Method => {
                self.function_type_string(&method_sig.params, method_sig.return_type.as_deref())
            }
            // An accessor is a property seen from outside: `get size():
            // number` reads like `readonly size: number`, and a setter's
            // parameter is the property's type.
            TSMethodSignatureKind::Get => method_sig.return_type.as_ref().map_or_else(
                || "any".to_string(),
                |rt| self.extract_type_string(&rt.type_annotation),
            ),
            TSMethodSignatureKind::Set => method_sig
                .params
                .items
                .first()
                .and_then(|param| param.type_annotation.as_ref())
                .map_or_else(
                    || "any".to_string(),
                    |ta| self.extract_type_string(&ta.type_annotation),
                ),
        };
        Some(PropertyDefinition {
            name,
            type_annotation,
            optional: method_sig.optional,
            readonly: method_sig.kind == TSMethodSignatureKind::Get,
        })
    }

    /// Faithful spelling for type nodes without a structural renderer
    /// (`typeof x`, conditional / mapped / template-literal types, `x is T`
    /// predicates, `import("./m").T`, …). Collapsing all of them onto one
    /// `unknown` token made every such property compare EQUAL — `guard: (v)
    /// => v is string` matched `guard: (v) => v is number`. The source text,
    /// with whitespace and comments normalized, keeps distinct spellings
    /// distinct.
    fn source_type_text(&self, ts_type: &TSType) -> String {
        self.collapsed_source(ts_type).unwrap_or_else(|| "unknown".to_string())
    }

    /// The type renderer behind type mode, shared with class mode so a class
    /// member's annotation is spelled exactly the way an interface member's
    /// is (qualified names, type arguments, literal and exotic types).
    pub(crate) fn render_type(&self, ts_type: &TSType) -> String {
        self.extract_type_string(ts_type)
    }

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
                    TSTypeName::IdentifierReference(ident) => ident.name.as_str().to_string(),
                    TSTypeName::QualifiedName(_) => {
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
                        let rendered: Vec<String> = args
                            .params
                            .iter()
                            .map(|param| self.extract_type_string(param))
                            .collect();
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
                        TSTupleElement::TSOptionalType(optional) => {
                            format!("{}?", self.extract_type_string(&optional.type_annotation))
                        }
                        TSTupleElement::TSRestType(rest) => {
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
                    TSTypeOperatorOperator::Keyof => format!("keyof {operand}"),
                    TSTypeOperatorOperator::Readonly => {
                        // `readonly T[]` is the same runtime shape as
                        // `ReadonlyArray<T>`; canonicalize the spelling.
                        format!("ReadonlyArray<{}>", operand.strip_suffix("[]").unwrap_or(&operand))
                    }
                    TSTypeOperatorOperator::Unique => format!("unique {operand}"),
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
                TSLiteral::StringLiteral(str_lit) => quote_string_literal(str_lit.value.as_str()),
                TSLiteral::NumericLiteral(num_lit) => num_lit.value.to_string(),
                TSLiteral::BooleanLiteral(bool_lit) => bool_lit.value.to_string(),
                _ => self.source_type_text(ts_type),
            },
            TSType::TSFunctionType(func_type) => {
                self.function_type_string(&func_type.params, Some(&func_type.return_type))
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
            _ => self.source_type_text(ts_type),
        }
    }

    /// A signature's parameter list as `name: type` pieces. An optional
    /// parameter keeps its `?` and the rest parameter its `...`: both are
    /// part of the call contract, and the rest parameter used to be dropped
    /// altogether, so `(...args: string[]) => void` spelled `() => void`.
    fn extract_function_params(&self, params: &FormalParameters) -> String {
        let annotation = |annotation: Option<&TSTypeAnnotation>| {
            annotation.map_or_else(
                || "any".to_string(),
                |ta| self.extract_type_string(&ta.type_annotation),
            )
        };
        let binding_name = |pattern: &'_ BindingPattern| match pattern {
            BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
            _ => "_".to_string(),
        };
        let mut pieces: Vec<String> = params
            .items
            .iter()
            .map(|param| {
                format!(
                    "{}{}: {}",
                    binding_name(&param.pattern),
                    if param.optional { "?" } else { "" },
                    annotation(param.type_annotation.as_deref())
                )
            })
            .collect();
        if let Some(rest) = &params.rest {
            pieces.push(format!(
                "...{}: {}",
                binding_name(&rest.rest.argument),
                annotation(rest.type_annotation.as_deref())
            ));
        }
        pieces.join(", ")
    }

    fn extract_generics(
        &self,
        type_params: Option<&oxc_allocator::Box<TSTypeParameterDeclaration>>,
    ) -> Vec<String> {
        if let Some(params) = type_params {
            params.params.iter().map(|param| param.name.name.as_str().to_string()).collect()
        } else {
            Vec::new()
        }
    }

    /// Heritage clauses rendered the way property annotations are —
    /// qualified names and type arguments included, generic parameters
    /// positional — so `extends Box<string>` and `extends Box<number>`
    /// are different contracts while `A<T> extends Box<T>` and
    /// `B<U> extends Box<U>` are the same one. Only a bare identifier
    /// used to survive; everything else was dropped.
    fn extract_extends(
        &self,
        heritage_clauses: &[TSInterfaceHeritage],
        generics: &[String],
    ) -> Vec<String> {
        heritage_clauses
            .iter()
            .map(|heritage| {
                let base = match &heritage.expression {
                    Expression::Identifier(ident) => ident.name.as_str().to_string(),
                    other => slice(self.source_text, other.span())
                        .map_or_else(|| "unknown".to_string(), collapse_whitespace_outside_strings),
                };
                let rendered = match &heritage.type_arguments {
                    Some(args) if !args.params.is_empty() => {
                        let rendered: Vec<String> = args
                            .params
                            .iter()
                            .map(|param| self.extract_type_string(param))
                            .collect();
                        format!("{base}<{}>", rendered.join(", "))
                    }
                    _ => base,
                };
                if generics.is_empty() {
                    rendered
                } else {
                    Self::substitute_generic_tokens(&rendered, generics)
                }
            })
            .collect()
    }

    /// `ignored` says whether an enclosing declaration carries the
    /// `similarity-ignore` directive.
    fn extract_type_literals_from_statement(
        &self,
        stmt: &Statement,
        type_literals: &mut Vec<TypeLiteralDefinition>,
        ignored: bool,
    ) {
        let ignored = ignored || self.directive_above(stmt.span().start);
        match stmt {
            Statement::FunctionDeclaration(func) => {
                self.extract_from_function(func, type_literals, ignored);
            }
            Statement::VariableDeclaration(var_decl) => {
                self.extract_from_variable_declaration(var_decl, type_literals, ignored);
            }
            // Exported declarations carry the same anonymous types as
            // un-exported ones.
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(Declaration::FunctionDeclaration(func)) => {
                    self.extract_from_function(func, type_literals, ignored);
                }
                Some(Declaration::VariableDeclaration(var_decl)) => {
                    self.extract_from_variable_declaration(var_decl, type_literals, ignored);
                }
                _ => {}
            },
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                    self.extract_from_function(func, type_literals, ignored);
                }
                // `export default (arg: { id: string }) => …` and
                // `export default function (arg: { id: string }) {…}` are
                // expression forms: walk them like a variable initializer.
                other => {
                    if let Some(expr) = other.as_expression() {
                        self.extract_type_literals_from_initializer(
                            expr,
                            "default",
                            type_literals,
                            ignored,
                        );
                    }
                }
            },
            _ => {}
        }
    }

    fn extract_from_function(
        &self,
        func: &Function,
        type_literals: &mut Vec<TypeLiteralDefinition>,
        ignored: bool,
    ) {
        let func_name =
            func.id.as_ref().map_or_else(|| "<anonymous>".to_string(), |id| id.name.to_string());
        self.extract_from_signature(
            &func_name,
            &func.params,
            func.return_type.as_ref().map(|annotation| &annotation.type_annotation),
            TypeLiteralContext::FunctionReturn(func_name.clone()),
            type_literals,
            ignored,
        );
        // Walk into the function body so type literals attached to nested
        // declarations don't disappear into the parent scope.
        if let Some(body) = &func.body {
            for stmt in &body.statements {
                self.extract_type_literals_from_statement(stmt, type_literals, ignored);
            }
        }
    }

    /// The type literals of one signature: the return type, then each
    /// named parameter's annotation.
    fn extract_from_signature(
        &self,
        owner: &str,
        params: &FormalParameters,
        return_type: Option<&TSType>,
        return_context: TypeLiteralContext,
        type_literals: &mut Vec<TypeLiteralDefinition>,
        ignored: bool,
    ) {
        if let Some(return_type) = return_type {
            type_literals.extend(self.type_literal_definition(
                return_type,
                return_context,
                ignored,
            ));
        }
        for param in &params.items {
            let (Some(param_name), Some(annotation)) =
                (Self::parameter_name(param), &param.type_annotation)
            else {
                continue;
            };
            type_literals.extend(self.type_literal_definition(
                &annotation.type_annotation,
                TypeLiteralContext::FunctionParameter(owner.to_string(), param_name),
                ignored,
            ));
        }
    }

    fn extract_from_variable_declaration(
        &self,
        var_decl: &VariableDeclaration,
        type_literals: &mut Vec<TypeLiteralDefinition>,
        ignored: bool,
    ) {
        for declarator in &var_decl.declarations {
            let Some(var_name) = Self::variable_name(declarator) else {
                continue;
            };
            if let Some(annotation) = &declarator.type_annotation {
                type_literals.extend(self.type_literal_definition(
                    &annotation.type_annotation,
                    TypeLiteralContext::VariableDeclaration(var_name.clone()),
                    ignored,
                ));
            }
            if let Some(init) = &declarator.init {
                self.extract_type_literals_from_initializer(
                    init,
                    &var_name,
                    type_literals,
                    ignored,
                );
            }
        }
    }

    /// The type literals of an arrow or function expression bound to
    /// `var_name`: its signature, then its body.
    fn extract_type_literals_from_initializer(
        &self,
        expr: &Expression,
        var_name: &str,
        type_literals: &mut Vec<TypeLiteralDefinition>,
        ignored: bool,
    ) {
        match expr {
            Expression::ArrowFunctionExpression(arrow) => {
                self.extract_from_signature(
                    var_name,
                    &arrow.params,
                    arrow.return_type.as_ref().map(|annotation| &annotation.type_annotation),
                    TypeLiteralContext::ArrowFunctionReturn(var_name.to_string()),
                    type_literals,
                    ignored,
                );
                if !arrow.expression {
                    for stmt in &arrow.body.statements {
                        self.extract_type_literals_from_statement(stmt, type_literals, ignored);
                    }
                }
            }
            Expression::FunctionExpression(func) => {
                self.extract_from_signature(
                    var_name,
                    &func.params,
                    func.return_type.as_ref().map(|annotation| &annotation.type_annotation),
                    TypeLiteralContext::FunctionReturn(var_name.to_string()),
                    type_literals,
                    ignored,
                );
                if let Some(body) = &func.body {
                    for stmt in &body.statements {
                        self.extract_type_literals_from_statement(stmt, type_literals, ignored);
                    }
                }
            }
            _ => {}
        }
    }

    /// The definition of `ts_type` when it is an object type literal.
    fn type_literal_definition(
        &self,
        ts_type: &TSType,
        context: TypeLiteralContext,
        ignored: bool,
    ) -> Option<TypeLiteralDefinition> {
        let TSType::TSTypeLiteral(type_literal) = ts_type else {
            return None;
        };
        Some(TypeLiteralDefinition {
            name: context.display_name(),
            context,
            properties: self.extract_interface_properties(&type_literal.members),
            start_line: self.line_of(type_literal.span.start),
            end_line: self.line_of(type_literal.span.end),
            file_path: self.file_path.to_string(),
            has_ignore_directive: ignored,
        })
    }

    fn parameter_name(param: &FormalParameter) -> Option<String> {
        match &param.pattern {
            BindingPattern::BindingIdentifier(ident) => Some(ident.name.to_string()),
            _ => None,
        }
    }

    fn variable_name(declarator: &VariableDeclarator) -> Option<String> {
        match &declarator.id {
            BindingPattern::BindingIdentifier(ident) => Some(ident.name.to_string()),
            _ => None,
        }
    }
}

/// Extract the interfaces and type aliases of a source file.
///
/// # Errors
///
/// Returns an error when the file cannot be analyzed: syntax errors, or
/// nesting beyond the limits of [`crate::source`].
pub fn extract_types_from_code(
    source_text: &str,
    file_path: &str,
) -> Result<Vec<TypeDefinition>, String> {
    let lines = LineIndex::new(source_text);
    with_checked_program(file_path, source_text, |program| {
        TypeExtractor::new(source_text, file_path, &lines).extract_types(program)
    })
}

/// Extract the anonymous object types of a source file's signatures and
/// variable annotations.
///
/// # Errors
///
/// Returns an error when the file cannot be analyzed (see
/// [`extract_types_from_code`]).
pub fn extract_type_literals_from_code(
    source_text: &str,
    file_path: &str,
) -> Result<Vec<TypeLiteralDefinition>, String> {
    let lines = LineIndex::new(source_text);
    with_checked_program(file_path, source_text, |program| {
        TypeExtractor::new(source_text, file_path, &lines).extract_type_literals(program)
    })
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
        assert_ne!(qualified.properties[0].type_annotation, bare.properties[0].type_annotation);
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
        let bodies: Vec<&str> =
            types.iter().map(|t| t.properties[0].type_annotation.as_str()).collect();
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0], bodies[1], "positional substitution must unify {bodies:?}");
        assert!(bodies[0].contains("#0"));
    }

    #[test]
    fn exotic_type_nodes_render_from_source() {
        // XT-N17 shape: these all used to collapse onto one `unknown`.
        let code = r"
declare const stringSource: { read(): string };
export interface Channel {
  guard: (value: unknown) => value is string;
  source: typeof stringSource;
  mapped: { [K in keyof Options]?: Options[K] };
  conditional: T extends string ? 'text' : 'other';
  template: `${string}px`;
}
";
        let types = extract_types_from_code(code, "test.ts").unwrap();
        let channel = types.iter().find(|t| t.name == "Channel").unwrap();
        let annotation = |name: &str| {
            channel.properties.iter().find(|p| p.name == name).unwrap().type_annotation.clone()
        };
        assert_eq!(annotation("guard"), "(value: unknown) => value is string");
        assert_eq!(annotation("source"), "typeof stringSource");
        assert_eq!(annotation("mapped"), "{ [K in keyof Options]?: Options[K] }");
        assert_eq!(annotation("conditional"), "T extends string ? \"text\" : \"other\"");
        assert_eq!(annotation("template"), "`${string}px`");
    }

    #[test]
    fn string_literals_take_one_canonical_spelling() {
        // Quote style is formatting: rendered annotations and source-text
        // bodies both spell a string literal in double quotes, an inner `"`
        // escaped, so `'1'` and `"1"` are one literal and `'say "yes"'`
        // stays one token.
        let code = r#"
export type SingleQuoted = '1';
export type DoubleQuoted = "1";
export type Quoted = 'say "yes"';
export type Apostrophe = 'it\'s';
export type Union = 'a' | "b";
export interface Consent {
  kind: 'say "yes"';
  note: "it's";
}
"#;
        let types = extract_types_from_code(code, "test.ts").unwrap();
        let body = |name: &str| {
            types.iter().find(|t| t.name == name).unwrap().properties[0].type_annotation.clone()
        };
        assert_eq!(body("SingleQuoted"), "\"1\"");
        assert_eq!(body("DoubleQuoted"), "\"1\"");
        assert_eq!(body("Quoted"), r#""say \"yes\"""#);
        assert_eq!(body("Apostrophe"), "\"it's\"");
        assert_eq!(body("Union"), "\"a\" | \"b\"");
        let consent = types.iter().find(|t| t.name == "Consent").unwrap();
        assert_eq!(consent.properties[0].type_annotation, r#""say \"yes\"""#);
        assert_eq!(consent.properties[1].type_annotation, "\"it's\"");
        assert_eq!(quote_string_literal("a\\b\n"), r#""a\\b\n""#);
    }

    #[test]
    fn heritage_keeps_type_arguments_and_qualified_bases() {
        let code = r"
interface StringBox extends Box<string>, Legacy.Tagged { id: number; }
interface Generic<T> extends Box<T> { id: number; }
export default interface Marker { id: number; }
";
        let types = extract_types_from_code(code, "test.ts").unwrap();
        assert_eq!(types[0].extends, vec!["Box<string>", "Legacy.Tagged"]);
        assert_eq!(types[1].extends, vec!["Box<#0>"]);
        assert_eq!(types[2].name, "Marker");
    }

    #[test]
    fn heritage_normalizes_after_positional_substitution() {
        // `Box<T | U>` substitutes to `Box<#0 | #1>` under `A<T, U>` and to
        // `Box<#1 | #0>` under `B<U, T>`; normalization sorts them again.
        let code = r"
interface A<T, U> extends Box<T | U> { id: number; }
interface B<U, T> extends Box<T | U> { id: number; }
";
        let types = extract_types_from_code(code, "test.ts").unwrap();
        let options = crate::type_normalizer::NormalizationOptions::default();
        let a = crate::type_normalizer::normalize_type(&types[0], &options);
        let b = crate::type_normalizer::normalize_type(&types[1], &options);
        assert_eq!(a.extends, vec!["Box<#0 | #1>"]);
        assert_eq!(a.extends, b.extends);
    }

    fn members(code: &str) -> Vec<(String, String, bool, bool)> {
        let types = extract_types_from_code(code, "test.ts").unwrap();
        types[0]
            .properties
            .iter()
            .map(|p| (p.name.clone(), p.type_annotation.clone(), p.optional, p.readonly))
            .collect()
    }

    fn member(name: &str, annotation: &str) -> (String, String, bool, bool) {
        (name.to_string(), annotation.to_string(), false, false)
    }

    #[test]
    fn call_and_construct_signatures_are_members() {
        // Both used to be dropped: an interface that is all signature was
        // an empty shape, the same as every other one.
        assert_eq!(
            members("interface Handler { (request: Request): Response; new (options: Options): Handler; name: string }"),
            [
                member("<call>", "(request: Request) => Response"),
                member("<new>", "(options: Options) => Handler"),
                member("name", "string"),
            ]
        );
        // Object types hold the same members.
        assert_eq!(
            members("type Factory = { new (options: Options): Widget }"),
            [member("<new>", "(options: Options) => Widget")]
        );
    }

    #[test]
    fn overloads_keep_a_member_each() {
        // Members are compared by name: of three signatures of one name,
        // only the last one used to count.
        assert_eq!(
            members(
                r#"interface Emitter {
    on(event: "open", listener: () => void): void;
    on(event: "close", listener: (code: number) => void): void;
    (strings: TemplateStringsArray): string;
    (content: string): string;
    (): string;
    off(event: string): void;
}"#
            ),
            [
                member("on", "(event: \"open\", listener: () => void) => void"),
                member("on#2", "(event: \"close\", listener: (code: number) => void) => void"),
                member("<call>", "(strings: TemplateStringsArray) => string"),
                member("<call>#2", "(content: string) => string"),
                member("<call>#3", "() => string"),
                member("off", "(event: string) => void"),
            ]
        );
    }

    #[test]
    fn accessors_are_properties() {
        // A getter reads like a readonly property; with its setter the
        // pair is one writable property, not two members of one name.
        assert_eq!(
            members("interface Box { get size(): number; get label(): string; set label(value: string); set limit(value: number) }"),
            [
                ("size".to_string(), "number".to_string(), false, true),
                member("label", "string"),
                member("limit", "number"),
            ]
        );
    }

    #[test]
    fn numeric_and_computed_keys_name_their_members() {
        // Both used to drop the member: a generated `{ 200: {…}; 404: {…} }`
        // response map was an empty shape.
        assert_eq!(
            members("type Responses = { 200: Ok; 404: NotFound; [Symbol.iterator]: Iterate; [ BRAND ]: Tag; 'quoted-key': string }"),
            [
                member("200", "Ok"),
                member("404", "NotFound"),
                member("[Symbol.iterator]", "Iterate"),
                member("[BRAND]", "Tag"),
                member("quoted-key", "string"),
            ]
        );
    }

    #[test]
    fn signatures_keep_optional_and_rest_parameters() {
        // The rest parameter used to be dropped, so `(...args: string[]) =>
        // void` spelled `() => void`; an optional parameter lost its `?`.
        assert_eq!(
            members("interface Logger { log(message: string, level?: Level, ...details: unknown[]): void; format: (...parts: string[]) => string; plain(): void }"),
            [
                member("log", "(message: string, level?: Level, ...details: unknown[]) => void"),
                member("format", "(...parts: string[]) => string"),
                member("plain", "() => void"),
            ]
        );
    }

    #[test]
    fn namespace_scoped_types_are_extracted() {
        let code = r"
export namespace Legacy {
  export interface Options {
    host: string;
  }
  export namespace Inner {
    export type Mode = 'a' | 'b';
  }
}
declare namespace Ambient {
  interface Hidden {
    id: number;
  }
}
";
        let types = extract_types_from_code(code, "test.ts").unwrap();
        let names: Vec<&str> = types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["Legacy.Options", "Legacy.Inner.Mode", "Ambient.Hidden"]);
    }
}
