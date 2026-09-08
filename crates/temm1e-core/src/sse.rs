//! Bounded incremental SSE framing for API transports. LF, CRLF and CR are
//! recognized without lossy decoding at network-chunk boundaries. Invalid UTF-8
//! and truncated data are explicit API errors rather than altered tool arguments.
use crate::types::error::Temm1eError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
    pub id: String,
}
pub struct SseDecoder {
    line: Vec<u8>,
    data: String,
    event: String,
    id: String,
    after_cr: bool,
    first_line: bool,
    data_seen: bool,
    max_bytes: usize,
}
impl SseDecoder {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            line: vec![],
            data: String::new(),
            event: String::new(),
            id: String::new(),
            after_cr: false,
            first_line: true,
            data_seen: false,
            max_bytes,
        }
    }
    fn error(message: &str) -> Temm1eError {
        Temm1eError::Provider(format!("SSE: {message}"))
    }
    pub fn push(&mut self, byte: u8) -> Result<Option<SseEvent>, Temm1eError> {
        if self.after_cr && byte == b'\n' {
            self.after_cr = false;
            return Ok(None);
        }
        self.after_cr = byte == b'\r';
        if byte != b'\n' && byte != b'\r' {
            if self.line.len() + self.data.len() + self.event.len() + self.id.len()
                >= self.max_bytes
            {
                return Err(Self::error("event exceeds configured byte limit"));
            }
            self.line.push(byte);
            return Ok(None);
        }
        let line = String::from_utf8(std::mem::take(&mut self.line))
            .map_err(|_| Self::error("invalid UTF-8"))?;
        let line = if self.first_line {
            self.first_line = false;
            line.strip_prefix('\u{feff}').unwrap_or(&line)
        } else {
            &line
        };
        if line.is_empty() {
            if !self.data_seen {
                self.event.clear();
                return Ok(None);
            }
            self.data_seen = false;
            self.data.pop(); // exactly one framing newline
            let event = std::mem::take(&mut self.event);
            return Ok(Some(SseEvent {
                event: if event.is_empty() {
                    "message".into()
                } else {
                    event
                },
                data: std::mem::take(&mut self.data),
                id: self.id.clone(),
            }));
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = line
            .split_once(':')
            .map(|(k, v)| (k, v.strip_prefix(' ').unwrap_or(v)))
            .unwrap_or((line, ""));
        match field {
            "data" => {
                self.data.push_str(value);
                self.data.push('\n');
                self.data_seen = true;
            }
            "event" => self.event = value.to_string(),
            "id" if !value.contains('\0') => self.id = value.to_string(),
            _ => {} // never automatically reconnect a provider POST
        }
        if self.data.len() + self.event.len() + self.id.len() > self.max_bytes {
            return Err(Self::error("event exceeds configured byte limit"));
        }
        Ok(None)
    }
    pub fn finish(&self) -> Result<(), Temm1eError> {
        if self.data_seen || !self.line.is_empty() {
            Err(Self::error("stream ended inside an event"))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_multiline_bom_and_all_line_endings_survive_every_split() {
        for newline in ["\n", "\r\n", "\r"] {
            let input=format!("\u{feff}: heartbeat{newline}event: delta{newline}id: 7{newline}data: café 🐈{newline}data: 中文{newline}{newline}data:{newline}{newline}");
            for split in 0..=input.len() {
                let mut decoder = SseDecoder::new(1024);
                let mut events = vec![];
                for chunk in [&input.as_bytes()[..split], &input.as_bytes()[split..]] {
                    for byte in chunk {
                        if let Some(event) = decoder.push(*byte).unwrap() {
                            events.push(event);
                        }
                    }
                }
                decoder.finish().unwrap();
                assert_eq!(
                    events,
                    vec![
                        SseEvent {
                            event: "delta".into(),
                            data: "café 🐈\n中文".into(),
                            id: "7".into()
                        },
                        SseEvent {
                            event: "message".into(),
                            data: "".into(),
                            id: "7".into()
                        }
                    ]
                );
            }
        }
    }
    #[test]
    fn malformed_truncated_and_oversized_events_fail_explicitly() {
        let mut decoder = SseDecoder::new(20);
        for byte in b"data: missing blank\n" {
            decoder.push(*byte).unwrap();
        }
        assert!(decoder.finish().is_err());
        let mut decoder = SseDecoder::new(100);
        assert!(b"data: \xff\n\n"
            .iter()
            .try_for_each(|b| decoder.push(*b).map(|_| ()))
            .is_err());
        let mut decoder = SseDecoder::new(10);
        assert!(b"data: 0123456789\n\n"
            .iter()
            .try_for_each(|b| decoder.push(*b).map(|_| ()))
            .is_err());
    }
}
