# Frontier harness research

Retrieved September 8, 2026. The six named products are the comparison set, not an exhaustive ranking of the industry. Compare implementation techniques against Tem's general-purpose, messaging-first mission. No same-model head-to-head benchmark was run here.

## What each reference contributes

| Harness | Verified reference behavior | Lesson for Tem | Boundary / reason not to copy wholesale |
|---|---|---|---|
| **Codex** | App Server exposes explicit thread/turn/item events, steering with an expected turn ID, interruption and review; documented sandbox and separate authentication modes | One service contract for lifecycle, cancellation, clients and account state | An OpenAI-centered backend is not a replacement for Tem's provider-neutral runtime. Public API capabilities are not automatically subscription-backend capabilities. [App Server](https://learn.chatgpt.com/docs/app-server), [sandbox](https://learn.chatgpt.com/docs/sandboxing), [auth](https://learn.chatgpt.com/docs/auth). |
| **Claude Code** | Agent loop, structured result messages, hooks, permissions, automatic compaction and deferred MCP schemas; sessions can resume/fork | Explicit output lifecycle, context lifecycle and tool interception; load expensive capability descriptions on demand | Documentation varies by deployment/provider; SDK availability does not imply third-party access to Claude subscription credentials. [Loop](https://code.claude.com/docs/en/agent-sdk/agent-loop), [SDK](https://code.claude.com/docs/en/agent-sdk/overview). |
| **Grok Build** | TUI, headless streaming output and ACP; durable sessions/forks; selectable OS sandbox profiles | Messaging clients can drive a real agent protocol instead of scraping a terminal; persist state consistently across frontends | Its documented sandbox defaults off; child-network enforcement has platform limits. It is not evidence that every frontier harness defaults to strict isolation. [Overview](https://docs.x.ai/build/overview), [sessions](https://docs.x.ai/build/features/sessions), [sandbox](https://docs.x.ai/build/features/sandbox). |
| **ZCode** | Goal Mode continues rounds toward an objective; project context/memory; account-based coding-plan setup | Separate goal lifetime from one turn and make quota/account connection part of onboarding | Public product docs establish UX, not independent proof of recovery correctness or internal architecture. [Goals](https://zcode.z.ai/en/docs/goal), [agent](https://zcode.z.ai/en/docs/agents), [plans](https://zcode.z.ai/en/docs/configuration). |
| **Pi** | Minimal extensible core, steering/follow-up queues, tree-structured sessions, compaction and provider/model selection | Keep a small execution kernel and compose distinctive product behavior around it | Pi intentionally leaves MCP, subagents and permission workflows to extensions. Those choices do not justify removing Tem's existing systems. [Pi](https://github.com/earendil-works/pi/tree/main/packages/coding-agent). |
| **OpenCode** | Separate server/client surface, provider-specific model variants, explicit coding-plan provider selections, configurable compaction and permissions | Model capability data and account routing belong below the agent loop; lifecycle must support several clients | Its model recommendation list itself warns it may be stale. Permission prompts alone are not OS isolation. [Server](https://opencode.ai/docs/server/), [models](https://opencode.ai/docs/models/), [providers](https://opencode.ai/docs/providers/), [config](https://opencode.ai/docs/config/). |

## Source-level spot checks

Pi checkout `b2602be77cb7b0de45dd616407fd210daa48aa75`:

- [`agent-loop.ts`](https://github.com/earendil-works/pi/blob/b2602be77cb7b0de45dd616407fd210daa48aa75/packages/agent/src/agent-loop.ts): separate inner tool/steering work and outer follow-up processing; explicit abort checks in tool execution. This provides a concrete alternative to interpreting “no more tool calls” as global completion.
- [`auth-storage.ts`](https://github.com/earendil-works/pi/blob/b2602be77cb7b0de45dd616407fd210daa48aa75/packages/coding-agent/src/core/auth-storage.ts): file locking includes a deadline, cancellation and compromised-lock handling. Tem's per-object mutex does not coordinate separately loaded stores across processes.
- [`model-registry.ts`](https://github.com/earendil-works/pi/blob/b2602be77cb7b0de45dd616407fd210daa48aa75/packages/coding-agent/src/core/model-registry.ts): delegates refresh through a model runtime. A runtime catalog is an architectural responsibility, not a larger hardcoded match expression.

OpenCode checkout `ecbc6ccac85b3e8087b6445e584318419b9e2b34`:

- [`session/processor.ts`](https://github.com/anomalyco/opencode/blob/ecbc6ccac85b3e8087b6445e584318419b9e2b34/packages/opencode/src/session/processor.ts): identifies context-overflow errors and requests compaction instead of treating all failures identically.
- [`session/compaction.ts`](https://github.com/anomalyco/opencode/blob/ecbc6ccac85b3e8087b6445e584318419b9e2b34/packages/opencode/src/session/compaction.ts): preserves recent material and represents tool states when building compaction input. This is distinct from simply deleting old messages.

These are bounded inspections, not full audits of competitors. Closed/internal behavior was not inferred from marketing. Only Tem was built and probed locally.

## Gap matrix

| Concern | Current Tem evidence | Modernization direction |
|---|---|---|
| Long work | TaskQueue library exists; production wiring absent in the inspected tree | Durable goal/turn/action records and restart reconciliation (F02/P02) |
| Truthful completion | DONE checklist empty; Witness partially wired; no-op maintenance reports success | Evidence-backed outcome contract, preserving partial delivery (F03–F06/P01/P03) |
| Context | Priority budgeting and λ already exist, but final request is not bounded after all additions | Resolve actual model/account limits once; budget after composition; compact with artifact references (F07/P04) |
| Models | Several string-based catalogs, pricing tables and modality guesses | Versioned catalog plus account discovery, capability tri-state and explicit overrides (F10/P06) |
| Subscription access | Codex-specific OAuth; generic Z.ai API route; no unified account/quota contract | First-class connection kinds and billing domains, with support status (F01/F11/F12/P05/P06) |
| Tools | Sequential normal loop plus unused unsafe parallel grouping helper | Typed effects, bounded output, owned subprocess trees, structured events (F08/F09/P07) |
| Memory | Rich long-term concepts; λ query lacks principal scope | Scoped retrieval/recall and provenance before ranking (F13/P08) |
| Self-growth | Minimal growth session and broad staged pipeline differ | Expose actual execution mode and require evidence for each claimed gate (F14/P09) |
| Continuous operation | Perpetuum LLM calls discard usage; maintenance gaps | Shared invocation service, retention and durable outbox (F06/F15/P10) |

## What is deliberately not recommended

Do not equate “frontier” with more subagents, an LLM call before every tool, or a larger system prompt. Do not replace Rust merely to use a popular SDK. Do not move all reasoning into a closed external coding harness and thereby lose Tem's general-purpose tools, personality, memory and initiative. External harness adapters may be optional tools or execution backends, with explicit capability differences.

Use the latest provider features only when the **specific connection** supports them. For example, OpenAI currently documents Astra async tools and steering, but Tem's subscription Responses backend must be independently verified before those fields are emitted. [Current model guidance](https://developers.openai.com/api/docs/guides/latest-model).
