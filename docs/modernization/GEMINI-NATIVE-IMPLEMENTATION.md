# Gemini native streaming and function continuation

## Confirmed defects

The former `stream()` waited for `generateContent`, returned one synthetic text chunk, and discarded tools and usage. Tool results in user messages were dropped; results in tool messages used an internal call ID as the function name. Thinking text could become public response text, textual signatures were lost, missing token counters became measured zero, and output usage omitted thinking tokens. HTTP errors could include API keys in query URLs and reflected response bodies. Model listing stopped at the first page.

## Implementation contract

`gemini.rs` owns native generateContent JSON requests and streamGenerateContent SSE requests, with API keys in the header and redirects disabled. Model IDs cannot escape their resource path. HTTP errors report status and sanitized transport errors, not raw provider bodies. JSON reads stop at 32 MiB. Explicit configured base URLs now reach this adapter, permitting real isolated HTTP fixtures.

`gemini_native.rs` retains original supported parts as opaque ProviderState scoped to endpoint and model. Thought text stays out of visible responses. Same-route replay checks normalized function IDs/names/arguments, so edited or removed calls cannot be silently resurrected. Public text appendices are added after original parts; replacing native text requires explicit migration. Foreign native state is not replayed. Legacy function-call signatures remain supported where no native state exists.

Request conversion maps each internal tool ID to its original function name and optional provider ID. Both User and Tool role results use that map and consume it once. Orphan/duplicate results and unresolved calls fail before a request. Results retain success/error distinction. Native function parts, including signatures, survive serialized-history restore. Unsupported output media/server tools fail explicitly; no fake media/tool support is claimed.

`gemini_stream.rs` accepts one candidate, streams public text immediately, retains native parts within 16 MiB/4,096 parts and 128 function calls, and rejects malformed/blocked/duplicate/truncated terminal data. Function calls and native history are emitted only after a successful end-of-stream with a valid terminal candidate. MAX_TOKENS text may be returned with that reason; truncated tool output is not executable. Late usage-only events are retained. The shared SSE transport adds an EOF finalization hook and discards queued data after errors; existing adapters retain their previous terminal rules. Its 2 MiB event and 64 MiB wire limits still apply.

Usage includes thinking tokens. When reported, total minus prompt yields output; separately reported candidate/thought counts are checked against it. Missing totals remain explicitly unknown; invalid/overflowing/decreasing counts fail. Cache reads are measured only if reported, and cache writes remain unknown. No API-equivalent USD price or subscription quota is invented.

Model listing follows at most 32 pages, detects repeated/bounded continuation tokens and returns usable model IDs without the `models/` prefix. This is discovery, not a promise of entitlement or adapter support.

## Evidence and limits

Two actual local HTTP fixtures prove incremental Unicode text before the server releases the remainder, native signature/function continuation after serialization, correct result name/provider ID, thinking-inclusive usage, and no tool exposure after an incomplete stream. Unit checks cover billing inconsistency/overflow, native route/model separation, changed-tool rejection and terminal gating. No paid Gemini account was used. The initial compile failure from comparing a non-PartialEq Role enum is preserved in `implementation-gemini-native-initial-tests.log`; fixed with explicit variant matching.

Final provider tests passed 83 unit tests, two Gemini HTTP fixtures, one Anthropic native HTTP fixture, one retry fixture and five existing stream transport fixtures (`implementation-gemini-native-post-review-tests.log`). Workspace/all-feature/all-target lint passed before a final function-field validation refinement (`implementation-gemini-native-workspace-clippy.log`), reclaiming 1.6 GiB. Final scoped provider/all-target lint passed (`implementation-gemini-native-post-review-clippy.log`), reclaiming 602.5 MiB. The refinement accepts omitted zero-argument objects and rejects unsupported partial-argument protocols; its regression test passes.

This implements the existing generateContent protocol. Google's newer Interactions API, hosted tools, output media, explicit server cache lifecycle, model-specific thinking controls, live Gemini acceptance and account quota discovery remain separate work. The static menu/catalog does not certify those capabilities. Replay is endpoint/model-scoped, not principal-scoped; broader identity policy remains unresolved. Legacy schemas still pass through the compatibility filter for unsupported fields.

Primary references verified September 9, 2026: [generateContent and streaming API](https://ai.google.dev/api/generate-content) defines parts, function result identity and usage totals; [current thinking guide](https://ai.google.dev/gemini-api/docs/thinking) describes the newer Interactions protocol, which is deliberately not conflated with generateContent. The old thought-signatures URL now redirects readers to that guide.
