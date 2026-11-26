use serde::Serialize;
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RateKind {
    Stream,
    Decode,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceMetrics {
    pub output_tokens: u64,
    pub prompt_tokens: u64,
    pub generation_ms: f64,
    pub tps: f64,
    pub elapsed_ms: u64,
    pub first_token_ms: Option<u64>,
    pub rate_kind: Option<RateKind>,
}

impl InferenceMetrics {
    pub fn from_finished_request(value: &Value, id: &str, elapsed_ms: u64) -> Option<Self> {
        let (output_tokens, prompt_tokens) = batch_counts(value, id)?;
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
            rate_kind: Some(RateKind::Decode),
        })
    }
}

fn batch_counts(value: &Value, id: &str) -> Option<(u64, u64)> {
    if id.is_empty() || value["id"] != id || value["endpoint"] != "batch.completions" {
        return None;
    }
    Some((
        value["generated_tokens"].as_u64()?,
        value["prompt_tokens"].as_u64()?,
    ))
}

#[derive(Default)]
pub struct StreamRate(Option<(Instant, u64)>);

impl StreamRate {
    pub fn sample(
        &mut self,
        status: &Value,
        id: &str,
        sampled_at: Instant,
        elapsed: Duration,
    ) -> Option<InferenceMetrics> {
        let active = status["active_requests"]
            .as_array()?
            .iter()
            .find(|request| request["id"] == id && request["endpoint"] == "batch.completions")?;
        // The active counter counts re-tokenized SSE chunks. The final server
        // totals can replace it briefly before the request leaves this list.
        if active["decode_speed"]
            .as_f64()
            .is_some_and(|speed| speed > 0.0)
        {
            return None;
        }
        let (output_tokens, prompt_tokens) = batch_counts(active, id)?;
        let tps = self
            .0
            .replace((sampled_at, output_tokens))
            .and_then(|(previous_at, previous_tokens)| {
                let tokens = output_tokens.checked_sub(previous_tokens)?;
                let seconds = sampled_at
                    .checked_duration_since(previous_at)?
                    .as_secs_f64();
                (seconds > 0.0).then(|| tokens as f64 / seconds)
            })
            .unwrap_or_default();
        Some(InferenceMetrics {
            output_tokens,
            prompt_tokens,
            tps,
            elapsed_ms: elapsed.as_millis() as u64,
            rate_kind: Some(RateKind::Stream),
            ..Default::default()
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

    #[test]
    fn measures_live_stream_rate_only_between_matching_counter_samples() {
        let started = Instant::now();
        let mut rate = StreamRate::default();
        let snapshot = |tokens| {
            json!({ "active_requests": [
            {"id":"other","endpoint":"batch.completions","generated_tokens":900,"prompt_tokens":21},
            {"id":"request-1","endpoint":"batch.completions","generated_tokens":tokens,"prompt_tokens":340,"decode_speed":0.0}
        ] })
        };
        let stats = rate
            .sample(&snapshot(32), "request-1", started, Duration::ZERO)
            .unwrap();
        assert_eq!(stats.output_tokens, 32);
        assert_eq!(stats.tps, 0.0);
        assert!(rate
            .sample(&snapshot(60), "unrelated", started, Duration::ZERO)
            .is_none());
        let elapsed = Duration::from_millis(500);
        let stats = rate
            .sample(&snapshot(96), "request-1", started + elapsed, elapsed)
            .unwrap();
        assert_eq!(stats.output_tokens, 96);
        assert_eq!(stats.tps, 128.0);
        assert_eq!(stats.elapsed_ms, 500);
        assert_eq!(serde_json::to_value(stats).unwrap()["rateKind"], "stream");
    }
}
