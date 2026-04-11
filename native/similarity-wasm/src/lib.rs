use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn analyze_project(input_json: &str) -> String {
    let input: similarity_core_ts::AnalyzeInput = match serde_json::from_str(input_json) {
        Ok(input) => input,
        Err(e) => return serde_json::json!({"warnings": [format!("invalid input: {e}")]}).to_string(),
    };

    let output = similarity_core_ts::analyze_project(input);

    serde_json::to_string(&output)
        .unwrap_or_else(|e| serde_json::json!({"warnings": [format!("serialization error: {e}")]}).to_string())
}
