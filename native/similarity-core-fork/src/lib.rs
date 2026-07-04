#![allow(clippy::uninlined_format_args)]

//! TypeScript-only similarity core (fork of `similarity-core`).
//!
//! The module set is intentionally small — exactly the pipeline the WASM
//! entry point (`similarity-core-ts`) drives:
//!
//! * `parser`/`tree` — oxc AST → canonicalized `TreeNode` conversion,
//!   including alpha-renaming and refactor-equivalence lowering
//! * `apted`/`tsed` — tree edit distance and the TSED similarity score
//! * `function_extractor` — function discovery + the unified pairwise scan
//! * `type_extractor`/`type_normalizer`/`type_comparator` — type mode
//! * `class_extractor`/`class_comparator` — class mode
//! * `subtree_fingerprint`/`overlap_detector` — overlap mode

pub mod apted;
pub mod class_comparator;
pub mod class_extractor;
pub mod function_extractor;
mod ignore_directive;
pub mod overlap_detector;
pub mod parser;
pub mod subtree_fingerprint;
pub mod tree;
pub mod tsed;
pub mod type_comparator;
pub mod type_extractor;
pub mod type_normalizer;

pub use apted::{compute_edit_distance, APTEDOptions};
pub use function_extractor::{
    compare_functions, extract_functions, find_similar_function_pairs,
    find_similar_functions_across_files, find_similar_functions_in_file, FunctionDefinition,
    FunctionType, PairScope, SimilarityResult, SkippedFile,
};
pub use parser::{
    ast_to_tree_node, parse_and_convert_to_tree, parse_and_convert_to_tree_canonical,
};
pub use tree::TreeNode;
pub use tsed::{calculate_tsed, calculate_tsed_from_code, TSEDOptions};

// Type-related exports
pub use type_comparator::{
    compare_types, find_similar_types, MatchedProperty, SimilarTypePair, TypeComparisonOptions,
    TypeComparisonResult, TypeDifferences, TypeMismatch,
};
pub use type_extractor::{
    extract_type_literals_from_code, extract_type_literals_from_files, extract_types_from_code,
    extract_types_from_files, PropertyDefinition, TypeDefinition, TypeKind, TypeLiteralContext,
    TypeLiteralDefinition,
};
pub use type_normalizer::{
    calculate_property_similarity, calculate_type_similarity, find_property_matches,
    normalize_type, NormalizationOptions, NormalizedType, PropertyMatch,
};

// Subtree fingerprint / overlap exports
pub use overlap_detector::{
    find_function_overlaps, find_overlaps_across_files, find_overlaps_with_similarity,
    DetailedOverlap, PartialOverlapWithFiles,
};
pub use subtree_fingerprint::{
    create_sliding_windows, detect_partial_overlaps, generate_subtree_fingerprints,
    IndexedFunction, OverlapOptions, PartialOverlap, SubtreeFingerprint,
};

// Class-related exports
pub use class_comparator::{
    compare_classes, find_similar_classes, find_similar_classes_across_files, normalize_class,
    ClassComparisonResult, ClassDifferences, MethodMismatch, NormalizedClass, PropertyMismatch,
    SimilarClassPair,
};
pub use class_extractor::{
    extract_classes_from_code, extract_classes_from_files, ClassDefinition, ClassMethod,
    ClassProperty, MethodKind,
};
