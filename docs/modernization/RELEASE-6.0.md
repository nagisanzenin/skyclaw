# Temm1e 6.0 — modernization candidate

Status:6.0.0 candidate; not yet merged, tagged or published.

Tem keeps its identity as a persistent companion that uses the model you choose. This release rebuilds important execution boundaries around that idea: provider-native context, scoped conversation storage, model/resource identity, interrupted delivery, evidence and background ownership.

## For everyday use

- A compact terminal transcript with expandable tool activity, optional panels, streaming and retained history.
- Explicit Z.ai Coding Plan setup alongside API-key and existing Codex OAuth connections. Account access and subscription quota remain provider-controlled.
- Current model suggestions and native OpenAI Responses, Anthropic and Gemini replay paths; unknown capabilities and usage are represented honestly.
- Source-backed context compaction with raw history retained and scoped recall, rather than silently discarding the only copy of a conversation.
- Saved final replies and commands to inspect interrupted work. A partial or uncertain send is not blindly repeated.
- Goal/check/evidence records that distinguish an answer from a verified result.
- Shared owning usage across foreground and auxiliary calls, bounds on background work and private browser profiles.
- Verified binary updates and guarded developer builds to reduce accumulated disk use.

## Upgrade behavior

Back up the entire selected profile before upgrading. Existing legacy chats are preserved; `/history-import` previews explicit import into an empty scoped conversation. `/session-new` preserves old evidence; TUI `/clear` clears the display only.

CLI/server `/model <id>` now changes the running instance while keeping state and saved startup defaults. The server's model remains instance-wide. TUI has a separate setup/persistence flow. The next inference checks account access; selecting a model does not make a paid validation call.

Automatic copying of personal Chrome sessions is removed. Existing explicit bounded profile import remains available. Finite USD budgets do not interpret unknown subscription prices as free usage.

## Validation and limits

Source15734b7 passed all8CIjobs, including Rust1.91.1, Linux/Windows, musl/glibc and Docker. Linux3138tests passed(21ignored); Windows3109passed(20ignored), reported separately. Actual local-provider CLI/TUI/persistence/recovery acceptance passed; the final release/version revision must retain applicable checks.

A/B completed60runs: baseline30/30, modernization29/30. The sole discordant case had an ambiguous output contract; one separately frozen clarification pair passed both versions. Original gate remains false; no broad equivalence or speedup is claimed. See [full results](CLOSEOUT-RESULTS.md), including workspace deviations, capped replies and timing/cache limitations.

Tem remains a personal-host agent, not a certified multi-tenant sandbox. Automatic durable pursuit, complete experimental growth/cloud/telemetry implementations, every external messaging/account integration and native Windows TUI behavior are outside these validation guarantees. Model-proposed criteria do not prove coverage of the entire goal; model verdicts do not prove execution. Local usage accounting is not provider account-wide quota or atomic per-goal USD reservation. See the feature ledger, upgrade guide and closeout validation for specific boundaries.

Release/merge method, published artifact hashes and previous-version update smoke: pending.
