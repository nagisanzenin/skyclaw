# Sources and research provenance

Research date: **2026-09-08**. Sources are primary product documentation or public source. Product documentation establishes supported contracts, not measured superiority. Prices, model IDs, subscription eligibility, quota and API details must be rechecked when implementing. A public client implementation does not grant permission to use a provider subscription in another product.

## Pinned repositories

- Temm1e baseline: [da503c09ca5c0f41c308c99e42d4736aa3611f8a](https://github.com/temm1e-labs/temm1e/tree/da503c09ca5c0f41c308c99e42d4736aa3611f8a), v5.8.1. All source line references use this snapshot.
- Pi: [b2602be77cb7b0de45dd616407fd210daa48aa75](https://github.com/earendil-works/pi/tree/b2602be77cb7b0de45dd616407fd210daa48aa75). Bounded source inspection of agent loop, auth storage and model registry.
- OpenCode: [ecbc6ccac85b3e8087b6445e584318419b9e2b34](https://github.com/anomalyco/opencode/tree/ecbc6ccac85b3e8087b6445e584318419b9e2b34). Bounded source inspection of session processor/compaction and provider/permission interfaces.

Grok Build is the official xAI harness at docs.x.ai/build; ZCode is the Z.ai application at zcode.z.ai. These resolve the creator's shorthand names. Closed-source internals were not inferred. Competitors were not independently benchmarked against Tem.

## Primary documentation consulted and cited

- [Gemini caching](https://ai.google.dev/gemini-api/docs/caching) — retrieved/reviewed 2026-09-08.
- [Loop](https://code.claude.com/docs/en/agent-sdk/agent-loop) — retrieved/reviewed 2026-09-08.
- [SDK](https://code.claude.com/docs/en/agent-sdk/overview) — retrieved/reviewed 2026-09-08.
- [Authentication](https://code.claude.com/docs/en/authentication) — retrieved/reviewed 2026-09-08.
- [OpenAI compaction](https://developers.openai.com/api/docs/guides/compaction) — retrieved/reviewed 2026-09-08.
- [Current model guidance](https://developers.openai.com/api/docs/guides/latest-model) — retrieved/reviewed 2026-09-08.
- [OpenAI caching](https://developers.openai.com/api/docs/guides/prompt-caching) — retrieved/reviewed 2026-09-08.
- [history documentation](https://docs.slack.dev/reference/methods/conversations.history/) — retrieved/reviewed 2026-09-08.
- [Slack upload documentation](https://docs.slack.dev/reference/methods/files.upload/) — retrieved/reviewed 2026-09-08.
- [sandbox](https://docs.x.ai/build/features/sandbox) — retrieved/reviewed 2026-09-08.
- [sessions](https://docs.x.ai/build/features/sessions) — retrieved/reviewed 2026-09-08.
- [Overview](https://docs.x.ai/build/overview) — retrieved/reviewed 2026-09-08.
- [FAQ](https://docs.x.ai/grok/faq) — retrieved/reviewed 2026-09-08.
- [Z.ai tool integration](https://docs.z.ai/devpack/tool/others) — retrieved/reviewed 2026-09-08.
- [`session/compaction.ts`](https://github.com/anomalyco/opencode/blob/ecbc6ccac85b3e8087b6445e584318419b9e2b34/packages/opencode/src/session/compaction.ts) — retrieved/reviewed 2026-09-08.
- [`session/processor.ts`](https://github.com/anomalyco/opencode/blob/ecbc6ccac85b3e8087b6445e584318419b9e2b34/packages/opencode/src/session/processor.ts) — retrieved/reviewed 2026-09-08.
- [`agent-loop.ts`](https://github.com/earendil-works/pi/blob/b2602be77cb7b0de45dd616407fd210daa48aa75/packages/agent/src/agent-loop.ts) — retrieved/reviewed 2026-09-08.
- [`auth-storage.ts`](https://github.com/earendil-works/pi/blob/b2602be77cb7b0de45dd616407fd210daa48aa75/packages/coding-agent/src/core/auth-storage.ts) — retrieved/reviewed 2026-09-08.
- [`model-registry.ts`](https://github.com/earendil-works/pi/blob/b2602be77cb7b0de45dd616407fd210daa48aa75/packages/coding-agent/src/core/model-registry.ts) — retrieved/reviewed 2026-09-08.
- [Source](https://github.com/earendil-works/pi/tree/b2602be77cb7b0de45dd616407fd210daa48aa75/packages/coding-agent) — retrieved/reviewed 2026-09-08.
- [Pi](https://github.com/earendil-works/pi/tree/main/packages/coding-agent) — retrieved/reviewed 2026-09-08.
- [App Server](https://learn.chatgpt.com/docs/app-server) — retrieved/reviewed 2026-09-08.
- [auth](https://learn.chatgpt.com/docs/auth) — retrieved/reviewed 2026-09-08.
- [sandbox](https://learn.chatgpt.com/docs/sandboxing) — retrieved/reviewed 2026-09-08.
- [MCP tools specification](https://modelcontextprotocol.io/specification/2025-11-25/server/tools) — retrieved/reviewed 2026-09-08.
- [config](https://opencode.ai/docs/config/) — retrieved/reviewed 2026-09-08.
- [models](https://opencode.ai/docs/models/) — retrieved/reviewed 2026-09-08.
- [providers](https://opencode.ai/docs/providers/) — retrieved/reviewed 2026-09-08.
- [Server](https://opencode.ai/docs/server/) — retrieved/reviewed 2026-09-08.
- [Anthropic caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching) — retrieved/reviewed 2026-09-08.
- [agent](https://zcode.z.ai/en/docs/agents) — retrieved/reviewed 2026-09-08.
- [plans](https://zcode.z.ai/en/docs/configuration) — retrieved/reviewed 2026-09-08.
- [Goals](https://zcode.z.ai/en/docs/goal) — retrieved/reviewed 2026-09-08.

## Tem vision/source traceability

Start with VISION.md, docs/TEMM1E_VISION.md, FEATURES.md and README.md, then the feature documents identified in FEATURE-COVERAGE. Older architectural promises and newer deliberate behavior are recorded as conflicts rather than silently choosing one. Research-paper assertions and benchmark prose are treated as hypotheses until supported by implementation/measurements.

[evidence/document-inventory.tsv](evidence/document-inventory.tsv) records Markdown headings and locations; [evidence/source-inventory.tsv](evidence/source-inventory.tsv) records every Rust production-source path and size. The inventories are completeness aids, not claims of exhaustive line-by-line manual review. Evidence labels and tests are defined in README/VALIDATION.

### Streaming checkpoint sources (checked September 8, 2026)

- OpenAI, [Chat Completions streaming events](https://developers.openai.com/api/reference/resources/chat/subresources/completions/streaming-events): indexed tool deltas, completion markers and trailing usage snapshots.
- Z.ai, [Chat Completions API](https://docs.z.ai/api-reference/llm/chat-completion): native streaming and tool calls. No documented `stream_options` contract; compatible endpoints are not assumed to support OpenAI-specific options.
- WHATWG, [Server-sent events](https://html.spec.whatwg.org/dev/server-sent-events.html): UTF-8, BOM, line endings and event framing. Tem's API decoder deliberately rejects invalid UTF-8/truncated API responses; it does not implement browser reconnect/replacement behavior.
- Anthropic, [Streaming messages](https://platform.claude.com/docs/en/build-with-claude/streaming): cumulative usage and indexed content-block lifecycle, used for the pending native streaming adapter.
