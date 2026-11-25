//! Demultiplex Albatross SSE choices by their original prompt index.
use crate::{extract_html, metrics::InferenceMetrics, prompt::HTML_PREFILL};
use serde_json::Value;

const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;

struct Lane {
    text: String,
    first_token_ms: Option<u64>,
    finished: bool,
}

pub struct BatchStream {
    lanes: Vec<Lane>,
    request_id: Option<String>,
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
                    first_token_ms: None,
                    finished: false,
                })
                .collect(),
            request_id: None,
        }
    }

    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    pub fn all_finished(&self) -> bool {
        self.lanes.iter().all(|lane| lane.finished)
    }

    pub fn push(&mut self, value: &Value, elapsed_ms: u64) -> Result<Vec<Update>, String> {
        if let Some(id) = value["id"].as_str() {
            if id.is_empty() || id.len() > 128 || self.request_id().is_some_and(|known| known != id)
            {
                return Err("Albatross 流返回了不一致的请求 ID".into());
            }
            if self.request_id.is_none() {
                self.request_id = Some(id.into());
            }
        }
        if let Some(error) = value.get("error") {
            let message = error
                .as_str()
                .or_else(|| error["message"].as_str())
                .unwrap_or("服务返回未知错误");
            return Err(format!(
                "Albatross 生成失败：{}",
                message.chars().take(250).collect::<String>()
            ));
        }
        let choices = value["choices"]
            .as_array()
            .ok_or("Albatross 流缺少方案信息")?;
        let mut updates = Vec::with_capacity(choices.len());
        for choice in choices {
            let index = choice["index"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or("Albatross 流缺少方案编号")?;
            let lane = self
                .lanes
                .get_mut(index)
                .ok_or("Albatross 流的方案编号越界")?;
            if lane.finished {
                continue;
            }
            let delta = choice["delta"]["content"]
                .as_str()
                .filter(|text| !text.is_empty());
            let first = delta.is_some() && lane.first_token_ms.is_none();
            if let Some(delta) = delta {
                lane.text.push_str(delta);
                if lane.text.len() > MAX_HTML_BYTES {
                    return Err("生成内容超过限制".into());
                }
                if first {
                    lane.first_token_ms = Some(elapsed_ms);
                }
            }
            let terminal = match choice["finish_reason"].as_str() {
                Some("stop" | "length") => {
                    lane.finished = true;
                    let marker = choice["matched_stop"].as_str();
                    let text = match marker {
                        Some(marker @ ("\n\nUser:" | "<EOD>")) => {
                            lane.text.strip_suffix(marker).unwrap_or(&lane.text)
                        }
                        _ => &lane.text,
                    };
                    Some(if text == HTML_PREFILL {
                        Err("Albatross 没有返回网页内容，请重试".into())
                    } else {
                        extract_html(text).map(|html| {
                            let incomplete = !html.to_ascii_lowercase().ends_with("</html>");
                            (html, incomplete)
                        })
                    })
                }
                Some(_) => return Err("Albatross 返回了未知的生成结束原因".into()),
                None => None,
            };
            let stats = (first || terminal.is_some()).then_some(InferenceMetrics {
                elapsed_ms,
                first_token_ms: lane.first_token_ms,
                ..Default::default()
            });
            if delta.is_some() || terminal.is_some() {
                updates.push(Update {
                    index,
                    delta: delta.map(str::to_owned),
                    stats,
                    terminal,
                });
            }
        }
        Ok(updates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn separates_multiple_choices_and_preserves_real_closing_tags() {
        let mut batch = BatchStream::new(2);
        let updates = batch
            .push(
                &json!({"id":"req-1","choices":[
                    {"index":1,"delta":{"content":"><h1>酒店</h1></body></ht"}},
                    {"index":0,"delta":{"content":"><h1>咖啡馆</h1>"}}
                ]}),
                500,
            )
            .unwrap();
        assert_eq!(updates[0].index, 1);
        assert_eq!(updates[0].stats.as_ref().unwrap().first_token_ms, Some(500));
        assert_eq!(updates[0].stats.as_ref().unwrap().output_tokens, 0);
        batch
            .push(
                &json!({"id":"req-1","choices":[{"index":1,"delta":{"content":"ml>"}}]}),
                600,
            )
            .unwrap();
        let mut updates = batch
            .push(
                &json!({"id":"req-1","choices":[
                    {"index":1,"delta":{},"finish_reason":"stop","matched_stop":"</html>"},
                    {"index":0,"delta":{},"finish_reason":"length"}
                ]}),
                800,
            )
            .unwrap()
            .into_iter();
        let (hotel, incomplete) = updates.next().unwrap().terminal.unwrap().unwrap();
        assert!(hotel.contains("酒店") && hotel.ends_with("</html>") && !incomplete);
        let (coffee, incomplete) = updates.next().unwrap().terminal.unwrap().unwrap();
        assert!(coffee.contains("咖啡馆") && !coffee.contains("酒店") && incomplete);
        assert_eq!(batch.request_id(), Some("req-1"));
        assert!(batch.all_finished());
    }

    #[test]
    fn rejects_bad_indices_and_server_errors() {
        let mut batch = BatchStream::new(2);
        assert!(batch
            .push(&json!({"id":"req-1","choices":[{"index":3,"delta":{}}]}), 1)
            .is_err());
        assert!(batch.push(&json!({"id":"req-2","choices":[]}), 1).is_err());
        assert!(
            matches!(batch.push(&json!({"error":"GPU memory exhausted"}), 1), Err(error) if error.contains("GPU memory exhausted"))
        );
    }

    #[test]
    fn preserves_partial_html_and_removes_the_model_end_marker() {
        let mut batch = BatchStream::new(1);
        let mut updates = batch
            .push(
                &json!({"id":"req-1","choices":[{
                    "index":0,"delta":{"content":"><h1>尚未完成</h1><EOD>"},
                    "finish_reason":"stop","matched_stop":"<EOD>"
                }]}),
                900,
            )
            .unwrap();
        let (html, incomplete) = updates.remove(0).terminal.unwrap().unwrap();
        assert!(html.ends_with("</h1>"));
        assert!(incomplete);
    }
}
