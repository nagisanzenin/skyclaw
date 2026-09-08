---
id: bp_prowl_search
name: Web Search Pattern
trigger_patterns: ['search the web', 'find on a website', 'look up online']
semantic_tags: ["web", "search", "browse", "find", "lookup"]
task_signature: "search for {query} on {site}"
success_count: 0
failure_count: 0
---
## Objective
Search for information on a website using the browser tool.

## Prerequisites
- Browser tool available
- Target URL known or derivable

## Phases

### Phase 1: Navigate (independent)
1. `browser(action="navigate", url="{target_url}")`
2. `browser(action="observe")` — get page structure

### Phase 2: Search (depends: Phase 1)
1. From the accessibility tree, find the search textbox (role=textbox, name contains "search")
2. `browser(action="type", selector="{search_selector}", text="{query}")`
3. Press Enter or click search button
4. `browser(action="observe")` — see results

### Phase 3: Extract (depends: Phase 2)
1. Parse results from accessibility tree (links, text, structured data)
2. Format as a concise summary for the user
3. If results span multiple pages, check if pagination is needed

## Failure Recovery
- If search box not found: try `browser(action="observe", hint="form")` for Tier 2
- If no results: try alternative search terms
- If site blocks: report to user, suggest alternative site

## Verification
- Results contain relevant content matching the query
- At least one actionable result returned to user
