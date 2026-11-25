use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceMetrics {
    pub output_tokens: u64,
    pub prompt_tokens: u64,
    pub generation_ms: f64,
    pub tps: f64,
    pub elapsed_ms: u64,
    pub first_token_ms: Option<u64>,
}

impl InferenceMetrics {
    pub fn from_finished_request(value: &Value, id: &str, elapsed_ms: u64) -> Option<Self> {
        if id.is_empty() || value["id"] != id || value["endpoint"] != "batch.completions" {
            return None;
        }
        let output_tokens = value["generated_tokens"].as_u64()?;
        let prompt_tokens = value["prompt_tokens"].as_u64()?;
        let tps = value["decode_speed"]
            .as_f64()
            .filter(|speed| *speed > 0.0)
            .unwrap_or_default();
        Some(Self {
            output_tokens,
            prompt_tokens,
            tps,
            elapsed_ms,
            generation_ms: if tps > 0.0 {
                output_tokens as f64 * 1000.0 / tps
            } else {
                0.0
            },
            first_token_ms: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn uses_server_totals_only_for_the_matching_completed_batch() {
        let finished = json!({"id":"request-1","endpoint":"batch.completions",
            "generated_tokens":120,"prompt_tokens":340,"decode_speed":240.0});
        let stats = InferenceMetrics::from_finished_request(&finished, "request-1", 800).unwrap();
        assert_eq!(stats.output_tokens, 120);
        assert_eq!(stats.prompt_tokens, 340);
        assert_eq!(stats.tps, 240.0);
        assert_eq!(stats.generation_ms, 500.0);
        assert!(InferenceMetrics::from_finished_request(&finished, "request-2", 800).is_none());
    }
}
