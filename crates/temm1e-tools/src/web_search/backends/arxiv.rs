//! arXiv API.
//!
//! Endpoint: https://export.arxiv.org/api/query
//! Returns Atom XML. Parsed with regex (no XML lib needed for our 5 fields).
//! ToS: 3-second spacing between requests, single connection.

use super::{fetch_bounded, make_client, truncate_safe, SearchBackend};
use crate::web_search::types::*;
use async_trait::async_trait;
use std::sync::OnceLock;

pub struct ArxivBackend {
    client: reqwest::Client,
}

impl ArxivBackend {
    pub fn new() -> Self {
        Self {
            client: make_client(),
        }
    }
}

impl Default for ArxivBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SearchBackend for ArxivBackend {
    fn id(&self) -> BackendId {
        BackendId::Arxiv
    }
    fn name(&self) -> &str {
        "arxiv"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn default_weight(&self) -> f32 {
        0.9
    }

    async fn search(&self, req: &SearchRequest) -> Result<Vec<SearchHit>, BackendError> {
        let endpoint = "https://export.arxiv.org/api/query";
        let max_results = req.per_backend_raw_cap().min(50);
        let request = self.client.get(endpoint).query(&[
            ("search_query", format!("all:{}", req.query)),
            ("start", "0".to_string()),
            ("max_results", max_results.to_string()),
            ("sortBy", "relevance".to_string()),
            ("sortOrder", "descending".to_string()),
        ]);

        let body = fetch_bounded(&self.client, request).await?;
        let entries = parse_atom_entries(&body);
        let total = entries.len();

        let hits: Vec<SearchHit> = entries
            .into_iter()
            .enumerate()
            .map(|(i, e)| {
                let snippet = truncate_safe(e.summary.replace('\n', " ").trim(), 250);
                let score = if total == 0 {
                    0.0
                } else {
                    1.0 - (i as f32 / total as f32)
                };
                SearchHit {
                    title: e.title.replace('\n', " ").trim().to_string(),
                    url: e.id,
                    snippet,
                    source: BackendId::Arxiv,
                    source_name: "arxiv".into(),
                    published: e.published,
                    score,
                    signal: Some(HitSignal::ArxivAuthors {
                        authors: e.authors,
                        primary_category: e.primary_category,
                    }),
                    also_in: vec![],
                }
            })
            .collect();

        Ok(hits)
    }
}

#[derive(Debug, Default, Clone)]
struct AtomEntry {
    id: String,
    title: String,
    summary: String,
    published: Option<String>,
    authors: Vec<String>,
    primary_category: Option<String>,
}

/// Extract `<entry>` blocks from Atom XML using a regex per field.
/// Tolerant: missing fields are skipped per-entry, malformed entries are dropped.
fn parse_atom_entries(xml: &str) -> Vec<AtomEntry> {
    static ENTRY_RE: OnceLock<regex::Regex> = OnceLock::new();
    static ID_RE: OnceLock<regex::Regex> = OnceLock::new();
    static TITLE_RE: OnceLock<regex::Regex> = OnceLock::new();
    static SUMMARY_RE: OnceLock<regex::Regex> = OnceLock::new();
    static PUBLISHED_RE: OnceLock<regex::Regex> = OnceLock::new();
    static AUTHOR_RE: OnceLock<regex::Regex> = OnceLock::new();
    static CATEGORY_RE: OnceLock<regex::Regex> = OnceLock::new();

    let entry_re = ENTRY_RE
        .get_or_init(|| regex::Regex::new(r"(?s)<entry>(.*?)</entry>").expect("static regex"));
    let id_re = ID_RE.get_or_init(|| regex::Regex::new(r"(?s)<id>(.*?)</id>").expect("static"));
    let title_re =
        TITLE_RE.get_or_init(|| regex::Regex::new(r"(?s)<title>(.*?)</title>").expect("static"));
    let summary_re = SUMMARY_RE
        .get_or_init(|| regex::Regex::new(r"(?s)<summary>(.*?)</summary>").expect("static"));
    let published_re = PUBLISHED_RE
        .get_or_init(|| regex::Regex::new(r"(?s)<published>(.*?)</published>").expect("static"));
    let author_re = AUTHOR_RE
        .get_or_init(|| regex::Regex::new(r"(?s)<author>\s*<name>(.*?)</name>").expect("static"));
    let category_re = CATEGORY_RE.get_or_init(|| {
        regex::Regex::new(r#"<arxiv:primary_category[^>]*term="([^"]+)""#).expect("static")
    });

    let mut entries = Vec::new();
    for cap in entry_re.captures_iter(xml) {
        let block = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let id = id_re
            .captures(block)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let title = title_re
            .captures(block)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();
        let summary = summary_re
            .captures(block)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();
        let published = published_re
            .captures(block)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string());
        let authors: Vec<String> = author_re
            .captures_iter(block)
            .filter_map(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .collect();
        let primary_category = category_re
            .captures(block)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string());

        entries.push(AtomEntry {
            id,
            title,
            summary,
            published,
            authors,
            primary_category,
        });
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arxiv_backend_construction() {
        let b = ArxivBackend::new();
        assert_eq!(b.name(), "arxiv");
    }

    #[test]
    fn parse_atom_xml_single_entry() {
        let xml = r#"<feed>
<entry>
  <id>http://arxiv.org/abs/2501.12345v1</id>
  <title>A Study of LLM Tool Use</title>
  <summary>This paper investigates how LLMs use tools.</summary>
  <published>2026-01-15T00:00:00Z</published>
  <author><name>Alice Smith</name></author>
  <author><name>Bob Jones</name></author>
  <arxiv:primary_category xmlns:arxiv="http://arxiv.org/schemas/atom" term="cs.AI" scheme="x"/>
</entry>
</feed>"#;
        let entries = parse_atom_entries(xml);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "http://arxiv.org/abs/2501.12345v1");
        assert_eq!(entries[0].title, "A Study of LLM Tool Use");
        assert_eq!(entries[0].authors.len(), 2);
        assert_eq!(entries[0].primary_category.as_deref(), Some("cs.AI"));
        assert_eq!(
            entries[0].published.as_deref(),
            Some("2026-01-15T00:00:00Z")
        );
    }

    #[test]
    fn parse_atom_xml_skips_malformed() {
        let xml = r#"<feed>
<entry>
  <title>No id, will be skipped</title>
</entry>
<entry>
  <id>http://arxiv.org/abs/x</id>
  <title>Has id</title>
  <summary>S</summary>
</entry>
</feed>"#;
        let entries = parse_atom_entries(xml);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Has id");
    }

    #[test]
    fn parse_atom_xml_multiline_summary() {
        let xml = r#"<feed>
<entry>
  <id>http://arxiv.org/abs/y</id>
  <title>Multiline title
on two lines</title>
  <summary>This summary
spans multiple
lines.</summary>
</entry>
</feed>"#;
        let entries = parse_atom_entries(xml);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].title.contains("Multiline title"));
        assert!(entries[0].summary.contains("multiple"));
    }
}
