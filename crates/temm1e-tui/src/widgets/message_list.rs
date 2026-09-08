//! Scrollable message list widget.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::markdown::RenderedLine;

/// Message role for display styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Agent,
    System,
    Tool,
}

/// Usage statistics for a single turn.
#[derive(Debug, Clone, Default)]
pub struct TurnUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: f64,
    pub elapsed_ms: u64,
}

/// A display-ready message.
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub tool_id: Option<String>,
    pub role: MessageRole,
    pub content: Vec<RenderedLine>,
    pub timestamp: DateTime<Utc>,
    pub usage: Option<TurnUsage>,
}

/// Message list state.
pub struct MessageList {
    pub messages: VecDeque<DisplayMessage>,
    pub scroll_offset: usize,
    pub max_messages: usize,
}

impl Default for MessageList {
    fn default() -> Self {
        Self {
            messages: VecDeque::new(),
            scroll_offset: 0,
            max_messages: 10_000,
        }
    }
}

impl MessageList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, msg: DisplayMessage) {
        let was_at_bottom = self.scroll_offset == 0;
        self.messages.push_back(msg);
        if self.messages.len() > self.max_messages {
            self.messages.pop_front();
            if self.scroll_offset > 0 {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
        }
        // Only auto-scroll if already at bottom — don't snap away from history
        if was_at_bottom {
            self.scroll_to_bottom();
        }
    }

    pub fn scroll_up(&mut self, amount: usize) {
        // Allow scrolling up generously — chat view clamps to actual content
        self.scroll_offset = self.scroll_offset.saturating_add(amount).min(100_000);
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.scroll_offset = 0;
    }

    /// Total number of rendered lines across all messages.
    ///
    /// Must match exactly what `render_lines` produces: for each
    /// message, `content.len()` rows + 1 usage row (if present) +
    /// 1 blank-line separator. Streaming content is not included
    /// here — callers that need it must add `streaming_renderer.lines().len()`
    /// separately.
    ///
    /// Used by the drag-to-select logic to convert between terminal
    /// row coordinates and absolute line indices so selection
    /// tracks content as the list scrolls.
    pub fn line_count(&self) -> usize {
        let mut total = 0;
        for msg in &self.messages {
            total += msg.content.len();
            if msg.usage.is_some() {
                total += 1;
            }
            total += usize::from(msg.role != MessageRole::Tool); // blank separator
        }
        total
    }

    /// Render messages into ratatui Lines for display.
    pub fn render_lines(
        &self,
        user_style: Style,
        agent_style: Style,
        system_style: Style,
        secondary_style: Style,
    ) -> Vec<Line<'static>> {
        self.render_range(
            0..usize::MAX,
            user_style,
            agent_style,
            system_style,
            secondary_style,
        )
    }

    /// Materialize only the requested viewport. Off-screen text and spans are
    /// never cloned; message lengths are enough to skip older content.
    pub fn render_range(
        &self,
        range: std::ops::Range<usize>,
        user_style: Style,
        agent_style: Style,
        system_style: Style,
        secondary_style: Style,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let mut row = 0usize;
        for msg in &self.messages {
            let separator = usize::from(msg.role != MessageRole::Tool);
            let message_rows = msg.content.len() + usize::from(msg.usage.is_some()) + separator;
            let next = row.saturating_add(message_rows);
            if row >= range.end {
                break;
            }
            if next <= range.start {
                row = next;
                continue;
            }
            let (prefix, prefix_style) = match msg.role {
                MessageRole::User => ("> ", user_style),
                MessageRole::Agent => ("", agent_style),
                MessageRole::System => ("[system] ", system_style),
                MessageRole::Tool => ("", secondary_style),
            };
            let first = range.start.saturating_sub(row).min(msg.content.len());
            let last = range.end.saturating_sub(row).min(msg.content.len());
            for rendered in &msg.content[first..last] {
                let mut spans = Vec::new();
                if !prefix.is_empty() {
                    spans.push(Span::styled(prefix, prefix_style));
                }
                spans.extend(rendered.spans.clone());
                lines.push(Line::from(spans));
            }
            let usage_row = row + msg.content.len();
            if range.contains(&usage_row) {
                if let Some(usage) = &msg.usage {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "  [{} in / {} out | {} | {:.1}s]",
                            usage.input_tokens,
                            usage.output_tokens,
                            if usage.cost_usd > 0.0 {
                                format!("Token est. ${:.4}", usage.cost_usd)
                            } else {
                                "cost unavailable".into()
                            },
                            usage.elapsed_ms as f64 / 1000.0,
                        ),
                        secondary_style,
                    )));
                }
            }
            if separator != 0 && range.contains(&(next - 1)) {
                lines.push(Line::from(""));
            }
            row = next;
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewport_preserves_content_usage_and_separators() {
        let mut messages = MessageList::new();
        for i in 0..1000 {
            messages.push(DisplayMessage {
                tool_id: None,
                role: if i % 2 == 0 {
                    MessageRole::User
                } else {
                    MessageRole::Agent
                },
                content: vec![RenderedLine {
                    spans: vec![Span::raw(format!("message {i}"))],
                    indent: 0,
                }],
                timestamp: Utc::now(),
                usage: (i % 3 == 0).then(TurnUsage::default),
            });
        }
        let style = Style::default();
        let all = messages.render_lines(style, style, style, style);
        assert_eq!(all.len(), messages.line_count());
        for start in [0, 1, 2, 3, 99, all.len() - 20, all.len()] {
            let end = (start + 17).min(all.len());
            let viewport = messages.render_range(start..end, style, style, style, style);
            assert_eq!(viewport, all[start..end]);
            assert!(viewport.len() <= 17);
        }
        assert_eq!(all[0].to_string(), "> message 0");
        assert!(all[1].to_string().contains("0 in / 0 out"));
        assert_eq!(all[2].to_string(), "");
    }
}
