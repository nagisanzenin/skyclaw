//! Incremental streaming markdown renderer.
//!
//! Buffers network deltas and renders at most once per UI frame. Provisional
//! previews are bounded; the final response remains authoritative.

use ratatui::style::Style;

use crate::widgets::markdown::{render_markdown, RenderedLine};

/// Renders streaming markdown incrementally.
pub struct StreamingRenderer {
    buffer: String,
    dirty: bool,
    clipped: bool,
    rendered_lines: Vec<RenderedLine>,
    base_style: Style,
    heading_style: Style,
    code_style: Style,
    link_style: Style,
    quote_style: Style,
}

impl StreamingRenderer {
    pub fn new(
        base_style: Style,
        heading_style: Style,
        code_style: Style,
        link_style: Style,
        quote_style: Style,
    ) -> Self {
        Self {
            buffer: String::new(),
            dirty: false,
            clipped: false,
            rendered_lines: Vec::new(),
            base_style,
            heading_style,
            code_style,
            link_style,
            quote_style,
        }
    }

    /// Append a text chunk from the stream.
    pub fn push(&mut self, delta: &str) {
        const MAX_PREVIEW: usize = 256 * 1024;
        if self.clipped {
            return;
        }
        let remaining = MAX_PREVIEW.saturating_sub(self.buffer.len());
        let mut end = delta.len().min(remaining);
        while !delta.is_char_boundary(end) {
            end -= 1;
        }
        self.buffer.push_str(&delta[..end]);
        if end < delta.len() {
            self.buffer
                .push_str("\n\n[Preview limit reached; waiting for final response.]");
            self.clipped = true;
        }
        self.dirty = true;
    }

    /// Get the current rendered lines.
    pub fn lines(&self) -> &[RenderedLine] {
        &self.rendered_lines
    }

    /// Get the raw buffer text.
    pub fn text(&self) -> &str {
        &self.buffer
    }

    /// Reset for a new message.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.rendered_lines.clear();
        self.dirty = false;
        self.clipped = false;
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn refresh(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.rendered_lines = render_markdown(
            &self.buffer,
            self.base_style,
            self.heading_style,
            self.code_style,
            self.link_style,
            self.quote_style,
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rendering_is_batched_and_unicode_preview_is_bounded() {
        let style = ratatui::style::Style::default();
        let mut renderer = super::StreamingRenderer::new(style, style, style, style, style);
        renderer.push("hello");
        assert!(renderer.lines().is_empty());
        renderer.refresh();
        assert!(!renderer.lines().is_empty());
        renderer.push(&"🐈".repeat(100_000));
        assert!(renderer.text().len() < 256 * 1024 + 100);
        assert!(renderer.text().contains("Preview limit reached"));
        let before = renderer.text().to_owned();
        renderer.push("ignored preview overflow");
        assert_eq!(renderer.text(), before);
        renderer.reset();
        renderer.push("new request");
        assert_eq!(renderer.text(), "new request");
    }
}
