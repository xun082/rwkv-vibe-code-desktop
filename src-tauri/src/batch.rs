//! Demultiplex one native llama-server completion stream by prompt index.
use crate::{extract_html, metrics::InferenceMetrics, prompt::HTML_PREFILL};
use serde_json::Value;

struct Lane {
    text: String,
    stats: InferenceMetrics,
    finished: bool,
}
pub struct BatchStream {
    lanes: Vec<Lane>,
}
pub struct Update {
    pub index: usize,
    pub delta: Option<String>,
    pub stats: Option<InferenceMetrics>,
    pub terminal: Option<Result<(String, bool), String>>,
}
impl BatchStream {
    pub fn new(count: usize) -> Self {
        Self {
            lanes: (0..count)
                .map(|_| Lane {
                    text: HTML_PREFILL.into(),
                    stats: InferenceMetrics::default(),
                    finished: false,
                })
                .collect(),
        }
    }
    pub fn all_finished(&self) -> bool {
        self.lanes.iter().all(|lane| lane.finished)
    }
    pub fn push(&mut self, value: &Value, elapsed_ms: u64) -> Result<Option<Update>, String> {
        if let Some(error) = value.get("error") {
            return Err(format!("模型返回错误：{error}"));
        }
        let index = value["index"].as_u64().ok_or("批量流缺少 prompt 索引")? as usize;
        let lane = self
            .lanes
            .get_mut(index)
            .ok_or("批量流的 prompt 索引越界")?;
        if lane.finished {
            return Ok(None);
        }
        let stats = lane
            .stats
            .update(value, elapsed_ms)
            .then(|| lane.stats.clone());
        let delta = value["content"]
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        if let Some(text) = &delta {
            lane.text.push_str(text);
        }
        if lane.text.len() > 2 * 1024 * 1024 {
            return Err("生成内容超过限制".into());
        }
        let terminal = if value["stop"].as_bool() == Some(true) {
            lane.finished = true;
            // Native stop strings are excluded from content. Restore only the
            // exact closing delimiter the engine reports having generated.
            if value["stopping_word"] == "</html>" {
                lane.text.push_str("</html>");
            }
            Some(if lane.text.len() == HTML_PREFILL.len() {
                Err("模型没有返回网页内容，请重试".into())
            } else {
                extract_html(&lane.text).map(|html| {
                    let incomplete = value["stop_type"] == "limit"
                        || value["truncated"] == true
                        || !html.to_ascii_lowercase().contains("</html>");
                    (html, incomplete)
                })
            })
        } else {
            None
        };
        Ok(Some(Update {
            index,
            delta,
            stats,
            terminal,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn interleaved_prompts_keep_text_metrics_and_terminal_states_separate() {
        let mut batch = BatchStream::new(2);
        batch.push(&json!({"index":1,"content":" class=\"bg-green-50\"><header>酒店</header><main><h1>四种客房</h1></main></body>","timings":{"predicted_n":60,"predicted_per_second":41.5}}),2000).unwrap();
        let coffee = batch.push(&json!({"index":0,"content":" class=\"bg-amber-50\"><header>咖啡馆</header></body>","timings":{"predicted_n":38,"predicted_per_second":42.2}}),2100).unwrap().unwrap();
        assert_eq!(coffee.index, 0);
        assert_eq!(coffee.stats.unwrap().output_tokens, 38);
        let hotel = batch.push(&json!({"index":1,"content":"","stop":true,"stop_type":"word","stopping_word":"</html>","timings":{"predicted_n":65,"predicted_per_second":41.7}}),2200).unwrap().unwrap();
        let (html, incomplete) = hotel.terminal.unwrap().unwrap();
        assert!(html.contains("四种客房"));
        assert!(!html.contains("咖啡馆"));
        assert!(html.ends_with("</html>"));
        assert!(!incomplete);
        assert!(!batch.all_finished());
        assert!(batch
            .push(&json!({"index":1,"content":"late duplicate"}), 2250)
            .unwrap()
            .is_none());
        let coffee = batch.push(&json!({"index":0,"content":"","stop":true,"stop_type":"limit","timings":{"predicted_n":40,"predicted_per_second":42.0}}),2300).unwrap().unwrap();
        let (html, incomplete) = coffee.terminal.unwrap().unwrap();
        assert!(html.contains("咖啡馆"));
        assert!(!html.contains("酒店"));
        assert!(incomplete);
        assert!(batch.all_finished());
        assert!(batch
            .push(&json!({"index":9,"content":"wrong"}), 2400)
            .is_err());
    }
}
