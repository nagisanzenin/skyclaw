---
id: bp_prowl_compare
name: Multi-Site Comparison
trigger_patterns: ['compare across websites', 'compare online prices']
semantic_tags: ["web", "compare", "price", "shop", "aggregate", "best", "cheapest"]
task_signature: "compare {item} across sites"
success_count: 0
failure_count: 0
---
## Objective
Compare information across multiple websites. This is a natural Hive decomposition candidate.

## Phases

### Phase 1-N: Search each site (independent — parallelizable)
For each target site:
1. Navigate to site
2. Search for the item
3. Extract structured data (price, name, availability, rating)
4. Store results

### Phase N+1: Aggregate (depends: all Phase 1-N)
1. Collect all results
2. Compare by user's criteria (price, rating, availability)
3. Rank results
4. Format as comparison table

## Notes
- Use sequential browsing by default. Parallel browsing requires confirmed isolated browser contexts and an available delegation tool.
- Do not assume separate browser sessions merely because Hive is enabled.
- Report partial results only through an available delivery mechanism; identify missing sites explicitly.
- If a site blocks or fails, skip it and note in results (don't block other sites)

## Failure Recovery
- Site blocked: skip, note in results
- No results on a site: skip, note in results
- All sites fail: report to user with specific errors per site

## Verification
- At least 2 sites successfully compared
- Results are structured as a comparison table
- Rankings match user's stated criteria
