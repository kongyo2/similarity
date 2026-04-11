use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFile {
    pub file_path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzeInput {
    pub files: Vec<ProjectFile>,
    pub modes: Vec<String>,
    pub threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzeOutput {
    pub warnings: Vec<String>,
}

pub fn analyze_project(_input: AnalyzeInput) -> AnalyzeOutput {
    AnalyzeOutput {
        warnings: vec![
            "similarity-core-ts scaffold: TS-specific Rust ports will be added incrementally".to_string(),
        ],
    }
}
