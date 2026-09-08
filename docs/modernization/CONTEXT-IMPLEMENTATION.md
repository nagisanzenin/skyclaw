# Source-grounded context handoffs

Implemented on the modernization branch, September 8, 2026. This replaces lossy history selection in the journal-enabled production runtime. It is part of P04/P14, not a claim that every context/cache feature is complete.

## Data and authority

`crates/temm1e-agent/src/compaction.rs` defines versioned handoffs containing an exclusive raw-message boundary, SHA-256 source hash, ordinal/content-derived source IDs, original messages, and structured model summaries. The summary has work state, decisions, pending work and uncertainties. Each item needs an exact quotation from a named source. Validation checks schema, bounded sizes, source identity/order/content, and quoted text. This verifies provenance, **not the semantic truth of the model's interpretation**.

Every earlier User and System message remains verbatim with its original role and content blocks, including images. Newer user corrections retain their later position. Tool-result IDs, error flags and content hashes are assembled mechanically. An error-free tool result is never promoted to goal completion. Generated summary text is explicitly labeled as interpretation.

Original source data is retained as immutable, content-addressed SQLite message rows in the existing private execution journal. Generations reference source IDs instead of copying the full raw prefix each time. Earlier development profiles with inline source documents remain readable, and loaded content is revalidated against its source hash. Scope includes channel/user/chat/workspace and session ID. `context_heads` points to an immutable `context_handoffs` row; a compare-and-swap rejects a stale writer. Failed generation or failed final-fit validation leaves the previous durable head and raw history intact.

## Runtime sequence

1. Load the scoped head. Reuse it only if its entire raw prefix still matches the caller's history. Never treat an unrelated or truncated history as the same generation.
2. Consider compaction when the retained history plus handoff/pins exceeds half the configured context allowance, or the configured legacy turn-count threshold. This is an early trigger, not a tokenizer guarantee. Keep the latest two user turns raw and reject a cut through outstanding native tool calls.
3. Build bounded requests from original source groups. Atomic tool groups remain together. Every source appears once across a generation's requests, with its original ID. A single oversized group pauses explicitly; it is not truncated. Snapshots are capped at 32 MiB and a generation at 128 summarizer requests.
4. Use the selected provider/model, no tools, and at most 4,096 output tokens per summarizer request (also bounded by model limits). Check each request's estimated final budget. Account for every returned response, including one corrective retry on invalid structured output/quotes. The turn's deadline/cancellation and spend checks still apply. A second invalid response fails without overwriting context.
5. Validate the merged handoff. Require a reduction after counting the verbatim pins; otherwise pause with the original history available. Rebuild future generations from raw messages rather than recursively summarizing generated prose.
6. Assemble the ordinary request with all remaining history, pinned messages, summary, optional memory and current runtime injections. Sort visible tools deterministically and reject duplicate names. The journal-enabled path never performs a second hidden history-dropping pass. The final compiler includes tool schemas, both system segments, framing and output reserve, with a 10% estimation margin.
7. Only after that final fit check succeeds, commit the new generation with compare-and-swap, then issue the foreground request. A new input outside the frozen prefix remains outside the summary.
8. Offer `context_recall` when a matching handoff exists. It can page original source text by ID, with Unicode-character offsets and an 8,192-character maximum page. The capability holds only that handoff and checks session/chat/workspace. It cannot query arbitrary files or another journal scope. Normal role/runtime tool filters apply both to visibility and dispatch.

The TUI displays an explicit Compacting phase. Full structured context panels and a manual compact command remain separate work.

## Evidence

- Runtime fixture passes three generations, preserving the earliest constraints and raw messages. It checks database reload, scoped lookup, stale-write rejection and invalid replacement after a valid generation.
- A final-request overflow fixture ensures a candidate is not committed before its full assembled request fits.
- Recall fixture checks original Unicode text, pagination and wrong-scope rejection.
- Dispatch fixture proves a model-invented hidden tool cannot bypass the runtime filter.
- Live Z.ai GLM-5.3-Flash fixture v3 passed: corrected color `amber`, publishing prohibited, and oldest-only `archive_label=maple-17`; raw history unchanged, generation 1, three provider calls including a validation retry. Total recorded turn usage was 16,426 input / 8,184 output tokens and 135.5 seconds. This is acceptance evidence, not a speed improvement or account bill.
- Live fixture v1 retained the requested facts but failed exact raw-JSON formatting because the model added fences. The failure is preserved. Fixture v2 accidentally failed to seed its expected archive label and is classified as an invalid test fixture, not a retention result; its original output and usage are preserved. V3 asserts that the fact is seeded before making a request.

## Remaining work and limits

Counting remains explicitly estimated; exact provider tokenization and native compaction adapters are not implemented here. Re-summarizing raw source batches can be expensive, and no efficiency gain is claimed. Signed thinking history, older binary/model migrations, manual compaction controls, compaction-specific usage presentation and a full long-context product comparison remain open.

A database reload test is not a complete session-recovery claim: older CLI/server history loaders can retain only a tail. Such a tail cannot match a prior full source prefix, and the handoff is deliberately not applied to it. Canonical session loading/reset epochs must be finished before claiming full restart continuity. Embedded hosts without durable execution still use the legacy history builder; production constructors use the journal-enabled path. These limitations must remain visible in release acceptance.
