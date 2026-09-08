//! Lossless UTF-8 splitting for text transports. Byte limits are conservative
//! relative to platform character limits; no delimiter or indentation is lost.
use crate::types::error::Temm1eError;

pub fn split_message(text: &str, max_bytes: usize) -> Result<Vec<String>, Temm1eError> {
    if max_bytes == 0 {
        return Err(Temm1eError::Channel(
            "message chunk limit must be positive".into(),
        ));
    }
    if text.is_empty() {
        return Ok(vec![String::new()]);
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while remaining.len() > max_bytes {
        let mut end = max_bytes;
        while !remaining.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            return Err(Temm1eError::Channel(
                "message chunk limit cannot fit the next UTF-8 character".into(),
            ));
        }
        let window = &remaining[..end];
        // Include the delimiter. A leading space/newline therefore always makes
        // progress and whitespace survives concatenation exactly.
        let split = window
            .rfind('\n')
            .or_else(|| window.rfind(' '))
            .map(|index| index + 1)
            .unwrap_or(end);
        chunks.push(remaining[..split].to_owned());
        remaining = &remaining[split..];
    }
    if !remaining.is_empty() {
        chunks.push(remaining.to_owned());
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn leading_delimiters_terminate_without_changing_text() {
        for prefix in [" ", "\n", "\n\n", "    "] {
            let text = format!("{prefix}{}", "a".repeat(9000));
            let chunks = split_message(&text, 4096).unwrap();
            assert_eq!(chunks.concat(), text);
            assert!(chunks
                .iter()
                .all(|chunk| !chunk.is_empty() && chunk.len() <= 4096));
            assert!(chunks.len() <= 4);
        }
    }
    #[test]
    fn unicode_and_code_indentation_round_trip_across_limits() {
        let text = "  fn test() {\n    // 🦀 Tiếng Việt\n\n    println!(\"你好\");\n}\n".repeat(25);
        for limit in 4..128 {
            let chunks = split_message(&text, limit).unwrap();
            assert_eq!(chunks.concat(), text);
            assert!(chunks
                .iter()
                .all(|chunk| !chunk.is_empty() && chunk.len() <= limit));
        }
        assert!(split_message("text", 0).is_err());
        assert!(split_message("a🦀", 3).is_err());
        assert_eq!(split_message("", 4).unwrap(), vec![""]);
    }
}
