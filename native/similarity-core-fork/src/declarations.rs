//! One pass over a source file: every kind of declaration the analysis
//! modes compare, from a single parse.

use crate::class_extractor::{ClassDefinition, ClassExtractor};
use crate::function_extractor::{extract_functions_from_program, FunctionDefinition};
use crate::source::{with_checked_program, LineIndex};
use crate::type_extractor::{TypeDefinition, TypeExtractor, TypeLiteralDefinition};

/// Which declarations [`extract_declarations`] collects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeclarationKinds {
    pub functions: bool,
    pub types: bool,
    pub type_literals: bool,
    pub classes: bool,
}

/// The declarations of one source file.
#[derive(Debug, Clone, Default)]
pub struct FileDeclarations {
    pub functions: Vec<FunctionDefinition>,
    pub types: Vec<TypeDefinition>,
    pub type_literals: Vec<TypeLiteralDefinition>,
    pub classes: Vec<ClassDefinition>,
}

/// Parse a source file once and extract the requested declarations.
///
/// # Errors
///
/// Returns the reason the file cannot be analyzed: syntax errors, or
/// nesting beyond the limits of [`crate::source`]. The check does not
/// depend on `kinds`, so a file is either analyzable by every mode or by
/// none.
pub fn extract_declarations(
    path: &str,
    source_text: &str,
    kinds: DeclarationKinds,
) -> Result<FileDeclarations, String> {
    let lines = LineIndex::new(source_text);
    with_checked_program(path, source_text, |program| {
        let mut declarations = FileDeclarations::default();
        if kinds.functions {
            declarations.functions = extract_functions_from_program(program, source_text, &lines);
        }
        if kinds.types || kinds.type_literals {
            let extractor = TypeExtractor::new(source_text, path, &lines);
            if kinds.types {
                declarations.types = extractor.extract_types(program);
            }
            if kinds.type_literals {
                declarations.type_literals = extractor.extract_type_literals(program);
            }
        }
        if kinds.classes {
            declarations.classes =
                ClassExtractor::new(source_text, path, &lines).extract_classes(program);
        }
        declarations
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r"
export interface Point { x: number; y: number }
export function norm(point: { x: number; y: number }): number {
    return Math.hypot(point.x, point.y);
}
export class Path {
    private points: Point[] = [];
    add(point: Point): void { this.points.push(point); }
}
";

    #[test]
    fn extracts_only_the_requested_kinds() {
        let all =
            DeclarationKinds { functions: true, types: true, type_literals: true, classes: true };
        let declarations = extract_declarations("shapes.ts", SOURCE, all).unwrap();
        let functions: Vec<&str> = declarations.functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(functions, vec!["norm", "Path.add"]);
        assert_eq!(declarations.types.len(), 1);
        assert_eq!(declarations.type_literals.len(), 1);
        assert_eq!(declarations.type_literals[0].name, "norm (parameter: point)");
        assert_eq!(declarations.classes.len(), 1);

        let only_classes = DeclarationKinds { classes: true, ..DeclarationKinds::default() };
        let declarations = extract_declarations("shapes.ts", SOURCE, only_classes).unwrap();
        assert!(declarations.functions.is_empty());
        assert!(declarations.types.is_empty());
        assert!(declarations.type_literals.is_empty());
        assert_eq!(declarations.classes[0].name, "Path");
    }

    #[test]
    fn a_file_that_does_not_parse_is_an_error_whatever_is_requested() {
        for kinds in [
            DeclarationKinds::default(),
            DeclarationKinds { types: true, ..DeclarationKinds::default() },
            DeclarationKinds { classes: true, ..DeclarationKinds::default() },
        ] {
            let error =
                extract_declarations("broken.ts", "export class {{{ nope", kinds).unwrap_err();
            assert!(error.starts_with("Parse errors: "), "{error}");
        }
    }
}
