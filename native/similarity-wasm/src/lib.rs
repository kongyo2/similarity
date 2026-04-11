use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn analyze_project(input_json: &str) -> String {
    let parsed = serde_json::from_str(input_json)
        .unwrap_or_else(|e| serde_json::json!({"warnings": [format!("invalid input: {e}")]}));

    let input = serde_json::from_value(parsed)
        .map_err(|e| serde_json::json!({"warnings": [format!("invalid schema: {e}")]}));

    let output = match input {
        Ok(input) => similarity_core_ts::analyze_project(input),
        Err(err) => return err.to_string(),
    };

    serde_json::to_string(&output)
        .unwrap_or_else(|e| serde_json::json!({"warnings": [format!("serialization error: {e}")]}).to_string())
}
