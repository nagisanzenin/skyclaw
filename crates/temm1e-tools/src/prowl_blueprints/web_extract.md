---
id: bp_prowl_extract
name: Web Data Extraction
trigger_patterns: ['extract web data', 'read a web table']
semantic_tags: ["web", "extract", "table", "data", "scrape", "read", "get"]
task_signature: "extract {data} from {url}"
success_count: 0
failure_count: 0
---
## Objective
Extract structured data from a web page.

## Phases

### Phase 1: Navigate and observe (independent)
1. `browser(action="navigate", url="{target_url}")`
2. `browser(action="observe")` — get page structure
3. Analyze tree: does it contain the target data?

### Phase 2: Extract (depends: Phase 1)
1. If data is in accessibility tree (text, links, list items) → extract directly
2. If data is in a table → `browser(action="observe", hint="table")` for Tier 2 Markdown
3. If data requires scrolling → scroll and re-observe
4. If data is behind a click (expand, "show more") → click and re-observe

### Phase 3: Structure and deliver (depends: Phase 2)
1. Format extracted data as structured output (Markdown table, JSON, or bullet list)
2. If multi-page, paginate and extract from each page
3. Return formatted results to user

## Failure Recovery
- Data not visible: check if login required (→ web_login blueprint)
- Dynamic content not loading: wait longer, retry observe
- Anti-bot block: report to user

## Verification
- Extracted data matches user's request
- Data is structured and readable
- No credential or sensitive data in output (scrubber applied)
