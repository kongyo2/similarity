//! WebAssembly entry point of the analysis engine.

use wasm_bindgen::prelude::*;

/// Analyze a project: takes an `AnalyzeInput` as JSON and returns an
/// `AnalyzeOutput` as JSON.
///
/// # Errors
///
/// Throws when the input is not a valid `AnalyzeInput` or its options
/// contradict each other.
#[wasm_bindgen]
pub fn analyze_project(input_json: &str) -> Result<String, JsError> {
    similarity_core_ts::analyze_project_json(input_json).map_err(|message| JsError::new(&message))
}

/// Why a source file cannot be analyzed — syntax errors, or code nested
/// too deeply — or `undefined` when it can.
///
/// The JavaScript side calls this on every file after the engine ran out
/// of stack: a file whose parse brings the engine down cannot report
/// itself, but it can be found by trying the files one at a time.
#[wasm_bindgen]
#[must_use]
pub fn check_source(file_path: &str, content: &str) -> Option<String> {
    similarity_core_ts::check_source(file_path, content).err()
}
