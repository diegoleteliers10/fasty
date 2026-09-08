use std::io::BufRead;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event_type: Option<String>,
    pub data: String,
    pub id: Option<String>,
}

pub fn read_sse_event<R: BufRead>(reader: &mut R) -> std::io::Result<Option<SseEvent>> {
    let mut event_type: Option<String> = None;
    let mut data_lines: Vec<String> = Vec::new();
    let mut id: Option<String> = None;
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            // EOF
            if !data_lines.is_empty() || event_type.is_some() {
                return Ok(Some(SseEvent {
                    event_type,
                    data: data_lines.join("\n"),
                    id,
                }));
            }
            return Ok(None);
        }

        let trimmed = line.trim_end_matches(&['\r', '\n'][..]);

        if trimmed.is_empty() {
            // Blank line triggers event dispatch
            if !data_lines.is_empty() || event_type.is_some() {
                return Ok(Some(SseEvent {
                    event_type,
                    data: data_lines.join("\n"),
                    id,
                }));
            }
            continue;
        }

        if trimmed.starts_with(':') {
            // SSE comment or keepalive ping, ignore
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("data:") {
            let val = rest.strip_prefix(' ').unwrap_or(rest);
            data_lines.push(val.to_string());
        } else if let Some(rest) = trimmed.strip_prefix("event:") {
            let val = rest.strip_prefix(' ').unwrap_or(rest);
            event_type = Some(val.to_string());
        } else if let Some(rest) = trimmed.strip_prefix("id:") {
            let val = rest.strip_prefix(' ').unwrap_or(rest);
            id = Some(val.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_parse_simple_sse() {
        let raw = "event: message\ndata: {\"text\": \"hello\"}\n\n";
        let mut cursor = Cursor::new(raw);
        let ev = read_sse_event(&mut cursor).unwrap().expect("should have event");
        assert_eq!(ev.event_type.as_deref(), Some("message"));
        assert_eq!(ev.data, "{\"text\": \"hello\"}");
    }

    #[test]
    fn test_parse_multiline_data_and_comments() {
        let raw = ": ping\ndata: line1\ndata: line2\n\n";
        let mut cursor = Cursor::new(raw);
        let ev = read_sse_event(&mut cursor).unwrap().expect("should have event");
        assert_eq!(ev.data, "line1\nline2");
    }
}
