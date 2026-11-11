use serde::Serialize;
use serde_json::Value;

/// Counters and decode throughput reported by llama-server, never character counts.
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
    pub fn update(&mut self, event: &Value, elapsed_ms: u64) -> bool {
        let timings = event.get("timings");
        let usage = event.get("usage");
        if timings.is_none() && usage.is_none() && event.get("prompt_progress").is_none() {
            return false;
        }
        self.elapsed_ms = elapsed_ms;
        if let Some(n) = usage
            .and_then(|v| v["completion_tokens"].as_u64())
            .or_else(|| timings.and_then(|v| v["predicted_n"].as_u64()))
        {
            self.output_tokens = self.output_tokens.max(n);
        }
        if let Some(n) = usage
            .and_then(|v| v["prompt_tokens"].as_u64())
            .or_else(|| event["prompt_progress"]["total"].as_u64())
            .or_else(|| timings.and_then(|v| v["prompt_n"].as_u64()))
        {
            self.prompt_tokens = self.prompt_tokens.max(n);
        }
        if let Some(ms) = timings
            .and_then(|v| v["predicted_ms"].as_f64())
            .filter(|n| n.is_finite() && *n >= 0.0)
        {
            self.generation_ms = ms;
        }
        if let Some(tps) = timings
            .and_then(|v| v["predicted_per_second"].as_f64())
            .filter(|n| n.is_finite() && *n >= 0.0)
        {
            self.tps = tps;
        }
        if self.output_tokens > 0 && self.first_token_ms.is_none() {
            self.first_token_ms = Some(elapsed_ms);
        }
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn preserves_engine_token_counts_and_rate_with_multibyte_text() {
        let mut m = InferenceMetrics::default();
        assert!(m.update(&json!({"choices":[{"delta":{"content":"中"}}],"timings":{"predicted_n":15,"predicted_ms":234.309,"predicted_per_second":59.75015897810157,"prompt_n":31}}), 600));
        assert_eq!(m.output_tokens, 15); // Not "中".len(), nor the number of chunks.
        assert_eq!(m.first_token_ms, Some(600));
        assert_eq!(m.tps, 59.75015897810157);
        m.update(&json!({"choices":[],"usage":{"completion_tokens":32,"prompt_tokens":31},"timings":{"predicted_n":32,"predicted_ms":510.705,"predicted_per_second":60.70040434301603}}), 900);
        assert_eq!(m.output_tokens, 32);
        assert_eq!(m.first_token_ms, Some(600));
        assert_eq!(m.tps, 60.70040434301603);
        assert!(!m.update(
            &json!({"choices":[{"delta":{"content":"一大段文本"}}]}),
            950
        ));
        assert_eq!(m.output_tokens, 32);
    }
    #[test]
    fn reports_prefill_and_does_not_move_counts_backwards() {
        let mut m = InferenceMetrics::default();
        m.update(&json!({"prompt_progress":{"total":200,"processed":10},"timings":{"prompt_n":10,"predicted_n":0}}),100);
        assert_eq!(m.prompt_tokens, 200);
        assert_eq!(m.first_token_ms, None);
        m.update(
            &json!({"timings":{"predicted_n":8,"predicted_per_second":45.5}}),
            300,
        );
        m.update(
            &json!({"timings":{"predicted_n":7,"predicted_per_second":null}}),
            320,
        );
        assert_eq!(m.output_tokens, 8);
        assert_eq!(m.tps, 45.5);
    }
}
