//! TypeScript-only similarity core (fork of `similarity-core`).
//!
//! The module set is intentionally small — exactly the pipeline the WASM
//! entry point (`similarity-core-ts`) drives:
//!
//! * `source` — parsing, line lookup, and the nesting guard
//! * `declarations` — every declaration of a file from one parse
//! * `parser`/`tree` — oxc AST → canonicalized `TreeNode` conversion,
//!   including alpha-renaming and refactor-equivalence lowering
//! * `apted`/`tsed` — tree edit distance and the TSED similarity score
//! * `function_extractor`/`function_comparator` — function discovery, the
//!   score of a pair, and the pairwise scan
//! * `type_extractor`/`type_normalizer`/`type_comparator` — type mode
//! * `class_extractor`/`class_comparator` — class mode
//! * `subtree_fingerprint`/`overlap_detector` — overlap mode

pub mod apted;
pub mod class_comparator;
pub mod class_extractor;
pub mod declarations;
mod function_atoms;
pub mod function_comparator;
pub mod function_extractor;
mod function_fragment;
mod ignore_directive;
mod module_scope;
pub mod overlap_detector;
pub mod parser;
pub mod source;
pub mod subtree_fingerprint;
mod text;
pub mod tree;
pub mod tsed;
pub mod type_comparator;
pub mod type_extractor;
pub mod type_normalizer;

pub use apted::{compute_edit_distance, APTEDOptions};
pub use function_comparator::{
    compare_functions, find_similar_function_pairs, find_similar_functions_across_files,
    find_similar_functions_in_file, find_similar_functions_in_files, FunctionSource, PairScope,
    SimilarFunctionPair, SimilarityResult, SkippedFile,
};
pub use function_extractor::{
    extract_functions, extract_functions_from_program, FunctionDefinition, FunctionType, SourceForm,
};
pub use parser::{parse_and_convert_to_tree, parse_and_convert_to_tree_canonical};
pub use tree::TreeNode;
pub use tsed::{calculate_tsed, calculate_tsed_from_code, TSEDOptions};

// Type-related exports
pub use type_comparator::{
    compare_types, find_similar_types, MatchedProperty, SimilarTypePair, TypeComparisonOptions,
    TypeComparisonResult, TypeDifferences, TypeMismatch,
};
pub use type_extractor::{
    extract_type_literals_from_code, extract_types_from_code, PropertyDefinition, TypeDefinition,
    TypeExtractor, TypeKind, TypeLiteralContext, TypeLiteralDefinition,
};
pub use type_normalizer::{
    calculate_property_similarity, calculate_type_similarity, find_property_matches,
    normalize_type, NormalizationOptions, NormalizedType, PropertyMatch,
};

// Subtree fingerprint / overlap exports
pub use overlap_detector::{
    find_function_overlaps, find_overlaps, find_overlaps_across_files, PartialOverlapWithFiles,
};
pub use subtree_fingerprint::{
    detect_partial_overlaps, IndexedFunction, OverlapOptions, PartialOverlap, SubtreeFingerprint,
    Window,
};

// Class-related exports
pub use class_comparator::{
    compare_classes, find_similar_classes, normalize_class, ClassComparisonResult,
    ClassDifferences, MethodMismatch, NormalizedClass, PropertyMismatch, SimilarClassPair,
};
pub use class_extractor::{
    extract_classes_from_code, ClassDefinition, ClassExtractor, ClassMethod, ClassProperty,
    MethodKind,
};

// Single-parse extraction and source checks
pub use declarations::{extract_declarations, DeclarationKinds, FileDeclarations};
pub use source::{check_source, LineIndex};
