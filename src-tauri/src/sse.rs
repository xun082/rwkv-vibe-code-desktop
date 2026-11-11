//! Buffer bytes rather than strings: UTF-8 characters and SSE events may span HTTP chunks.
const MAX_EVENT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Default)]
pub struct SseDecoder {
    pending: Vec<u8>,
    data: Option<String>,
}
impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        let mut consumed = 0;
        for raw in self.pending.split_inclusive(|byte| *byte == b'\n') {
            let Some(line) = raw.strip_suffix(b"\n") else {
                break;
            };
            if line.len() > MAX_EVENT_BYTES {
                return Err("流式事件过大".into());
            }
            consumed += raw.len();
            let line = std::str::from_utf8(line.strip_suffix(b"\r").unwrap_or(line))
                .map_err(|e| e.to_string())?;
            if line.is_empty() {
                if let Some(data) = self.data.take() {
                    events.push(data);
                }
            } else if let Some(value) = line.strip_prefix("data:") {
                let value = value.strip_prefix(' ').unwrap_or(value);
                if let Some(data) = self.data.as_mut() {
                    if data.len() + 1 + value.len() > MAX_EVENT_BYTES {
                        return Err("流式事件过大".into());
                    }
                    data.push('\n');
                    data.push_str(value);
                } else {
                    self.data = Some(value.to_owned());
                }
            }
        }
        self.pending.drain(..consumed);
        if self.pending.len() > MAX_EVENT_BYTES {
            return Err("流式事件过大".into());
        }
        Ok(events)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_utf8_at_every_possible_byte_boundary() {
        let input = "data: {\"text\":\"中文网页🌱\"}\r\n\r\ndata: [DONE]\n\n".as_bytes();
        for split in 0..=input.len() {
            let mut decoder = SseDecoder::default();
            let mut out = decoder.push(&input[..split]).unwrap();
            out.extend(decoder.push(&input[split..]).unwrap());
            assert_eq!(out, ["{\"text\":\"中文网页🌱\"}", "[DONE]"]);
        }
    }
    #[test]
    fn joins_data_lines_and_ignores_comments() {
        let mut decoder = SseDecoder::default();
        assert_eq!(
            decoder
                .push(b": heartbeat\ndata: one\ndata: two\n\n")
                .unwrap(),
            ["one\ntwo"]
        );
    }
    #[test]
    fn limits_multiline_events_across_chunks() {
        let line = format!("data: {}\n", "a".repeat(MAX_EVENT_BYTES / 2));
        let mut decoder = SseDecoder::default();
        assert!(decoder.push(line.as_bytes()).unwrap().is_empty());
        assert!(decoder.push(line.as_bytes()).is_err());
    }
}
