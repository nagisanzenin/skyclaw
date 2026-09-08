# Anthropic native conversation state

## Problem and behavior

The previous Messages adapter discarded thinking/signature/redacted blocks, and the stream decoder treated them as ignorable. That loses the provider-native state required to continue a tool workflow. New Anthropic generations also have different sampling/thinking parameters; sending the old generic temperature to them is invalid.

The adapter now stores bounded, validated native output in opaque `ProviderState`, alongside normalized visible text and executable tool calls. Both JSON and SSE use the same normalization. Thinking is never emitted to the UI observer. Stream tool calls become dispatchable only after a valid `message_stop`. Unknown native content types fail explicitly rather than silently authorizing an incomplete response. The adapter rejects duplicate/invalid tool IDs, non-object arguments, unfinished tool stops and overflowing usage totals. Response error bodies are neither logged nor exposed.

Replay requires the same normalized endpoint and model, and unchanged normalized tool identities/arguments. Harness corrections to visible text are appended distinctly instead of rewriting original native blocks. Other provider serializers omit opaque state. Raw output is capped at 16 MiB/1,024 blocks; JSON transport is capped at 32 MiB. Existing stream transport and collector limits still apply.

## Fable prefix binding

Official documentation says Fable 5.1 signatures bind to the system prompt, tool definitions and preceding messages, with enforcement for new accounts from August 31, 2026. Compaction or changing an injected reminder can therefore invalidate otherwise intact reasoning.

Each response records SHA-256 over the actual system/tools/messages wire prefix. Replay computes the corresponding prefix in order. For Fable 5.1, missing or mismatching fingerprints remove thinking and redacted-thinking blocks from the outgoing provider view; later prefix hashes also invalidate affected suffixes. Visible text and tool blocks remain. The durable original is preserved. This implements the documented client-side invalid-block removal option without requiring beta headers or changing creator memory semantics.

Exact Fable 5.1, Opus 5 and Sonnet 5 requests select adaptive thinking and omit temperature. Older models retain existing behavior. System-role history constraints now survive Messages serialization as system text rather than being silently discarded.

## Validation and limits

Provider unit tests cover signature deltas (including split Unicode), redacted reasoning, tool validation, terminal dispatch, same-prefix replay, volatile-system mismatch, cross-endpoint/model isolation, preserved original history, and cache arithmetic overflow. HTTP streaming fixtures continue to exercise transport truncation and late usage. This is deterministic protocol validation, not a live Anthropic subscription/account acceptance claim. No Anthropic account was charged.

The fingerprint is a local provenance check, not proof of semantic accuracy or an Anthropic signature verifier. Endpoint/model binding does not establish organization/account portability. Unsupported server-tool blocks fail explicitly. Full Gemini native streaming, user-facing reasoning controls and live provider acceptance remain separate work.

## Primary sources (retrieved September 8–9, 2026)

- [Thinking tool workflows](https://platform.claude.com/docs/en/build-with-claude/thinking-tool-workflows): preserve complete native reasoning blocks with tool continuations.
- [Thinking troubleshooting](https://platform.claude.com/docs/en/build-with-claude/thinking-troubleshooting): Fable prefix binding and invalid-block handling options.
- [Sonnet 5 changes](https://platform.claude.com/docs/en/models/sonnet-5/whats-new-sonnet-5): adaptive thinking and sampling restrictions.
- [Extended thinking](https://platform.claude.com/docs/en/build-with-claude/extended-thinking): generation-specific modes and deprecations.
