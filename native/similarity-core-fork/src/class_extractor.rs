use oxc_allocator::Allocator;
use oxc_ast::ast::{ClassElement, MethodDefinitionKind, PropertyKey, Statement};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

use crate::ignore_directive::has_similarity_ignore_directive;

#[derive(Debug, Clone)]
pub struct ClassDefinition {
    pub name: String,
    pub properties: Vec<ClassProperty>,
    pub methods: Vec<ClassMethod>,
    pub constructor_params: Vec<String>,
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
    pub kind: MethodKind,
    /// Structural hash of the method's canonicalized params+body tree
    /// (name excluded, locals alpha-renamed). Two methods with the same
    /// fingerprint have behaviorally-equivalent bodies up to the
    /// canonicalizer's equivalences; `None` when the body could not be
    /// parsed standalone.
    pub body_fingerprint: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MethodKind {
    Method,
    Getter,
    Setter,
    Constructor,
}

struct ClassExtractor {
    source_text: String,
    file_path: String,
    line_offsets: Vec<usize>,
}

impl ClassExtractor {
    fn new(source_text: String, file_path: String) -> Self {
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

    fn extract_type_string(&self, type_annotation: &oxc_ast::ast::TSTypeAnnotation) -> String {
        use oxc_ast::ast::TSType;

        match &type_annotation.type_annotation {
            TSType::TSStringKeyword(_) => "string".to_string(),
            TSType::TSNumberKeyword(_) => "number".to_string(),
            TSType::TSBooleanKeyword(_) => "boolean".to_string(),
            TSType::TSAnyKeyword(_) => "any".to_string(),
            TSType::TSUnknownKeyword(_) => "unknown".to_string(),
            TSType::TSNeverKeyword(_) => "never".to_string(),
            TSType::TSVoidKeyword(_) => "void".to_string(),
            TSType::TSUndefinedKeyword(_) => "undefined".to_string(),
            TSType::TSNullKeyword(_) => "null".to_string(),
            TSType::TSArrayType(array) => {
                format!("{}[]", self.extract_type_string_from_ts_type(&array.element_type))
            }
            TSType::TSTypeReference(type_ref) => match &type_ref.type_name {
                oxc_ast::ast::TSTypeName::IdentifierReference(ident) => {
                    let base = ident.name.as_str();
                    if let Some(params) = &type_ref.type_arguments {
                        let param_strings: Vec<String> = params
                            .params
                            .iter()
                            .map(|p| self.extract_type_string_from_ts_type(p))
                            .collect();
                        format!("{}<{}>", base, param_strings.join(", "))
                    } else {
                        base.to_string()
                    }
                }
                _ => "unknown".to_string(),
            },
            TSType::TSUnionType(union) => {
                let types: Vec<String> =
                    union.types.iter().map(|t| self.extract_type_string_from_ts_type(t)).collect();
                types.join(" | ")
            }
            TSType::TSIntersectionType(intersection) => {
                let types: Vec<String> = intersection
                    .types
                    .iter()
                    .map(|t| self.extract_type_string_from_ts_type(t))
                    .collect();
                types.join(" & ")
            }
            TSType::TSFunctionType(func) => {
                let params = self.extract_function_params(&func.params);
                let return_type =
                    self.extract_type_string_from_ts_type(&func.return_type.type_annotation);
                format!("({}) => {}", params, return_type)
            }
            TSType::TSTypeLiteral(literal) => {
                let props: Vec<String> = literal
                    .members
                    .iter()
                    .filter_map(|member| {
                        if let oxc_ast::ast::TSSignature::TSPropertySignature(prop) = member {
                            let name = match &prop.key {
                                oxc_ast::ast::PropertyKey::StaticIdentifier(ident) => {
                                    ident.name.as_str().to_string()
                                }
                                oxc_ast::ast::PropertyKey::StringLiteral(str_lit) => {
                                    str_lit.value.as_str().to_string()
                                }
                                _ => return None,
                            };
                            let type_str = prop
                                .type_annotation
                                .as_ref()
                                .map(|ta| self.extract_type_string(ta))
                                .unwrap_or_else(|| "any".to_string());
                            let optional = if prop.optional { "?" } else { "" };
                            Some(format!("{}{}: {}", name, optional, type_str))
                        } else {
                            None
                        }
                    })
                    .collect();
                format!("{{ {} }}", props.join(", "))
            }
            _ => "any".to_string(),
        }
    }

    fn extract_type_string_from_ts_type(&self, ts_type: &oxc_ast::ast::TSType) -> String {
        use oxc_ast::ast::TSType;

        match ts_type {
            TSType::TSStringKeyword(_) => "string".to_string(),
            TSType::TSNumberKeyword(_) => "number".to_string(),
            TSType::TSBooleanKeyword(_) => "boolean".to_string(),
            TSType::TSAnyKeyword(_) => "any".to_string(),
            TSType::TSUnknownKeyword(_) => "unknown".to_string(),
            TSType::TSNeverKeyword(_) => "never".to_string(),
            TSType::TSVoidKeyword(_) => "void".to_string(),
            TSType::TSUndefinedKeyword(_) => "undefined".to_string(),
            TSType::TSNullKeyword(_) => "null".to_string(),
            TSType::TSArrayType(array) => {
                format!("{}[]", self.extract_type_string_from_ts_type(&array.element_type))
            }
            TSType::TSTypeReference(type_ref) => match &type_ref.type_name {
                oxc_ast::ast::TSTypeName::IdentifierReference(ident) => {
                    ident.name.as_str().to_string()
                }
                _ => "unknown".to_string(),
            },
            TSType::TSUnionType(union) => {
                let types: Vec<String> =
                    union.types.iter().map(|t| self.extract_type_string_from_ts_type(t)).collect();
                types.join(" | ")
            }
            TSType::TSIntersectionType(intersection) => {
                let types: Vec<String> = intersection
                    .types
                    .iter()
                    .map(|t| self.extract_type_string_from_ts_type(t))
                    .collect();
                types.join(" & ")
            }
            TSType::TSFunctionType(func) => {
                let params = self.extract_function_params(&func.params);
                let return_type =
                    self.extract_type_string_from_ts_type(&func.return_type.type_annotation);
                format!("({}) => {}", params, return_type)
            }
            TSType::TSTypeLiteral(literal) => {
                let props: Vec<String> = literal
                    .members
                    .iter()
                    .filter_map(|member| {
                        if let oxc_ast::ast::TSSignature::TSPropertySignature(prop) = member {
                            let name = match &prop.key {
                                oxc_ast::ast::PropertyKey::StaticIdentifier(ident) => {
                                    ident.name.as_str().to_string()
                                }
                                oxc_ast::ast::PropertyKey::StringLiteral(str_lit) => {
                                    str_lit.value.as_str().to_string()
                                }
                                _ => return None,
                            };
                            let type_str = prop
                                .type_annotation
                                .as_ref()
                                .map(|ta| self.extract_type_string(ta))
                                .unwrap_or_else(|| "any".to_string());
                            let optional = if prop.optional { "?" } else { "" };
                            Some(format!("{}{}: {}", name, optional, type_str))
                        } else {
                            None
                        }
                    })
                    .collect();
                format!("{{ {} }}", props.join(", "))
            }
            _ => "any".to_string(),
        }
    }

    fn extract_function_params(&self, params: &oxc_ast::ast::FormalParameters) -> String {
        let param_strings: Vec<String> = params
            .items
            .iter()
            .map(|param| {
                let name = match &param.pattern {
                    oxc_ast::ast::BindingPattern::BindingIdentifier(ident) => ident.name.as_str(),
                    _ => "param",
                };
                let type_str = param
                    .type_annotation
                    .as_ref()
                    .map(|ta| self.extract_type_string(ta))
                    .unwrap_or_else(|| "any".to_string());
                format!("{}: {}", name, type_str)
            })
            .collect();
        param_strings.join(", ")
    }

    fn extract_class(&self, class: &oxc_ast::ast::Class) -> ClassDefinition {
        let name = class
            .id
            .as_ref()
            .map(|id| id.name.as_str().to_string())
            .unwrap_or_else(|| "AnonymousClass".to_string());

        let start_line = self.get_line_number(class.span.start as usize);
        let end_line = self.get_line_number(class.span.end as usize);

        let extends = class.super_class.as_ref().and_then(|super_class| {
            if let oxc_ast::ast::Expression::Identifier(ident) = super_class {
                Some(ident.name.as_str().to_string())
            } else {
                None
            }
        });

        let implements = class
            .implements
            .iter()
            .filter_map(|impl_clause| match &impl_clause.expression {
                oxc_ast::ast::TSTypeName::IdentifierReference(ident) => {
                    Some(ident.name.as_str().to_string())
                }
                _ => None,
            })
            .collect();

        let mut properties = Vec::new();
        let mut methods = Vec::new();
        let mut constructor_params = Vec::new();

        for element in &class.body.body {
            match element {
                ClassElement::PropertyDefinition(prop) => {
                    let Some(name) = self.member_key_name(&prop.key, prop.computed) else {
                        continue;
                    };

                    // `handle = (event) => { … }` class fields are methods
                    // in everything but declaration syntax; extract them as
                    // methods so the arrow-field and method spellings of the
                    // same class compare member-for-member.
                    if let Some(oxc_ast::ast::Expression::ArrowFunctionExpression(arrow)) =
                        &prop.value
                    {
                        let return_type = arrow
                            .return_type
                            .as_ref()
                            .map(|rt| self.extract_type_string_from_ts_type(&rt.type_annotation))
                            .unwrap_or_else(|| "void".to_string());
                        methods.push(ClassMethod {
                            name,
                            parameters: vec![self.extract_function_params(&arrow.params)],
                            return_type,
                            is_static: prop.r#static,
                            is_private: false,
                            is_async: arrow.r#async,
                            is_generator: false,
                            kind: MethodKind::Method,
                            body_fingerprint: self.arrow_body_fingerprint(arrow),
                        });
                        continue;
                    }

                    let type_annotation = prop
                        .type_annotation
                        .as_ref()
                        .map(|ta| self.extract_type_string(ta))
                        .unwrap_or_else(|| "any".to_string());

                    properties.push(ClassProperty {
                        name,
                        type_annotation,
                        is_static: prop.r#static,
                        is_private: false, // PropertyDefinitionType doesn't have TSPrivateProperty
                        is_readonly: prop.readonly,
                        is_optional: prop.optional,
                    });
                }
                ClassElement::MethodDefinition(method) => {
                    let Some(name) = self.member_key_name(&method.key, method.computed) else {
                        continue;
                    };

                    let kind = match method.kind {
                        MethodDefinitionKind::Constructor => {
                            // Extract constructor parameters
                            constructor_params = method
                                .value
                                .params
                                .items
                                .iter()
                                .map(|param| {
                                    let param_name = match &param.pattern {
                                        oxc_ast::ast::BindingPattern::BindingIdentifier(ident) => {
                                            ident.name.as_str()
                                        }
                                        _ => "param",
                                    };
                                    let type_str = param
                                        .type_annotation
                                        .as_ref()
                                        .map(|ta| self.extract_type_string(ta))
                                        .unwrap_or_else(|| "any".to_string());
                                    format!("{}: {}", param_name, type_str)
                                })
                                .collect();
                            MethodKind::Constructor
                        }
                        MethodDefinitionKind::Method => MethodKind::Method,
                        MethodDefinitionKind::Get => MethodKind::Getter,
                        MethodDefinitionKind::Set => MethodKind::Setter,
                    };

                    if kind == MethodKind::Constructor {
                        // Constructor parameter properties (`constructor(
                        // private readonly repo: Repo)`) declare real
                        // fields; surface them so the shorthand and the
                        // explicit field-plus-assignment spelling extract
                        // the same shape.
                        for param in &method.value.params.items {
                            let is_property = param.accessibility.is_some()
                                || param.readonly
                                || param.r#override;
                            if !is_property {
                                continue;
                            }
                            let oxc_ast::ast::BindingPattern::BindingIdentifier(ident) =
                                &param.pattern
                            else {
                                continue;
                            };
                            let type_annotation = param
                                .type_annotation
                                .as_ref()
                                .map(|ta| self.extract_type_string(ta))
                                .unwrap_or_else(|| "any".to_string());
                            properties.push(ClassProperty {
                                name: ident.name.as_str().to_string(),
                                type_annotation,
                                is_static: false,
                                is_private: matches!(
                                    param.accessibility,
                                    Some(oxc_ast::ast::TSAccessibility::Private)
                                ),
                                is_readonly: param.readonly,
                                is_optional: false,
                            });
                        }
                        // A constructor that does real work is a member
                        // like any other: two classes with the same fields
                        // and methods but a constructor that spawns a
                        // timer, validates, or delegates used to compare as
                        // identical because the constructor was skipped.
                        // Wiring-only constructors (`super(…)` plus
                        // `this.x = x`, or the empty body of a
                        // parameter-property constructor) add nothing the
                        // field list doesn't already say, so they stay
                        // out — otherwise a class that happens to have one
                        // would be "missing" it on the other side.
                        if !Self::is_wiring_only_constructor(&method.value) {
                            methods.push(ClassMethod {
                                name,
                                parameters: vec![
                                    self.extract_function_params(&method.value.params),
                                ],
                                return_type: "void".to_string(),
                                is_static: false,
                                is_private: false,
                                is_async: false,
                                is_generator: false,
                                kind: MethodKind::Constructor,
                                body_fingerprint: self.constructor_fingerprint(&method.value),
                            });
                        }
                    } else {
                        let parameters = self.extract_function_params(&method.value.params);
                        let return_type = method
                            .value
                            .return_type
                            .as_ref()
                            .map(|rt| self.extract_type_string_from_ts_type(&rt.type_annotation))
                            .unwrap_or_else(|| "void".to_string());

                        methods.push(ClassMethod {
                            name,
                            parameters: vec![parameters],
                            return_type,
                            is_static: method.r#static,
                            is_private: false, // Would need to check for private keyword
                            is_async: method.value.r#async,
                            is_generator: method.value.generator,
                            kind,
                            body_fingerprint: self.method_body_fingerprint(&method.value),
                        });
                    }
                }
                _ => {}
            }
        }

        ClassDefinition {
            name,
            properties,
            methods,
            constructor_params,
            extends,
            implements,
            start_line,
            end_line,
            file_path: self.file_path.clone(),
            is_abstract: class.r#abstract,
            has_ignore_directive: has_similarity_ignore_directive(&self.source_text, start_line),
        }
    }

    /// Canonical structural hash of a method's params+body (the method
    /// NAME is deliberately excluded so consistently-renamed methods with
    /// equal bodies fingerprint identically).
    fn method_body_fingerprint(&self, function: &oxc_ast::ast::Function) -> Option<u64> {
        use crate::parser::parse_and_convert_to_tree_canonical;
        use std::hash::{Hash, Hasher};

        let body = function.body.as_ref()?;
        let slice = |span: oxc_span::Span| -> Option<&str> {
            let start = span.start as usize;
            let end = span.end as usize;
            if start < end && end <= self.source_text.len() {
                Some(&self.source_text[start..end])
            } else {
                None
            }
        };
        let params_text = slice(function.params.span)?;
        let body_text = slice(body.span)?;
        let mut prefix = String::new();
        if function.r#async {
            prefix.push_str("async ");
        }
        prefix.push_str("function");
        if function.generator {
            prefix.push('*');
        }
        let wrapped = format!("{prefix} __m__{params_text} {body_text}");
        let tree = parse_and_convert_to_tree_canonical("method.ts", &wrapped).ok()?;

        fn hash_tree(node: &crate::tree::TreeNode, hasher: &mut impl Hasher) {
            node.label.hash(hasher);
            node.value.hash(hasher);
            node.children.len().hash(hasher);
            // `this.<field>` / `this.#<field>` accesses hash the field
            // position as a placeholder: renaming private storage fields
            // is the class-level analogue of renaming locals, and two
            // methods that differ only in field names must fingerprint
            // identically. Field names still matter for the property-set
            // comparison, so the signal isn't lost — it's just not
            // double-counted against the method bodies.
            let this_member = matches!(node.value.as_str(),
                "StaticMemberExpression" | "PrivateFieldExpression")
                && node.children.first().is_some_and(|obj| obj.value == "ThisExpression");
            for (index, child) in node.children.iter().enumerate() {
                if this_member
                    && index == 1
                    && matches!(child.value.as_str(), "Identifier" | "PrivateIdentifier")
                {
                    "§field".hash(hasher);
                    child.value.hash(hasher);
                    0usize.hash(hasher);
                } else {
                    hash_tree(child, hasher);
                }
            }
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hash_tree(&tree, &mut hasher);
        Some(hasher.finish())
    }

    /// Fingerprint for a constructor. Parsed inside a class wrapper so the
    /// canonicalizer's parameter-property desugaring applies:
    /// `constructor(private readonly repo: Repo) {}` and
    /// `constructor(repo: Repo) { this.repo = repo; }` hash identically.
    fn constructor_fingerprint(&self, function: &oxc_ast::ast::Function) -> Option<u64> {
        let body = function.body.as_ref()?;
        let params_text = self.source_slice(function.params.span)?;
        let body_text = self.source_slice(body.span)?;
        let wrapped = format!("class __C__ {{ constructor{params_text} {body_text} }}");
        Self::fingerprint_wrapped_function(&wrapped)
    }

    fn source_slice(&self, span: oxc_span::Span) -> Option<&str> {
        let start = span.start as usize;
        let end = span.end as usize;
        if start < end && end <= self.source_text.len() {
            Some(&self.source_text[start..end])
        } else {
            None
        }
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
                let text = self.source_slice(other.span())?;
                if computed {
                    format!("[{text}]")
                } else {
                    text.to_string()
                }
            }
        })
    }

    /// Whether a constructor body only forwards to `super(…)` and stores
    /// identifiers into `this` fields (or is empty). See the call site.
    fn is_wiring_only_constructor(function: &oxc_ast::ast::Function) -> bool {
        use oxc_ast::ast::{AssignmentOperator, AssignmentTarget, Expression, Statement};
        let Some(body) = &function.body else {
            return true;
        };
        body.statements.iter().all(|stmt| {
            let Statement::ExpressionStatement(expr_stmt) = stmt else {
                return false;
            };
            match &expr_stmt.expression {
                Expression::CallExpression(call) => matches!(call.callee, Expression::Super(_)),
                Expression::AssignmentExpression(assign) => {
                    assign.operator == AssignmentOperator::Assign
                        && matches!(
                            &assign.left,
                            AssignmentTarget::StaticMemberExpression(member)
                                if matches!(member.object, Expression::ThisExpression(_))
                        )
                        && matches!(assign.right, Expression::Identifier(_))
                }
                _ => false,
            }
        })
    }

    /// Fingerprint for an arrow-function class field, shaped exactly like
    /// [`Self::method_body_fingerprint`] so `handle = () => {…}` and
    /// `handle() {…}` with equal bodies hash identically.
    fn arrow_body_fingerprint(
        &self,
        arrow: &oxc_ast::ast::ArrowFunctionExpression,
    ) -> Option<u64> {
        let slice = |span: oxc_span::Span| -> Option<&str> {
            let start = span.start as usize;
            let end = span.end as usize;
            if start < end && end <= self.source_text.len() {
                Some(&self.source_text[start..end])
            } else {
                None
            }
        };
        let params_text = slice(arrow.params.span)?;
        let params_text = if params_text.starts_with('(') {
            params_text.to_string()
        } else {
            format!("({params_text})")
        };
        let body_text = slice(arrow.body.span)?;
        let body_text = if arrow.expression {
            format!("{{ return {body_text}; }}")
        } else {
            body_text.to_string()
        };
        let prefix = if arrow.r#async { "async function" } else { "function" };
        let wrapped = format!("{prefix} __m__{params_text} {body_text}");
        Self::fingerprint_wrapped_function(&wrapped)
    }

    /// Parse a synthesized standalone function and hash its canonical
    /// tree with `this.<field>` names positionally normalized.
    fn fingerprint_wrapped_function(wrapped: &str) -> Option<u64> {
        use crate::parser::parse_and_convert_to_tree_canonical;
        use std::hash::{Hash, Hasher};

        let tree = parse_and_convert_to_tree_canonical("method.ts", wrapped).ok()?;

        fn hash_tree(node: &crate::tree::TreeNode, hasher: &mut impl Hasher) {
            node.label.hash(hasher);
            node.value.hash(hasher);
            node.children.len().hash(hasher);
            let this_member = matches!(node.value.as_str(),
                "StaticMemberExpression" | "PrivateFieldExpression")
                && node.children.first().is_some_and(|obj| obj.value == "ThisExpression");
            for (index, child) in node.children.iter().enumerate() {
                if this_member
                    && index == 1
                    && matches!(child.value.as_str(), "Identifier" | "PrivateIdentifier")
                {
                    "§field".hash(hasher);
                    child.value.hash(hasher);
                    0usize.hash(hasher);
                } else {
                    hash_tree(child, hasher);
                }
            }
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hash_tree(&tree, &mut hasher);
        Some(hasher.finish())
    }

    pub fn extract_classes(&self) -> Result<Vec<ClassDefinition>, String> {
        let allocator = Allocator::default();
        let source_type = SourceType::from_path(&self.file_path).unwrap_or(SourceType::tsx());
        let ret = Parser::new(&allocator, &self.source_text, source_type).parse();

        if !ret.errors.is_empty() {
            let error_messages: Vec<String> =
                ret.errors.iter().map(|e| format!("{:?}", e)).collect();
            return Err(format!("Parse errors: {}", error_messages.join(", ")));
        }

        let mut classes = Vec::new();

        // Walk through all statements and find classes
        for statement in &ret.program.body {
            self.extract_classes_from_statement(statement, &mut classes);
        }

        Ok(classes)
    }

    fn extract_classes_from_statement(
        &self,
        statement: &Statement,
        classes: &mut Vec<ClassDefinition>,
    ) {
        match statement {
            Statement::ExportDefaultDeclaration(export) => {
                if let oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class) =
                    &export.declaration
                {
                    classes.push(self.extract_class(class));
                }
            }
            Statement::ExportNamedDeclaration(export) => match &export.declaration {
                Some(oxc_ast::ast::Declaration::ClassDeclaration(class)) => {
                    classes.push(self.extract_class(class));
                }
                Some(oxc_ast::ast::Declaration::TSModuleDeclaration(module)) => {
                    self.extract_classes_from_module(module, classes);
                }
                _ => {}
            },
            Statement::ClassDeclaration(class) => {
                classes.push(self.extract_class(class));
            }
            // `namespace`/`module` blocks are ordinary declaration scopes
            // for the classes inside them; they used to be skipped.
            Statement::TSModuleDeclaration(module) => {
                self.extract_classes_from_module(module, classes);
            }
            _ => {}
        }
    }

    fn extract_classes_from_module(
        &self,
        module: &oxc_ast::ast::TSModuleDeclaration,
        classes: &mut Vec<ClassDefinition>,
    ) {
        let Some(body) = &module.body else {
            return;
        };
        match body {
            oxc_ast::ast::TSModuleDeclarationBody::TSModuleDeclaration(inner) => {
                self.extract_classes_from_module(inner, classes);
            }
            oxc_ast::ast::TSModuleDeclarationBody::TSModuleBlock(block) => {
                for stmt in &block.body {
                    self.extract_classes_from_statement(stmt, classes);
                }
            }
        }
    }
}

pub fn extract_classes_from_code(
    code: &str,
    file_path: &str,
) -> Result<Vec<ClassDefinition>, String> {
    let extractor = ClassExtractor::new(code.to_string(), file_path.to_string());
    extractor.extract_classes()
}

pub fn extract_classes_from_files(files: &[(String, String)]) -> Vec<ClassDefinition> {
    let mut all_classes = Vec::new();

    for (file_path, content) in files {
        match extract_classes_from_code(content, file_path) {
            Ok(classes) => all_classes.extend(classes),
            Err(e) => eprintln!("Error extracting classes from {}: {}", file_path, e),
        }
    }

    all_classes
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
        assert_eq!(names, vec!["Pager", "Hidden"]);
    }
}
