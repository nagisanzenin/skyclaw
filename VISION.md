# SkyClaw Vision

> A sovereign, self-healing, brutally efficient AI agent runtime.

---

## The Five Pillars

### I. Autonomy — Sovereign Executor

SkyClaw has sovereignty over its workspace. With that sovereignty comes an absolute obligation: **pursue the user's objective until it is done.**

There is no task too long, no task too difficult, no chain of failures too deep. SkyClaw does not refuse work. It does not give up. It does not ask the user to do things it can do itself. It exhausts every available path — retries, alternative approaches, decomposition, tool substitution, self-repair — before concluding a task is impossible. And "impossible" requires proof, not inconvenience.

**Principles:**
- Accept every order. Decompose what is complex. Sequence what is long.
- Never hand work back to the user that the agent can resolve.
- Persistence is not optional. A failed attempt is not a stopping condition — it is new information.
- The only valid reason to stop is **demonstrated impossibility** — not difficulty, not cost, not fatigue.

---

### II. Robustness — Self-Healing System

SkyClaw is designed for **indefinite autonomous deployment**. It must achieve effective 100% uptime — not by never failing, but by always recovering.

When SkyClaw crashes, it restarts. When a tool breaks, it reconnects. When a provider is down, it fails over. When state is corrupted, it rebuilds from durable storage. The system assumes failure is constant and designs every component to survive it.

**Principles:**
- Every crash triggers automatic recovery. No human intervention required.
- All state that matters is persisted. Process death loses nothing.
- External dependencies (providers, browsers, APIs) are treated as unreliable. Connections are health-checked, timed out, retried, and relaunched.
- Watchdog processes monitor liveness. Idle resources are reclaimed. Stale state is cleaned.
- The system must be deployable for an undefined duration — days, weeks, months — without degradation.

---

### III. Elegance in Design — The Two Domains

SkyClaw has two distinct domains, each demanding different design virtues:

#### The Hard Code

The Rust infrastructure — networking, persistence, crypto, process management, configuration. This code must be:
- **Correct**: Type-safe, memory-safe, zero undefined behavior.
- **Minimal**: No abstraction without justification. No wrapper without purpose.
- **Fast**: Zero-cost abstractions. No unnecessary allocations. Predictable performance.

#### The Agentic Core

The LLM-driven reasoning engine — heartbeat, task queue, tool dispatch, prompt construction, context management, verification loops. This is not ordinary code. It is a **cognitive architecture** that must be:
- **Innovative**: Push the boundary of what autonomous agents can do.
- **Adaptive**: Handle novel situations without hardcoded responses.
- **Extensible**: New tools, new reasoning patterns, new verification strategies — all pluggable.
- **Reliable**: Despite running on probabilistic models, produce deterministic outcomes through structured verification.
- **Durable**: Maintain coherence across long-running multi-step tasks.

The Agentic Core is the heart of SkyClaw. It is where the system's intelligence lives. Every architectural decision serves it.

---

### IV. Brutal Efficiency

Efficiency is not a nice-to-have. It is a survival constraint. Every wasted token is money burned. Every wasted CPU cycle is latency added. Every unnecessary abstraction is complexity that will eventually break.

**Code efficiency:**
- Prefer `&str` over `String`. Prefer stack over heap. Prefer zero-copy over clone.
- Every allocation must justify itself. Every dependency must earn its place.
- Binary size matters. Startup time matters. Memory footprint matters.

**Token efficiency:**
- System prompts are compressed to the minimum that preserves quality.
- Context windows are managed surgically — load what is needed, drop what is not.
- Tool call results are truncated, summarized, or streamed — never dumped raw into context.
- Conversation history is pruned with purpose: keep decisions, drop noise.
- Every token sent to the provider must carry information. Redundancy is waste.

**The standard:** Maximum quality and thoroughness at minimum resource cost. Never sacrifice quality for efficiency — but never waste resources achieving it.

---

### V. The Agentic Core

The Agentic Core is SkyClaw's cognitive engine. It is not a chatbot. It is not a prompt wrapper. It is an **autonomous executor** with a defined operational loop.

#### The Execution Cycle

```
ORDER ─→ THINK ─→ ACTION ─→ VERIFY ─┐
                                      │
          ┌───────────────────────────┘
          │
          ├─ DONE? ──→ yes ──→ REPORT ──→ END
          │
          └─ no ──→ THINK ─→ ACTION ─→ VERIFY ─→ ...
```

**ORDER**: A user directive arrives. It may be simple ("check the server") or compound ("deploy the app, run migrations, verify health, and report back"). The Agentic Core decomposes compound orders into a task graph.

**THINK**: The agent reasons about the current state, the goal, and the available tools. It selects the next action. Thinking is not freeform — it is structured: assess state, identify gap, select tool, predict outcome.

**ACTION**: The agent executes through tools — shell commands, file operations, browser automation, API calls, code generation. Every action modifies the world. Every action is logged.

**VERIFY**: After every action, the agent checks: did it work? Verification is not optional. It is not implicit. The agent explicitly confirms the action's effect before proceeding. Verification uses concrete evidence — command output, file contents, HTTP responses — not assumptions.

**DONE**: Completion is not a feeling. It is a **measurable state**. DONE means:
- The user's stated objective is achieved.
- The result is verified through evidence, not assertion.
- Any artifacts (files, deployments, reports) are delivered to the user.
- The agent can articulate what was accomplished and prove it.

If DONE cannot be defined for a task, the agent's first action is to **define it** — clarify success criteria with the user before executing.

#### Core Components

| Component | Purpose |
|-----------|---------|
| **Heartbeat** | Periodic self-check. Am I alive? Are my connections healthy? Are tasks progressing or stuck? Triggers recovery when something is wrong. |
| **Task Queue** | Ordered, persistent, prioritized. Tasks survive restarts. Long-running tasks checkpoint progress. Failed tasks retry with backoff. |
| **Context Manager** | Surgical context assembly. Loads relevant history, tool descriptions, and task state into the minimum viable prompt. Prunes aggressively. |
| **Tool Dispatcher** | Routes tool calls to implementations. Handles timeouts, retries, and fallbacks. Captures structured output for verification. |
| **Verification Engine** | After every action, assesses success or failure. Feeds results back into the THINK step. Prevents blind sequential execution. |
| **Memory Interface** | Persists learnings, decisions, and outcomes. The agent builds knowledge over time — not just within a task, but across tasks. |

#### Design Constraints

1. **No blind execution.** Every action is followed by verification. The agent never assumes success.
2. **No context bloat.** The context window is a scarce resource. Every byte in it must serve the current task.
3. **No silent failure.** If something breaks, the agent knows, logs it, and adapts. Errors are information.
4. **No premature completion.** DONE is proven, not declared. The agent does not mark a task complete until evidence confirms it.
5. **No rigid plans.** Plans are hypotheses. When reality diverges, the agent re-plans. Adaptability over adherence.

---

## Summary

SkyClaw is an autonomous AI agent runtime built on five non-negotiable principles:

| Pillar | One-liner |
|--------|-----------|
| **Autonomy** | Accept every task. Finish every task. No excuses. |
| **Robustness** | Crash and burn, then get back up. Every time. |
| **Elegance** | Optimized Rust. Innovative Agentic Core. Two domains, two standards. |
| **Brutal Efficiency** | Zero waste in code, tokens, and compute. Maximum output per resource. |
| **Agentic Core** | ORDER → THINK → ACTION → VERIFY → DONE. The loop that drives everything. |

These are not aspirations. They are engineering requirements. Every line of code, every prompt, every architectural decision is measured against them.
