# Tem's Lab

Where TEMM1E's cognitive systems are researched, built, and proven.

Every feature in Tem's Mind starts here — as a theory, gets stress-tested against real models and real conversations, and only ships when the data says it works. No vaporware. Every claim has a benchmark behind it.

---

## λ-Memory

**Memory that fades, not disappears.**

Current AI agents delete old messages or summarize them into oblivion. Both permanently lose information. λ-Memory takes a different approach: memories decay through an exponential function (`score = importance × e^(−λt)`) but never truly disappear. The agent sees old memories at progressively lower fidelity — full text → summary → essence → hash — and can recall any memory by hash to restore full detail.

Three features no other system has ([competitive analysis](LAMBDA_MEMORY_RESEARCH.md)):
- **Hash-based recall** from compressed memory — the agent sees the shape of what it forgot and can pull it back
- **Dynamic skull budgeting** — same algorithm adapts from 16K to 2M context windows without overflow
- **Pre-computed fidelity layers** — full/summary/essence written once at creation, selected at read time by decay score

### Benchmarked: 1,200+ API calls across GPT-5.2 and Gemini Flash

| Test | λ-Memory | Echo Memory | Naive Summary |
|------|----------|-------------|---------------|
| [Single-session recall](LAMBDA_BENCH_GPT52_REPORT.md) (GPT-5.2) | 81.0% | **86.0%** | 65.0% |
| [Multi-session recall](LAMBDA_BENCH_MULTISESSION_REPORT.md) (GPT-5.2, 5 sessions) | **95.0%** | 58.8% | 23.8% |
| [Cross-model](LAMBDA_BENCH_REPORT.md) (Gemini Flash) | 67.0% | 76.0% | 48.5% |

Echo Memory (keyword search over recent context) wins when everything fits in one session. The moment context resets between sessions — which is how real users interact with agents — λ-Memory achieves **95% recall where Echo drops to 59%** and naive summarization collapses to 24%.

Naive rolling summarization is the worst strategy in every test. It destroys information from early turns as new summaries overwrite old ones. [Full data](LAMBDA_EFFECTIVENESS_REPORT.md).

### Documents

| | |
|---|---|
| [Research Paper](LAMBDA_RESEARCH_PAPER.md) | The complete story — problem, architecture, 1,200+ API calls of benchmarks, per-question scoring, cross-model analysis, conclusions |
| [Design Doc](LAMBDA_MEMORY.md) | Decay function, skull model, dynamic budget, adaptive pressure thresholds, full dry run with exact numbers |
| [Competitive Research](LAMBDA_MEMORY_RESEARCH.md) | Landscape review of Letta/MemGPT, Mem0, Zep, FadeMem, Kore, LangMem, CrewAI — what exists, what's novel, honest gaps |
| [Implementation Guide](LAMBDA_MEMORY_IMPLEMENTATION.md) | Every file, function, SQL statement — the bone for implementation |
| [Multi-Session Benchmark](LAMBDA_BENCH_MULTISESSION_REPORT.md) | 5 sessions over 7 simulated days, context reset between each, 20-question recall exam |
| [GPT-5.2 Single-Session](LAMBDA_BENCH_GPT52_REPORT.md) | 100 turns, 3 strategies, per-question breakdown, cross-model comparison |
| [Final Report](LAMBDA_FINAL_REPORT.md) | Consolidated analysis across all runs with recommendations |

**Status:** Implemented in Rust. 1,509 tests pass. Ships with `/memory` command for hot-switching between λ-Memory and Echo Memory mid-conversation.

---

## Tem's Mind v2.0

**The agentic loop that knows what kind of task you're asking before it starts working.**

v1 treats every message the same — full system prompt, full tool pipeline, same iteration limits whether you say "thanks" or ask to debug a codebase. v2 classifies each message into a complexity tier (Trivial → Simple → Standard → Complex) **before** calling the LLM, using zero-cost rule-based heuristics.

### Benchmarked: 30 turns across Gemini Flash and GPT-5.2

| Metric | v1 | v2 | Delta |
|--------|----|----|-------|
| [Cost per successful turn](TEMS_MIND_V2_BENCHMARK.md) (Gemini Flash) | $0.00159 | $0.00145 | **-9.3%** |
| [Compound task savings](TEMS_MIND_V2_BENCHMARK_TOOLS.md) (GPT-5.2) | baseline | -12.2% | **-12.2%** |
| Tool call failures | 2 turns | 1 turn | **-50%** |
| Classification accuracy | N/A | 100% (30/30) | Zero LLM overhead |

The savings come from compound multi-step tasks where fewer API rounds mean less cumulative context. Trivial turns (greetings, acknowledgments) skip the tool pipeline entirely.

### Documents

| | |
|---|---|
| [Architecture](TEMS_MIND_ARCHITECTURE.md) | Full component map — runtime, context manager, executor, self-correction, circuit breaker, 15 subsystems |
| [Implementation Plan](TEMS_MIND_V2_PLAN.md) | Token optimization architecture, resilience design, blueprint system, prompt stratification |
| [Benchmark: Gemini Flash](TEMS_MIND_V2_BENCHMARK.md) | 10-turn A/B — 9.3% cost reduction, 50% fewer provider errors |
| [Benchmark: GPT-5.2](TEMS_MIND_V2_BENCHMARK_TOOLS.md) | 20-turn tool-heavy A/B — 4.8% total, 12.2% on compound, 100% classification accuracy |
| [Experiment Insights](TEMS_MIND_V2_EXPERIMENT_INSIGHTS.md) | Where v2 saves, where it doesn't, what the 4.8% number undersells |
| [Release Notes](TEMS_MIND_V2_RELEASE.md) | User-facing changelog |

**Status:** Shipped. Running in production.

---

## Eigen-Tune (Self-Tuning)

**An AI agent that fine-tunes itself. Zero added LLM cost.**

Every LLM call is a training example. Eigen-Tune captures them, scores quality from user behavior, curates datasets, trains local models, and graduates them through statistically rigorous gates — all with zero user intervention beyond `/eigentune on` and zero added LLM cost.

The bet: open-source models will only get better. Our job is to have the best domain-specific training data ready when they do. **The data is the moat. The model is a commodity.**

### Architecture

A 7-stage closed-loop pipeline with per-tier state machines governed by statistical tests. **Default: $0 added LLM cost.** Optional Teacher Mode for users who want to pay for stronger guarantees.

| Stage | Method | Cost |
|-------|--------|------|
| Collect | Fire-and-forget hook on every provider call | $0 |
| Score | Beta-Binomial model from user behavior signals | $0 |
| Curate | Shannon entropy + Thompson sampling | $0 |
| Train | QLoRA via Unsloth/MLX → GGUF → Ollama | $0 (local compute) |
| Evaluate | Embedding similarity (local) + Wilson score (99% CI) | $0 |
| Shadow | User behavior → SPRT (Wald, 1945) | $0 |
| Monitor | User behavior → CUSUM (Page, 1954) | $0 |
| *Teacher (opt-in)* | *LLM-as-judge with position debiasing* | *LLM API cost* |

Each complexity tier (Simple, Standard, Complex) graduates independently. Simple first, complex last. Cloud is always the fallback. The user IS the judge — their behavior (continue, retry, reject) drives graduation decisions.

### Benchmarked: Real fine-tuning on Apple M2, 16GB RAM

| Metric | Result |
|--------|--------|
| Data collected | 10 conversations, 3 tiers, 8 domains |
| Training loss | 2.450 → 1.242 (49% reduction) |
| Peak memory | 0.509 GB training, 0.303 GB inference |
| Speed | ~28 it/sec training, ~200 tok/sec inference |
| Base model 72°F | "150°C" (wrong) |
| **Fine-tuned 72°F** | **"21.2°C" (close to 22.2°C)** |
| Statistical tests | 128 tests, all passing |

The base model made a fundamental arithmetic error. Ten training examples fixed it. This is knowledge distillation working at consumer scale. [Research paper →](eigen/RESEARCH_PAPER.md) · [Full pipeline log →](eigen/PIPELINE_PROOF_LOG.txt)

### Documents

| | |
|---|---|
| [Research Paper](eigen/RESEARCH_PAPER.md) | Full paper: architecture, math, real M2 results, economics, limitations |
| [Design Doc](eigen/DESIGN.md) | Formal state machine, mathematical formulas (SPRT, CUSUM, Wilson, Beta-Binomial, Shannon entropy), zero-cost evaluation architecture, data model, risk assessment |
| [Implementation Plan](eigen/IMPLEMENTATION.md) | Phase-by-phase build guide — every struct, function, file, and test |
| [Technical Reference](eigen/TECHNICAL_REFERENCE.md) | Ollama API endpoints, training scripts (Unsloth + MLX), embedding judge, two-tier behavior detection, ChatML format, codebase hook locations |
| [Pipeline Proof Log](eigen/PIPELINE_PROOF_LOG.txt) | Unedited output: data → training → inference on Apple M2, 2026-03-18 |
| [Setup Guide](eigen/SETUP.md) | User-facing: install Ollama + MLX/Unsloth, enable, choose model, troubleshooting |

**Status:** Implemented and proven. 136 tests, real fine-tuning on M2.

---

## Tem Gaze (Desktop Vision Control)

**An AI agent that sees and controls any computer. No local models. No DOM. Just vision.**

Current web agents (Claude Computer Use, OpenAI Operator, AskUI) control desktops via screenshots. TEMM1E's Tem Prowl controls browsers via DOM. Tem Gaze bridges the gap — extending TEMM1E to full desktop control (Ubuntu + macOS) using the user's already-configured VLM with zero new model dependencies.

Three techniques, all model-agnostic:
- **Zoom-Refine** — coarse localization then progressive zoom into the target region (+29pp on ScreenSpot-Pro, up to +61% relative improvement)
- **Set-of-Mark overlay** — numbered labels on detected elements, converting coordinate regression to classification (49x improvement when applied to GPT-4o)
- **Verify-Retry** — post-action verification catches misclicks (OSAgent achieves superhuman 76.26% on OSWorld through persistent self-checking)

Research surveyed 20+ frameworks, 8 benchmarks, 15+ papers. Key finding: the industry converged on pure vision for desktop (Claude CU, UI-TARS, Agent S2, AskUI, Fara-7B). Desktop accessibility covers only 33-65% of apps. Vision covers 100%.

### Documents

| | |
|---|---|
| [Research Paper](gaze/RESEARCH_PAPER.md) | Full landscape: 20+ frameworks, benchmark analysis (OSWorld, ScreenSpot-Pro), accessibility coverage data, multi-model routing rejection, technique evaluation |
| [Design Doc](gaze/DESIGN.md) | Formal spec: 7 system axioms, ScreenController trait, grounding pipeline with coordinate transform math, SoM overlay, config schema, risk assessment |
| [Implementation Plan](gaze/IMPLEMENTATION.md) | Phase-by-phase: Prowl V2 (browser), Tem Gaze (desktop), validation benchmarks — every file, function, and test |
| [Architecture Overview](../docs/design/TEM_GAZE_ARCHITECTURE.md) | Developer-facing: crate map, pipeline diagram, cost profile, platform support matrix |
| [Experiment Report](gaze/EXPERIMENT_REPORT.md) | 7 live tests (4 browser + 3 desktop): SoM overlay on 650 elements, zoom_region 2x, desktop click opened Finder, full Spotlight→TextEdit→type proof |
| [Phase 1 Report](gaze/PHASE1_REPORT.md) | Prowl V2 implementation details: 596 lines, 23 tests, compilation gates |

### Benchmarked: 7 live tests on gemini-3-flash-preview, $0.069 total

| Test | Type | Result |
|------|------|--------|
| SoM Observe (14 elements) | Browser | PASS — Tier 3, overlays match tree |
| Zoom Region (2x) | Browser | PASS — 27 KB crop, vision injected |
| Dense Page (650 elements) | Browser | PASS — zero crash, 178 KB screenshot |
| Multi-step E2E | Browser | PASS — self-corrected after 94px miss |
| Desktop Screenshot | Desktop | PASS — identified Arc, iTerm2, VS Code |
| Desktop Click (Finder) | Desktop | PASS — Finder opened, verified |
| Full Computer Use Proof | Desktop | PASS — Spotlight→TextEdit→typed message |

**Status:** Implemented. Proven. 45 new tests, 19 crates. Browser + desktop computer use verified live.

---

## Tem Prowl (Web-Native Browsing)

**Headless browsing for messaging-first agents.**

See [TEM_PROWL_PAPER.md](TEM_PROWL_PAPER.md) and [prowl/](prowl/) for the full research, benchmarks, and implementation history. Prowl V2 (browser vision upgrade) shipped as part of the Tem Gaze initiative — SoM overlay generalization, zoom_region action, blueprint bypass.

**Status:** V1 shipped. V2 shipped (Gaze Phase 1).

---

## Tem Cambium (Gap-Driven Self-Grow)

**Tem writes its own code.**

Cambium is the layer where Tem extends its own runtime. It observes a gap between what a user needed and what the system could deliver, writes new Rust code to close the gap, verifies the code through a 13-stage deterministic harness, and deploys via blue-green binary swap supervised by an immutable watchdog. Named after the biological cambium — the thin layer of growth tissue under tree bark where new wood is added each year. The heartwood (immutable kernel: vault, traits, security, the pipeline itself) never changes; the cambium adds rings at the edge.

The architectural choice that makes Cambium timeproof: a **pluggable LLM-backed code generator** is separated from a **fixed mechanical verification harness**. The model's code quality improves with each generation; the safety guarantees stay constant because they are mechanical, not probabilistic.

### Verified end-to-end with real LLMs

**Skill-layer proof** (markdown skill files):

| Tier | Model | Result | Time | Files written |
|------|-------|--------|------|---------------|
| Cheap | [gemini-3-flash-preview](cambium/CAMBIUM_RESEARCH_PAPER.md) | OK | 5.8s | 1 skill |
| Medium | [claude-sonnet-4-6](cambium/CAMBIUM_RESEARCH_PAPER.md) | OK | 12.8s | 2 skills |

Both analysed 7 synthetic Docker/k8s activity notes and produced loadable YAML-frontmatter skill files registered in `SkillRegistry`. **Total cost: < $0.02**.

**Code-level proof** (real Rust that compiles, lints clean, passes tests):

| Tier | Model | Result | Time | cargo check | cargo clippy | cargo test |
|------|-------|--------|------|---|---|---|
| Cheap | gemini-3-flash-preview | OK | 40.8s | OK | OK | OK (6 tests passed) |

Gemini Flash autonomously generated a complete `format_bytes(bytes: u64) -> String` function with 5 unit tests covering the zero / small-bytes / kilobytes / megabytes / large cases, preserved the existing `marker()` function exactly, included a doc comment, used a loop-based unit progression. The deterministic verification harness ran all three gates against the generated code in an isolated tempdir crate. **Cost: ~$0.001**.

This is the proof that matters: skill authoring is trivial markdown, but code-level growth required the LLM to write valid Rust that survives the compiler, the linter (warnings-as-errors), and the test runner. It does. See `crates/temm1e-cambium/tests/real_code_grow_test.rs` for the test, run with `TEMM1E_CAMBIUM_REAL_CODE_TEST=1 cargo test -p temm1e-cambium --test real_code_grow_test -- --nocapture`.

### Five phases shipped

| Phase | Component | Status |
|-------|-----------|--------|
| 0 | Theory + codebase self-model ([THEORY.md](../docs/lab/cambium/THEORY.md), [PROTECTED_ZONES.md](../docs/lab/cambium/PROTECTED_ZONES.md)) | DONE |
| 1 | `CambiumConfig` + 8 core types (`GrowthTrigger`, `GrowthKind`, `TrustLevel`, `PipelineStage`, `StageResult`, `GrowthOutcome`, `GrowthSession`, `TrustState`) | DONE |
| 2 | `temm1e-cambium` library: `zone_checker`, `trust`, `budget`, `history`, `sandbox`, `pipeline`, `deploy` (107 unit tests) | DONE |
| 3 | Skill-layer growth via `SelfWorkKind::CambiumSkills` + `grow_skills()` handler with 24h rate limit, path traversal sanitization, JSON-from-prose extractor | DONE |
| 4 | Code pipeline with dedicated git-clone sandbox isolation at `~/.temm1e/cambium/sandbox/` + `cambium-reviewer` and `cambium-auditor` TemDOS cores | DONE |
| 5 | Blue-green binary swap with `try_wait` crash detection, macOS code-signing safe inode replacement, zombie-aware `is_process_alive` + `temm1e-watchdog` immutable supervisor | DONE |

### Documents

| | |
|---|---|
| [Research Paper](cambium/CAMBIUM_RESEARCH_PAPER.md) | The complete story — abstract, 10 first principles, architecture, timeproof design, empirical validation, prior art, conclusion, references including botanical citation |
| [Theory](../docs/lab/cambium/THEORY.md) | The 10 principles, the trust hierarchy, the 13-stage pipeline, the heartwood/cambium/bark/rings metaphor |
| [Implementation Plan](../docs/lab/cambium/IMPLEMENTATION_PLAN.md) | Phase-by-phase plan with success metrics and confidence gates |
| [Protected Zones](../docs/lab/cambium/PROTECTED_ZONES.md) | SHA-256 catalogue of every Level 0 file in the immutable kernel |
| [Architecture](../docs/lab/cambium/ARCHITECTURE.md) | Crate map, dependency graph, message flow, trust level per crate |
| [Coding Standards](../docs/lab/cambium/CODING_STANDARDS.md) | Rules self-grown code must follow |

**Status:** All 5 phases shipped. Real-LLM verified. Enabled by default (toggle with `/cambium on` / `/cambium off`). 2275 tests passing, 0 failures.

---

## Research Philosophy

Every system in Tem's Lab follows the same process:

1. **Theory** — what's the problem, what's the hypothesis
2. **Landscape** — what exists, what's been tried, what failed
3. **Design** — architecture with exact math, not hand-waving
4. **Implement** — in Rust, in the actual codebase, not a prototype
5. **Benchmark** — against alternatives, on real models, with scoring rubrics
6. **Ship or kill** — if the data says it doesn't work, it doesn't ship

No feature ships without a benchmark. No benchmark ships without a scoring rubric. No claim is made without data behind it.

---

*TEMM1E's Lab — where Tem's mind is built*
