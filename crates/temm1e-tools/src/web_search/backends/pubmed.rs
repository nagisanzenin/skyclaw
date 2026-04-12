//! PubMed E-utilities (esearch + esummary).
//!
//! Endpoint: https://eutils.ncbi.nlm.nih.gov/entrez/eutils/
//! Auth: none up to 3 req/sec without a key.
//! Two-step protocol: esearch returns UIDs, esummary returns metadata.

use super::{fetch_bounded, make_client, truncate_safe, SearchBackend};
use crate::web_search::types::*;
use async_trait::async_trait;
use serde::Deserialize;

pub struct PubmedBackend {
    client: reqwest::Client,
}

impl PubmedBackend {
    pub fn new() -> Self {
        Self {
            client: make_client(),
        }
    }
}

impl Default for PubmedBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct EsearchResponse {
    esearchresult: EsearchResult,
}

#[derive(Debug, Deserialize)]
struct EsearchResult {
    #[serde(default)]
    idlist: Vec<String>,
}

#[async_trait]
impl SearchBackend for PubmedBackend {
    fn id(&self) -> BackendId {
        BackendId::Pubmed
    }
    fn name(&self) -> &str {
        "pubmed"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn default_weight(&self) -> f32 {
        0.85
    }

    async fn search(&self, req: &SearchRequest) -> Result<Vec<SearchHit>, BackendError> {
        let retmax = req.per_backend_raw_cap().min(20);

        // Step 1: esearch → list of UIDs
        let esearch_req = self
            .client
            .get("https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi")
            .query(&[
                ("db", "pubmed"),
                ("term", req.query.as_str()),
                ("retmode", "json"),
                ("retmax", &retmax.to_string()),
            ]);
        let esearch_body = fetch_bounded(&self.client, esearch_req).await?;
        let esearch: EsearchResponse = serde_json::from_str(&esearch_body)
            .map_err(|e| BackendError::Parse(format!("pubmed esearch json: {e}")))?;

        if esearch.esearchresult.idlist.is_empty() {
            return Ok(vec![]);
        }

        // Step 2: esummary → metadata for the UIDs
        let id_str = esearch.esearchresult.idlist.join(",");
        let esum_req = self
            .client
            .get("https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esummary.fcgi")
            .query(&[
                ("db", "pubmed"),
                ("id", id_str.as_str()),
                ("retmode", "json"),
            ]);
        let esum_body = fetch_bounded(&self.client, esum_req).await?;

        // Parse the loose esummary structure: top-level "result" is a map of uid → record
        // plus a "uids" array.
        let value: serde_json::Value = serde_json::from_str(&esum_body)
            .map_err(|e| BackendError::Parse(format!("pubmed esummary json: {e}")))?;
        let result = value
            .get("result")
            .ok_or_else(|| BackendError::Parse("pubmed esummary: missing result".into()))?;
        let uids: Vec<String> = result
            .get("uids")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let total = uids.len();
        let hits: Vec<SearchHit> = uids
            .into_iter()
            .enumerate()
            .filter_map(|(i, uid)| {
                let entry = result.get(&uid)?;
                let title = entry
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if title.is_empty() {
                    return None;
                }
                let journal = entry
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let pubdate = entry
                    .get("pubdate")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let authors: Vec<String> = entry
                    .get("authors")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|a| {
                                a.get("name").and_then(|n| n.as_str()).map(String::from)
                            })
                            .take(5)
                            .collect()
                    })
                    .unwrap_or_default();

                let snippet = truncate_safe(
                    &format!(
                        "{} · {} · {}",
                        authors
                            .iter()
                            .take(3)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", "),
                        journal,
                        pubdate.as_deref().unwrap_or("")
                    ),
                    200,
                );
                let score = if total == 0 {
                    0.0
                } else {
                    1.0 - (i as f32 / total as f32)
                };
                Some(SearchHit {
                    title,
                    url: format!("https://pubmed.ncbi.nlm.nih.gov/{uid}/"),
                    snippet,
                    source: BackendId::Pubmed,
                    source_name: "pubmed".into(),
                    published: pubdate,
                    score,
                    signal: Some(HitSignal::PubmedAuthors { authors, journal }),
                    also_in: vec![],
                })
            })
            .collect();

        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pubmed_backend_construction() {
        let b = PubmedBackend::new();
        assert_eq!(b.name(), "pubmed");
    }

    #[test]
    fn parse_esearch_response() {
        let json = r#"{
            "header": {"type": "esearch"},
            "esearchresult": {
                "count": "100",
                "retmax": "10",
                "idlist": ["12345", "67890", "11111"]
            }
        }"#;
        let parsed: EsearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.esearchresult.idlist.len(), 3);
    }

    #[test]
    fn parse_esearch_empty_idlist() {
        let json = r#"{"esearchresult": {"idlist": []}}"#;
        let parsed: EsearchResponse = serde_json::from_str(json).unwrap();
        assert!(parsed.esearchresult.idlist.is_empty());
    }
}
