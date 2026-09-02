#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Incremental SSE reader. Chunks arrive from the HTTP body in arbitrary
/// sizes, so bytes are buffered and only split on `\n`; a multi-byte UTF-8
/// character never contains that byte, which keeps a character split across
/// two chunks intact.
#[derive(Debug, Default)]
pub struct SseStreamParser {
    buffer: Vec<u8>,
    event_name: String,
    data_lines: Vec<String>,
    saw_first_line: bool,
}

impl SseStreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_bytes(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=position).collect();
            let text = String::from_utf8_lossy(&line[..position]).into_owned();
            self.push_line(&text, &mut events);
        }
        events
    }

    pub fn finish(mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let text = String::from_utf8_lossy(&std::mem::take(&mut self.buffer)).into_owned();
            self.push_line(&text, &mut events);
        }
        flush_sse(&mut events, &mut self.event_name, &mut self.data_lines);
        events
    }

    fn push_line(&mut self, raw: &str, events: &mut Vec<SseEvent>) {
        let raw = if self.saw_first_line {
            raw
        } else {
            self.saw_first_line = true;
            raw.trim_start_matches('\u{feff}')
        };
        let line = raw.trim_end_matches('\r').trim_start();
        if line.is_empty() {
            flush_sse(events, &mut self.event_name, &mut self.data_lines);
            return;
        }
        if line.starts_with(':') {
            return;
        }
        if let Some(value) = line.strip_prefix("event:") {
            self.event_name = value.trim().to_string();
            return;
        }
        if let Some(value) = line.strip_prefix("data:") {
            let data = value.trim_start().to_string();
            if is_standalone_data_line(&data) {
                if !self.data_lines.is_empty() {
                    flush_sse(events, &mut self.event_name, &mut self.data_lines);
                }
                self.data_lines.push(data);
                flush_sse(events, &mut self.event_name, &mut self.data_lines);
                return;
            }
            self.data_lines.push(data);
        }
    }
}

pub fn parse_sse(text: &str) -> Vec<SseEvent> {
    let mut parser = SseStreamParser::new();
    let mut events = parser.push_bytes(text.as_bytes());
    events.extend(parser.finish());
    events
}

fn is_standalone_data_line(data: &str) -> bool {
    let trimmed = data.trim();
    if trimmed == "[DONE]" {
        return true;
    }
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
}

fn flush_sse(events: &mut Vec<SseEvent>, event_name: &mut String, data_lines: &mut Vec<String>) {
    if data_lines.is_empty() {
        event_name.clear();
        return;
    }
    let data = data_lines.join("\n");
    data_lines.clear();
    if data == "[DONE]" {
        event_name.clear();
        events.push(SseEvent {
            event: "done".to_string(),
            data: "[DONE]".to_string(),
        });
        return;
    }
    events.push(SseEvent {
        event: std::mem::take(event_name),
        data,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_and_data_only_events() {
        let events = parse_sse("event: ping\ndata: {\"ok\":true}\n\ndata: [DONE]\n");
        assert_eq!(events[0].event, "ping");
        assert_eq!(events[0].data, "{\"ok\":true}");
        assert_eq!(events[1].event, "done");
    }

    #[test]
    fn splits_complete_json_data_lines_without_blank_separators() {
        let events = parse_sse(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\ndata: {\"choices\":[{\"delta\":{\"content\":\" there\"}}]}\ndata: [DONE]\n",
        );
        assert_eq!(events.len(), 3);
        assert!(events[0].data.contains("hi"));
        assert!(events[1].data.contains("there"));
        assert_eq!(events[2].data, "[DONE]");
    }

    #[test]
    fn strips_bom_and_leading_whitespace() {
        let events = parse_sse("\u{feff}  data: {\"ok\":true}\n");
        assert_eq!(events[0].data, "{\"ok\":true}");
    }

    #[test]
    fn stream_parser_joins_events_split_across_chunks() {
        let mut parser = SseStreamParser::new();
        assert!(parser.push_bytes(b"event: response.out").is_empty());
        assert!(parser
            .push_bytes(b"put_text.delta\ndata: {\"del")
            .is_empty());
        let events = parser.push_bytes(b"ta\":\"hi\"}\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "response.output_text.delta");
        assert_eq!(events[0].data, "{\"delta\":\"hi\"}");
        assert!(parser.finish().is_empty());
    }

    #[test]
    fn stream_parser_keeps_multibyte_chars_split_across_chunks() {
        let payload = "data: {\"delta\":\"中文\"}\n\n".as_bytes();
        let split = payload
            .iter()
            .position(|byte| *byte == 0xe4)
            .expect("first byte of 中")
            + 1;
        let mut parser = SseStreamParser::new();
        let mut events = parser.push_bytes(&payload[..split]);
        events.extend(parser.push_bytes(&payload[split..]));
        events.extend(parser.finish());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "{\"delta\":\"中文\"}");
    }

    #[test]
    fn stream_parser_flushes_trailing_event_without_blank_line() {
        let mut parser = SseStreamParser::new();
        let events = parser.push_bytes(b"event: message_stop\ndata: {\"type\":\"stop\"}");
        assert!(events.is_empty());
        let tail = parser.finish();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].event, "message_stop");
    }

    #[test]
    fn stream_parser_strips_bom_from_first_chunk() {
        let mut parser = SseStreamParser::new();
        let events = parser.push_bytes("\u{feff}data: {\"ok\":true}\n\n".as_bytes());
        assert_eq!(events[0].data, "{\"ok\":true}");
    }
}
