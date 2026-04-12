//! LLM-optimized output formatting for web_search results.
//!
//! Renders DispatcherOutput into a scannable text format with:
//! - Top-of-output query + status line
//! - Per-hit blocks with title/url/signal/snippet
//! - Footer with discoverability (used / available / disabled / custom)
//! - Truncation transparency and refinement hints
//!
//! Hard guarantee: output bytes ≤ req.max_total_chars (UTF-8 safe).

use crate::web_search::types::*;
use std::fmt::Write as _;

/// Truncate a string to at most `max_bytes` bytes on a UTF-8 char boundary.
///
/// Per MEMORY.md UTF-8 safety rule (the 2026-03-09 Vietnamese `ẹ` incident),
/// NEVER use `&s[..n]` on user-derived strings. Always walk back to a char
/// boundary first.
pub fn truncate_safe(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Render a DispatcherOutput into the LLM-optimized text format.
pub fn render(out: &DispatcherOutput) -> String {
    if let Some(err) = &out.input_error {
        return format!("Search input rejected: {err}\n");
    }

    let max_total = out.req.max_total_chars;

    // Build header
    let mut s = String::with_capacity(max_total.min(8192));
    let _ = writeln!(s, "Search results for: \"{}\"", out.query);

    let used: Vec<&str> = out.backends_succeeded.iter().map(|n| n.as_str()).collect();
    let used_str = if used.is_empty() {
        "(none)".to_string()
    } else {
        used.join(", ")
    };
    let _ = writeln!(
        s,
        "{} of {} found · used: {}",
        out.hits.len(),
        out.total_candidates_before_truncation.max(out.hits.len()),
        used_str
    );
    s.push('\n');

    // Render hits with progressive degradation as budget tightens
    // Reserve room for the footer (~600 chars max in practice)
    let footer_reserve: usize = 800;
    let hit_budget = max_total.saturating_sub(s.len() + footer_reserve);
    let (hits_text, hits_shown) = render_hits(&out.hits, out.req.max_snippet_chars, hit_budget);
    s.push_str(&hits_text);

    // Footer block
    let footer = render_footer(out, hits_shown);
    s.push_str(&footer);

    // Final hard cap — UTF-8 safe — never exceed max_total_chars.
    let result = truncate_safe(&s, max_total);
    debug_assert!(
        result.len() <= max_total,
        "format violated max_total_chars: {} > {}",
        result.len(),
        max_total
    );
    result
}

/// Render hits with progressive degradation. Returns (text, hits_actually_shown).
fn render_hits(hits: &[SearchHit], max_snippet: usize, budget: usize) -> (String, usize) {
    let mut s = String::new();
    let mut shown = 0;

    for (i, hit) in hits.iter().enumerate() {
        // Decide degradation level based on position vs remaining budget
        let position = i + 1;
        let remaining = budget.saturating_sub(s.len());

        // Minimum guarantee: first 3 hits keep their full snippet
        let allow_snippet = position <= 3 || remaining > 400;
        let allow_signal = remaining > 100;

        let mut block = String::new();
        let _ = writeln!(block, "[{}] {}", position, truncate_safe(&hit.title, 200));
        let _ = writeln!(block, "    {}", hit.url);

        if allow_signal {
            if let Some(signal_line) = render_signal(&hit.signal, hit.published.as_deref()) {
                let _ = writeln!(block, "    {signal_line}");
            }
        }

        if allow_snippet && !hit.snippet.is_empty() {
            let snippet = truncate_safe(&hit.snippet, max_snippet);
            for line in wrap_lines(&snippet, 70).iter().take(3) {
                let _ = writeln!(block, "    {line}");
            }
        }

        // Source line — only when multi-source
        if !hit.also_in.is_empty() {
            let _ = writeln!(
                block,
                "    source: {} · also: {}",
                hit.source_name,
                hit.also_in.join(", ")
            );
        } else {
            let _ = writeln!(block, "    source: {}", hit.source_name);
        }
        block.push('\n');

        // Budget check: if adding this block would overflow, stop here
        if s.len() + block.len() > budget && shown >= 3 {
            break;
        }

        s.push_str(&block);
        shown += 1;
    }

    if shown < hits.len() {
        let dropped = hits.len() - shown;
        let _ = writeln!(s, "... ({} more, dropped due to budget)", dropped);
        s.push('\n');
    }

    (s, shown)
}

/// Wrap text into lines of approximately `width` characters, breaking at spaces.
fn wrap_lines(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Render the per-hit signal line (e.g. "★ 2,847 · Rust").
fn render_signal(signal: &Option<HitSignal>, published: Option<&str>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(sig) = signal {
        match sig {
            HitSignal::HnPoints { points, comments } => {
                parts.push(format!("↑ {points} points"));
                parts.push(format!("{comments} comments"));
            }
            HitSignal::GithubStars { stars, language } => {
                parts.push(format!("★ {}", format_thousands(*stars)));
                if let Some(lang) = language {
                    parts.push(lang.clone());
                }
            }
            HitSignal::StackOverflowScore {
                score,
                answers,
                accepted,
            } => {
                parts.push(format!("{score} score"));
                parts.push(format!("{answers} answers"));
                if *accepted {
                    parts.push("✓ accepted".into());
                }
            }
            HitSignal::RedditUpvotes {
                ups,
                comments,
                subreddit,
            } => {
                parts.push(format!("↑ {ups}"));
                parts.push(format!("{comments} comments"));
                parts.push(format!("r/{subreddit}"));
            }
            HitSignal::ArxivAuthors {
                authors,
                primary_category,
            } => {
                let names = authors
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                if !names.is_empty() {
                    parts.push(format!("📄 {names}"));
                }
                if let Some(cat) = primary_category {
                    parts.push(cat.clone());
                }
            }
            HitSignal::PubmedAuthors { authors, journal } => {
                let names = authors
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                if !names.is_empty() {
                    parts.push(format!("📄 {names}"));
                }
                parts.push(journal.clone());
            }
            HitSignal::Wikipedia { description } => {
                if let Some(d) = description {
                    parts.push(d.clone());
                }
            }
            HitSignal::MarginaliaQuality { quality } => {
                parts.push(format!("quality {:.2}", quality));
            }
        }
    }
    if let Some(p) = published {
        parts.push(p.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn format_thousands(n: u32) -> String {
    let s = n.to_string();
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*c);
    }
    out
}

/// Render the footer block: catalog + skip/fail/clamp/hint lines.
fn render_footer(out: &DispatcherOutput, hits_shown: usize) -> String {
    let mut s = String::new();
    s.push_str("─────\n");

    // Used backends
    if !out.backends_succeeded.is_empty() {
        let _ = writeln!(s, "Used:        {}", out.backends_succeeded.join(", "));
    }

    // Available built-in (excluding the ones already in Used)
    if !out.catalog.available.is_empty() {
        let _ = writeln!(s, "Available:   {}", out.catalog.available.join(", "));
    }

    // Disabled with hint (paid backends behind env vars)
    if !out.catalog.disabled_with_hint.is_empty() {
        let parts: Vec<String> = out
            .catalog
            .disabled_with_hint
            .iter()
            .map(|(name, hint)| format!("{name} (set {hint})"))
            .collect();
        let _ = writeln!(s, "Not enabled: {}", parts.join(", "));
    }

    // Custom backends
    if !out.catalog.custom.is_empty() {
        let _ = writeln!(s, "Custom:      {}", out.catalog.custom.join(", "));
    }

    // Failed backends
    if !out.backends_failed.is_empty() {
        let parts: Vec<String> = out
            .backends_failed
            .iter()
            .map(|(name, err)| format!("{name} ({err})"))
            .collect();
        let _ = writeln!(s, "Failed:      {}", parts.join(", "));
    }

    // Skipped backends
    if !out.backends_skipped.is_empty() {
        let parts: Vec<String> = out
            .backends_skipped
            .iter()
            .map(|(name, reason)| format!("{name} ({reason})"))
            .collect();
        let _ = writeln!(s, "Skipped:     {}", parts.join(", "));
    }

    // Clamps applied
    if !out.clamps_applied.is_empty() {
        let _ = writeln!(s, "Clamped:     {}", out.clamps_applied.join(", "));
    }

    // Truncation hint
    if hits_shown < out.hits.len() {
        let dropped = out.hits.len() - hits_shown;
        let suggested_total = (out.req.max_total_chars * 2).min(HARD_MAX_TOTAL_CHARS);
        let suggested_results = (out.req.max_results + 10).min(HARD_MAX_RESULTS);
        let _ = writeln!(
            s,
            "Truncated:   {dropped} hits dropped (over budget) — try max_results={suggested_results} or max_total_chars={suggested_total}"
        );
    }

    // Refinement hint when results are thin
    if let Some(hint) = maybe_emit_hint(out) {
        let _ = writeln!(s, "Hint:        {hint}");
    }

    s
}

fn maybe_emit_hint(out: &DispatcherOutput) -> Option<String> {
    if out.hits.len() >= 3 {
        return None;
    }
    let stronger: Vec<&String> = out
        .catalog
        .available
        .iter()
        .filter(|name| !out.backends_succeeded.contains(name))
        .collect();
    if stronger.is_empty() {
        if !out.catalog.disabled_with_hint.is_empty() {
            let names: Vec<&str> = out
                .catalog
                .disabled_with_hint
                .iter()
                .map(|(n, _)| n.as_str())
                .collect();
            return Some(format!(
                "results look thin. Set one of: {} for premium search.",
                names.join(", ")
            ));
        }
        return None;
    }
    Some(format!(
        "results look thin. Try `backends=[\"{}\"]` for broader coverage.",
        stronger[0]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_request(max_total: usize, max_snippet: usize) -> SearchRequest {
        SearchRequest {
            query: "test".into(),
            max_results: 10,
            max_total_chars: max_total,
            max_snippet_chars: max_snippet,
            time_range: TimeRange::All,
            category: None,
            language: None,
            region: None,
            include_domains: vec![],
            exclude_domains: vec![],
            sort: SortOrder::Relevance,
        }
    }

    fn fake_hit(idx: usize, source_name: &str) -> SearchHit {
        SearchHit {
            title: format!("Result {idx}"),
            url: format!("https://example.com/{idx}"),
            snippet: format!("This is the snippet for result {idx}, with some text to fill space."),
            source: BackendId::Wikipedia,
            source_name: source_name.into(),
            published: None,
            score: 1.0 - (idx as f32 * 0.1),
            signal: None,
            also_in: vec![],
        }
    }

    fn fake_output(hits: Vec<SearchHit>, req: SearchRequest) -> DispatcherOutput {
        let total = hits.len();
        DispatcherOutput {
            query: "test query".into(),
            req,
            hits,
            total_candidates_before_truncation: total,
            backends_succeeded: vec!["wikipedia".into()],
            backends_failed: vec![],
            backends_skipped: vec![],
            catalog: Catalog {
                available: vec!["wikipedia".into(), "hackernews".into()],
                disabled_with_hint: vec![],
                custom: vec![],
            },
            clamps_applied: vec![],
            input_error: None,
        }
    }

    // ── truncate_safe tests ─────────────────────────────────────────────

    #[test]
    fn truncate_safe_preserves_ascii() {
        assert_eq!(truncate_safe("hello world", 100), "hello world");
        assert_eq!(truncate_safe("hello world", 5), "hello");
    }

    #[test]
    fn truncate_safe_handles_vietnamese_e_at_boundary() {
        // ẹ is 3 bytes (U+1EB9)
        let s = "abcẹdef"; // bytes: a b c [e1 ba b9] d e f → 9 bytes
        let truncated = truncate_safe(s, 4);
        // byte 4 falls inside the ẹ multibyte sequence; should walk back to byte 3
        assert_eq!(truncated, "abc");
    }

    #[test]
    fn truncate_safe_handles_chinese() {
        // 你 is 3 bytes
        let s = "x你好y";
        let truncated = truncate_safe(s, 5);
        // byte 5 is mid-好; walk back to end of 你
        assert_eq!(truncated, "x你");
    }

    #[test]
    fn truncate_safe_handles_emoji() {
        // 🦀 is 4 bytes
        let s = "rust🦀ferris";
        let truncated = truncate_safe(s, 6);
        // byte 6 is mid-🦀; walk back
        assert_eq!(truncated, "rust");
    }

    #[test]
    fn truncate_safe_zero_max_returns_empty() {
        assert_eq!(truncate_safe("anything", 0), "");
    }

    // ── format tests ────────────────────────────────────────────────────

    #[test]
    fn format_renders_empty_results_gracefully() {
        let req = fake_request(8000, 200);
        let out = DispatcherOutput {
            query: "test".into(),
            req,
            hits: vec![],
            total_candidates_before_truncation: 0,
            backends_succeeded: vec![],
            backends_failed: vec![],
            backends_skipped: vec![],
            catalog: Catalog::default(),
            clamps_applied: vec![],
            input_error: None,
        };
        let s = render(&out);
        assert!(s.contains("Search results for: \"test\""));
        assert!(s.contains("0 of 0 found"));
    }

    #[test]
    fn format_renders_full_example_under_budget() {
        let req = fake_request(8000, 200);
        let hits: Vec<SearchHit> = (1..=5).map(|i| fake_hit(i, "wikipedia")).collect();
        let out = fake_output(hits, req);
        let s = render(&out);
        assert!(s.contains("[1] Result 1"));
        assert!(s.contains("[5] Result 5"));
        assert!(s.contains("─────"));
        assert!(s.contains("Used:"));
        assert!(s.len() < 8000);
    }

    #[test]
    fn format_truncates_to_max_total_chars() {
        let req = fake_request(2000, 500);
        // Create 30 hits with long snippets — way over budget
        let hits: Vec<SearchHit> = (1..=30)
            .map(|i| {
                let mut h = fake_hit(i, "wikipedia");
                h.snippet = "very long snippet ".repeat(20);
                h
            })
            .collect();
        let out = fake_output(hits, req);
        let s = render(&out);
        assert!(
            s.len() <= 2000,
            "output was {} bytes, expected <= 2000",
            s.len()
        );
    }

    #[test]
    fn format_keeps_first_3_full_at_minimum() {
        let req = fake_request(2000, 200);
        let hits: Vec<SearchHit> = (1..=10).map(|i| fake_hit(i, "wikipedia")).collect();
        let out = fake_output(hits, req);
        let s = render(&out);
        // First 3 results' titles must always appear
        assert!(s.contains("[1] Result 1"));
        assert!(s.contains("[2] Result 2"));
        assert!(s.contains("[3] Result 3"));
    }

    #[test]
    fn format_drops_hits_from_bottom_when_over_budget() {
        let req = fake_request(1500, 300);
        let hits: Vec<SearchHit> = (1..=20)
            .map(|i| {
                let mut h = fake_hit(i, "wikipedia");
                h.snippet = "lorem ipsum dolor sit amet ".repeat(5);
                h
            })
            .collect();
        let out = fake_output(hits, req);
        let s = render(&out);
        // Should report truncation
        assert!(
            s.contains("Truncated:") || s.contains("dropped due to budget"),
            "expected truncation marker, got:\n{s}"
        );
        assert!(s.len() <= 1500);
    }

    #[test]
    fn format_footer_always_present_even_at_cap() {
        let req = fake_request(1000, 200);
        let hits: Vec<SearchHit> = (1..=15).map(|i| fake_hit(i, "wikipedia")).collect();
        let out = fake_output(hits, req);
        let s = render(&out);
        assert!(s.contains("─────"), "footer separator missing:\n{s}");
        assert!(
            s.contains("Used:") || s.contains("Available:"),
            "footer body missing:\n{s}"
        );
    }

    #[test]
    fn format_footer_lists_used_backends() {
        let req = fake_request(8000, 200);
        let mut out = fake_output(vec![fake_hit(1, "hackernews")], req);
        out.backends_succeeded = vec!["hackernews".into(), "wikipedia".into()];
        let s = render(&out);
        assert!(s.contains("Used:"));
        assert!(s.contains("hackernews"));
        assert!(s.contains("wikipedia"));
    }

    #[test]
    fn format_footer_lists_disabled_with_env_hint() {
        let req = fake_request(8000, 200);
        let mut out = fake_output(vec![fake_hit(1, "wikipedia")], req);
        out.catalog.disabled_with_hint = vec![
            ("exa".into(), "EXA_API_KEY".into()),
            ("brave".into(), "BRAVE_API_KEY".into()),
        ];
        let s = render(&out);
        assert!(s.contains("Not enabled:"));
        assert!(s.contains("exa (set EXA_API_KEY)"));
        assert!(s.contains("brave (set BRAVE_API_KEY)"));
    }

    #[test]
    fn format_footer_lists_custom_backends() {
        let req = fake_request(8000, 200);
        let mut out = fake_output(vec![fake_hit(1, "wikipedia")], req);
        out.catalog.custom = vec!["kagi".into(), "internal-confluence".into()];
        let s = render(&out);
        assert!(s.contains("Custom:"));
        assert!(s.contains("kagi"));
        assert!(s.contains("internal-confluence"));
    }

    #[test]
    fn format_emits_hint_when_results_thin_and_stronger_available() {
        let req = fake_request(8000, 200);
        let mut out = fake_output(vec![fake_hit(1, "wikipedia")], req);
        out.backends_succeeded = vec!["wikipedia".into()];
        out.catalog.available = vec!["wikipedia".into(), "hackernews".into(), "github".into()];
        let s = render(&out);
        assert!(s.contains("Hint:"));
        assert!(s.contains("backends="));
    }

    #[test]
    fn format_emits_paid_hint_when_no_free_alternatives() {
        let req = fake_request(8000, 200);
        let mut out = fake_output(vec![fake_hit(1, "wikipedia")], req);
        out.backends_succeeded = vec!["wikipedia".into()];
        out.catalog.available = vec!["wikipedia".into()]; // exhausted
        out.catalog.disabled_with_hint = vec![("exa".into(), "EXA_API_KEY".into())];
        let s = render(&out);
        assert!(s.contains("Hint:"));
        assert!(s.contains("premium"));
    }

    #[test]
    fn format_no_hint_when_results_sufficient() {
        let req = fake_request(8000, 200);
        let hits: Vec<SearchHit> = (1..=5).map(|i| fake_hit(i, "wikipedia")).collect();
        let out = fake_output(hits, req);
        let s = render(&out);
        assert!(
            !s.contains("Hint:"),
            "should not emit hint with 5 hits, got:\n{s}"
        );
    }

    #[test]
    fn format_per_hit_snippet_capped_at_max_snippet_chars() {
        let req = fake_request(8000, 50); // tight snippet cap
        let mut hit = fake_hit(1, "wikipedia");
        hit.snippet = "x".repeat(500);
        let out = fake_output(vec![hit], req);
        let s = render(&out);
        // None of the lines containing the snippet should exceed ~70 chars wide
        for line in s.lines() {
            if line.starts_with("    x") {
                assert!(line.len() <= 80, "line too long: {} chars", line.len());
            }
        }
    }

    #[test]
    fn format_input_error_short_circuit() {
        let req = fake_request(8000, 200);
        let out = DispatcherOutput::input_error("test".into(), req, "query is empty".into());
        let s = render(&out);
        assert!(s.contains("Search input rejected"));
        assert!(s.contains("query is empty"));
    }

    #[test]
    fn format_thousands_works() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(999), "999");
        assert_eq!(format_thousands(1_000), "1,000");
        assert_eq!(format_thousands(2_847), "2,847");
        assert_eq!(format_thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn wrap_lines_basic() {
        let lines = wrap_lines("the quick brown fox jumps over the lazy dog", 20);
        assert!(lines.len() >= 2);
        for l in &lines {
            assert!(l.chars().count() <= 25, "line too wide: {l}");
        }
    }

    #[test]
    fn wrap_lines_empty() {
        assert!(wrap_lines("", 20).is_empty());
    }
}
