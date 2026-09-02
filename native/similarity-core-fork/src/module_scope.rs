//! Namespace-scoped declaration names.
//!
//! Declarations inside `namespace`/`module` blocks are reported under their
//! qualified name (`Legacy.Options`, `a.b.c.Pager`) so they can be told
//! apart from a top-level declaration with the same bare name. Comparators
//! score on the bare name (see [`unqualified_name`]): a qualifier shared by
//! two unrelated declarations must not read as lexical agreement, and a
//! qualifier on one side only must not hide it.

use oxc_ast::ast::TSModuleDeclarationName;

/// The qualifier for declarations directly inside `module`, given the
/// qualifier of the enclosing scope (`""` at the top level). String-named
/// modules (`declare module "fs" { … }`) and `declare global` scope their
/// members like a file rather than a namespace, so they add nothing.
pub(crate) fn module_qualifier(id: &TSModuleDeclarationName, outer: &str) -> String {
    match id {
        TSModuleDeclarationName::Identifier(ident) if ident.name.as_str() != "global" => {
            qualified_name(outer, ident.name.as_str())
        }
        _ => outer.to_string(),
    }
}

/// `qualifier.name`, or just `name` at the top level.
pub(crate) fn qualified_name(qualifier: &str, name: &str) -> String {
    if qualifier.is_empty() {
        name.to_string()
    } else {
        format!("{qualifier}.{name}")
    }
}

/// The bare declaration name behind a (possibly) qualified one.
pub(crate) fn unqualified_name(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_qualify_and_unqualify() {
        assert_eq!(qualified_name("", "Options"), "Options");
        assert_eq!(qualified_name("Legacy.Inner", "Options"), "Legacy.Inner.Options");
        assert_eq!(unqualified_name("Legacy.Inner.Options"), "Options");
        assert_eq!(unqualified_name("Options"), "Options");
    }
}
