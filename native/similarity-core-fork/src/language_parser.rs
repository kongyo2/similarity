use crate::tree::TreeNode;
use std::error::Error;
use std::rc::Rc;

/// Supported programming languages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    JavaScript,
    TypeScript,
    Python,
    Php,
    Rust,
    Go,
    Java,
    C,
    Cpp,
    CSharp,
    Ruby,
    Unknown,
}

impl Language {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "js" | "mjs" | "cjs" => Some(Language::JavaScript),
            "ts" | "tsx" => Some(Language::TypeScript),
            "py" => Some(Language::Python),
            "php" => Some(Language::Php),
            "rs" => Some(Language::Rust),
            "go" => Some(Language::Go),
            "java" => Some(Language::Java),
            "c" | "h" => Some(Language::C),
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "c++" => Some(Language::Cpp),
            "cs" => Some(Language::CSharp),
            "rb" => Some(Language::Ruby),
            _ => None,
        }
    }

    pub fn from_filename(filename: &str) -> Option<Self> {
        filename.split('.').next_back().and_then(Self::from_extension)
    }
}

/// Generic function definition that works across languages
#[derive(Debug, Clone)]
pub struct GenericFunctionDef {
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub body_start_line: u32,
    pub body_end_line: u32,
    pub parameters: Vec<String>,
    pub is_method: bool,
    pub class_name: Option<String>,
    pub is_async: bool,
    pub is_generator: bool,
    pub decorators: Vec<String>,
}

impl GenericFunctionDef {
    /// Check if this function and `other` are nested in each other (one
    /// strictly contains the other). Used to drop parent-child pairs in
    /// same-file overlap scans, where the parent's subtree fingerprints
    /// trivially include the child and would always "match" — that's a
    /// containment artifact, not a duplication signal.
    ///
    /// Containment is decided on body line ranges so that two functions
    /// declared on identical line spans (impossible in practice but cheap
    /// to guard against) are not incorrectly treated as parent/child.
    pub fn is_parent_child_relationship(&self, other: &GenericFunctionDef) -> bool {
        let other_inside_self = self.start_line <= other.start_line
            && self.end_line >= other.end_line
            && self.body_start_line < other.body_start_line
            && self.body_end_line > other.body_end_line;

        let self_inside_other = other.start_line <= self.start_line
            && other.end_line >= self.end_line
            && other.body_start_line < self.body_start_line
            && other.body_end_line > self.body_end_line;

        other_inside_self || self_inside_other
    }
}

/// Generic type definition that works across languages
#[derive(Debug, Clone)]
pub struct GenericTypeDef {
    pub name: String,
    pub kind: String, // "struct", "enum", "type_alias", etc.
    pub start_line: u32,
    pub end_line: u32,
    pub fields: Vec<String>, // Fields for structs, variants for enums, etc.
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeDefKind {
    Class,
    Interface,
    TypeAlias,
    Enum,
    Struct,
}

/// Trait for language-specific parsers
pub trait LanguageParser: Send + Sync {
    /// Parse source code into a TreeNode structure
    fn parse(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<Rc<TreeNode>, Box<dyn Error + Send + Sync>>;

    /// Extract function definitions from source code
    fn extract_functions(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<Vec<GenericFunctionDef>, Box<dyn Error + Send + Sync>>;

    /// Extract type definitions from source code
    fn extract_types(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<Vec<GenericTypeDef>, Box<dyn Error + Send + Sync>>;

    /// Get the language this parser handles
    fn language(&self) -> Language;
}

// ParserFactory is removed - each language CLI now manages its own parser

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_detection() {
        assert_eq!(Language::from_filename("test.js"), Some(Language::JavaScript));
        assert_eq!(Language::from_filename("test.ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_filename("test.py"), Some(Language::Python));
        assert_eq!(Language::from_filename("test.php"), Some(Language::Php));
        assert_eq!(Language::from_filename("test.rs"), Some(Language::Rust));
        assert_eq!(Language::from_filename("test.go"), Some(Language::Go));
        assert_eq!(Language::from_filename("test.txt"), None);
    }

    #[test]
    fn test_case_insensitive_extension() {
        assert_eq!(Language::from_extension("JS"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension("Py"), Some(Language::Python));
    }
}
