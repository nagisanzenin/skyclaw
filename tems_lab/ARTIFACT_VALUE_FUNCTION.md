# The Unified Artifact Value Function

> How TEMM1E's self-learning subsystems score, decay, and drain their artifacts.

---

## The Problem

Traditional machine learning learns by adjusting numeric weights. Agentic AI learns by producing **artifacts** — memories, lessons, blueprints, skills, training pairs. Unlike weights, artifacts grow. They consume context. They go stale. Left unmanaged, they overwhelm the skull and the agent gets worse by learning.

Every artifact-producing subsystem needs a drain. The question is: **what mathematical framework unifies all drains?**

## The Function

Every artifact `a` in every self-learning subsystem has a value at time `t`:

```
V(a, t) = Q(a) × R(a, t) × U(a)
```

| Component | Symbol | Definition | Domain |
|-----------|--------|------------|--------|
| **Quality** | `Q(a)` | How good is this artifact? | `[0, 1]` or `[0.1, 5.0]` |
| **Recency** | `R(a, t)` | How fresh is it? | `(0, 1]` via exponential decay |
| **Utility** | `U(a)` | How often has it been useful? | `[1, ∞)` via logarithmic growth |

The three components are multiplicative because:
- A high-quality but ancient artifact should fade (Q high, R low -> V low)
- A fresh but low-quality artifact should not dominate (Q low, R high -> V low)
- A frequently-used artifact earns its place (U amplifies V)

## Instantiation Per Subsystem

### Lambda Memory

```
V_lambda = effective_importance(a) * exp(-lambda * hours_since_last_access)

effective_importance = (importance + recall_boost).clamp(0.1, 5.0)
```

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| `importance` | [1.0, 5.0] | LLM-assigned at creation, immutable |
| `recall_boost` | [0.0, 2.0] | +0.3 per recall, -0.1 per GC sweep |
| `lambda` (decay rate) | 0.01/hour | Half-life ~29 days |
| Gone threshold | 0.01 | Below this, memory is invisible |

Lambda memory's "Q" is `effective_importance / 5.0`, "R" is the exponential decay, and "U" is implicit via recall_boost (recall-reinforced entries decay slower because their effective_importance is higher).

The drain is exponential decay + garbage collection. The feedback signal is `lambda_touch()` on recall.

### Cross-Task Learnings

```
V_learning = Q(a) * R(a, t) * U(a)

Q(a)    = alpha / (alpha + beta)           -- Beta posterior mean
R(a, t) = exp(-0.015 * days_since_created) -- half-life ~46 days
U(a)    = 1 + 0.3 * ln(1 + times_applied) -- log-reinforcement
```

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| Initial `alpha, beta` | 2 + 3*confidence, 2 + 3*(1-confidence) | From LLM confidence in `<learning>` block |
| Decay lambda | 0.015/day | Learnings are broader than memories, decay slower |
| Gone threshold | 0.05 | Below this, not injected into context |
| Delete threshold | 0.01 | Below this, GC deletes |

The drain is exponential decay + GC. Supersession prevents contradictions: same task_type + opposite outcome always replaces. Retrieval is scored by V — top 5 by value, not by timestamp.

### Blueprints

```
F_blueprint = S^2 * R(bp, t) * U(bp)

S = wilson_lower(succeeded, executed, 99% CI)   -- conservative success rate
R = exp(-0.005 * days_since_last_executed)       -- half-life ~139 days
U = 1.0 + 0.5 * ln(1 + times_executed)          -- execution frequency
```

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| Wilson CI | 99% (z=2.576) | Conservative: 1/1 success -> S~0.08, not 1.0 |
| S exponent | 2 (quadratic) | Strong selection pressure: 50% -> 0.25, 90% -> 0.81 |
| Decay lambda | 0.005/day | Procedures persist longer: half-life ~139 days |
| Delete threshold | 0.005 | Very conservative: only truly dead blueprints |
| Forced retirement | 5+ executions AND S < 0.20 | Proven bad, regardless of recency |

The drain is startup GC. The feedback signal is execution outcome (success/failure updates Wilson bounds).

### Eigen-Tune Pairs

```
retention_score = quality_score  (Beta(alpha, beta) posterior mean)
```

Eigen-Tune doesn't use time-decay because training data doesn't lose value with age — a high-quality example from 6 months ago is as useful as one from today. Instead, the drain is **reservoir eviction**: when a tier reaches `max_pairs_per_tier` (default 5000), new pairs must beat the lowest-quality existing pair to be retained.

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| Initial quality | Beta(2, 2), E=0.5 | Uninformative prior |
| Retention floor | 180 days + quality < 0.5 | Very old AND low quality -> prune |
| Max per tier | 5000 | Bounded storage per complexity level |
| Eviction policy | Min-heap by quality_score | Worst pair evicted when full |

The drain is quality-competitive eviction. The feedback signal is user behavior (continued, retried, tool success/failure) updating Beta posteriors.

### Tem Anima — User Profile Learning

```
profile_dimension = old_value * (1 - weight) + eval_value * weight

weight         = evidence_strength * merge_rate
evidence_strength = confidence * min(1.0, turns_analyzed / 10.0)
merge_rate     = 0.4 / (1 + 0.1 * eval_count)
```

Tem Anima profiles users across conversations — communication style, emotional state, personality traits, trust level. Unlike the other subsystems, Anima doesn't produce discrete artifacts. It maintains a **fixed-size profile** that converges toward the user's true preferences through weighted Bayesian updating.

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| Confidence decay | 5% per unobserved eval (x0.95) | Dimensions not reinforced fade toward uncertainty |
| Zero-out threshold | confidence < 0.1 | Below this, dimension is removed from injection |
| Confidence tiers | 0.3 cosmetic, 0.5 tonal, 0.7 behavioral, 0.8 relational, 0.9 confrontational | Graduated trust — higher confidence unlocks deeper adaptation |
| Buffer caps | 30 facts, 50 observations, 100 eval logs | Hard limits prevent unbounded storage |
| Merge rate decay | 0.4 / (1 + 0.1 * eval_count) | Early evaluations have high influence, later ones converge |
| Adaptive N | 5-30 turns between evals, log growth with stability | Frequent re-eval when behavior shifts, rare when stable |
| Reset threshold | delta > 0.15 | Behavioral shift resets evaluation frequency to N=5 |

The drain is confidence decay — unobserved dimensions lose 5% confidence per evaluation cycle and zero out below 0.1. The feedback signal is user behavior observed every N turns via background LLM evaluation.

Anima's V(a,t) is implicit: `confidence` IS the quality dimension, `merge_rate` decay IS the recency dimension, and the `adaptive N` scheduling IS the utility signal (frequent evaluation for volatile profiles, rare for stable ones). The fixed-size profile means Anima never grows — it converges.

## Parameter Design Rationale

### Half-Lives Are Ordered by Artifact Persistence

```
memories (29d) < learnings (46d) < blueprints (139d)
```

This matches the cognitive hierarchy:
- **Memories** are specific facts about specific conversations. They go stale fast.
- **Learnings** are strategic lessons extracted from task patterns. They're broader, persist longer.
- **Blueprints** are full procedures for complex tasks. They may be needed quarterly (deployments, migrations). They persist longest.

Eigen-Tune pairs don't decay because training data is cumulative, not episodic.

### Why Multiplicative, Not Additive

If V = Q + R + U, then a high-quality ancient artifact (Q=0.9, R=0.01, U=1.0) scores V=1.91, which is competitive with a fresh low-quality one (Q=0.1, R=1.0, U=1.0) at V=2.1. The ancient artifact wastes context with stale content.

With V = Q * R * U, the ancient artifact scores 0.009 — effectively invisible. Multiplication ensures that ANY dimension collapsing to zero eliminates the artifact. This is the correct behavior: a stale artifact is worthless regardless of how high-quality it was.

### Why Logarithmic Utility

`U = 1 + c * ln(1 + times_used)` has diminishing returns:

| times_used | U (c=0.3) | U (c=0.5) |
|------------|-----------|-----------|
| 0 | 1.00 | 1.00 |
| 1 | 1.21 | 1.35 |
| 5 | 1.54 | 1.90 |
| 10 | 1.72 | 2.20 |
| 50 | 2.18 | 2.96 |
| 100 | 2.38 | 3.31 |

Without the log, a heavily-used artifact would dominate forever. With the log, 100 uses gives only 2.4x the score of 0 uses — enough to survive longer, not enough to be immortal.

## Convergence with the Skull

The skull (context window) is the hard constraint. The learning loops are the pressure. The value function is the prioritization mechanism that decides what earns its place in the skull.

When the skull is under pressure:
1. Lambda memory thresholds rise adaptively (pressure = 1 - budget/max_budget)
2. Low-V learnings are filtered out (gone threshold = 0.05)
3. Blueprints degrade: full body -> outline -> catalog
4. Everything below its gone threshold becomes invisible

The value function doesn't just score — it creates a **priority queue for cognitive resources**. The skull has room for N tokens of injected artifacts. The value function ensures those N tokens are the N most valuable artifacts the system has ever learned.

## All 13 Layers (v4.6.0)

### Recall Reinforcement

Each time a lambda memory is recalled via the `lambda_recall` tool, `recall_boost` increases by +0.3 (capped at 2.0). During GC, entries with existing boost but no recent access lose 0.1. The effective importance fed into the decay function becomes `(importance + recall_boost).clamp(0.1, 5.0)`.

A memory recalled 7 times reaches maximum effective importance (5.0) regardless of its initial LLM-assigned score. A memory with a single recall (boost 0.3) loses its boost after 3 GC cycles without access. This is the feedback signal that lambda memory's original design was missing — importance updates from actual usage.

**Drain:** Boost decays via GC penalty (-0.1/sweep). Bounded at [0.0, 2.0].

### Memory Dedup

At startup, `dedup_candidates()` scans all lambda memories and identifies near-duplicates using a dual threshold: Jaccard tag similarity > 0.6 AND essence word overlap > 0.5. Both must match. Matched entries are merged — the more recently accessed entry absorbs the other's evidence (higher importance, higher boost, combined access counts, union of tags). The absorbed entry is deleted.

**Explicit saves are NEVER merged** — if the user said "remember this," the hash survives intact.

**Drain:** Merging IS the drain. Duplicate entries are eliminated, freeing skull space.

### Core Stats

Every `invoke_core` call loads the core's historical `CoreStats` from memory before execution, records success/failure + rounds + cost after execution, and persists the updated stats. The output includes lifetime performance: `[TemDOS:research | 5 rounds | $0.04 | lifetime: 75% success (N=8)]`.

The main agent sees this and naturally avoids unreliable cores. A core with 0% success rate (like `research` on Gemini in our CLI test) becomes self-documenting — the data speaks.

**Drain:** Fixed-size records, updated in place (UPSERT). No growth.

### Tool Reliability

After every tool execution, the runtime records `(tool_name, task_type, success/failure)` to the `tool_reliability` table. Task type is the classifier's `category:difficulty` label. On context building, a compact 30-day rolling summary is injected (~50-100 tokens):

```
Tool reliability (last 30 days):
  shell: Order:Standard 68% (N=19)
  browser: Order:Standard 100% (N=5)
  file_read: Order:Standard 50% (N=2)
```

The LLM reads this and routes tool choices accordingly. Records with fewer than 3 observations are filtered out (insufficient sample size).

**Drain:** 30-day rolling window on retrieval. Old data naturally excluded.

### Classification Feedback

After every task completes, the runtime persists the classification prediction alongside the actual outcome: `(category, difficulty, rounds, tools_used, cost_usd, success, prompt_tier, had_whisper)`. Aggregated into empirical priors:

```
Order/Simple: avg 2.0 rounds, 1.0 tools, $0.003 (N=12)
Order/Standard: avg 9.5 rounds, 8.5 tools, $0.024 (N=45)
```

These priors can be injected into the classifier prompt so it learns what each category actually costs. If "Simple" tasks consistently use 8+ rounds, the classifier recalibrates.

**Drain:** Retention cap at 500 rows. Oldest deleted beyond limit.

### Skill Tracking

When a skill is invoked (action = "invoke" in the `use_skill` tool), the `skill_usage` table records the skill name with an upsert: `invocations += 1, last_invoked_at = now`. This enables adoption tracking — which skills are actually used vs which sit idle.

Future use: boost high-adoption skills in matching, auto-archive skills unused for 90+ days.

**Drain:** Fixed-size records per skill (UPSERT). No growth beyond the skill count.

### Prompt Tier Tracking

The `prompt_tier` field (Minimal, Basic, Standard, Full) is recorded alongside every classification outcome. Aggregating by tier reveals cost-effectiveness:

```
Basic: avg 2.0 rounds, $0.003 (N=10)
Standard: avg 5.8 rounds, $0.015 (N=38)
Full: avg 8.2 rounds, $0.022 (N=7)
```

If Basic tier achieves similar success rates at 5x lower cost, the system should use it more. If Minimal tier causes 3x more rounds (higher total cost), the "savings" from a shorter prompt are illusory.

**Drain:** Shares the classification_outcomes retention cap (500 rows).

### Consciousness Efficacy

The `had_whisper` boolean in classification outcomes tracks whether the consciousness observer injected a whisper into the agent's system prompt for that turn. Aggregating:

```
WITH whisper: avg 4.2 rounds, $0.07, 92% success (N=120)
WITHOUT: avg 5.8 rounds, $0.11, 85% success (N=280)
```

This is **continuous production A/B testing**. If consciousness consistently reduces rounds and cost, it's earning its ~$0.001-0.005 per-turn LLM call. If the numbers are flat, consciousness is overhead.

**Drain:** Shares the classification_outcomes retention cap (500 rows).

---

## Implementation Status (v4.6.0)

| # | Layer | V(a,t) Scored | Drain Active | Feedback Loop |
|---|-------|:------------:|:------------:|:-------------:|
| 1 | Lambda Memory | decay_score() | GC + dedup | recall_boost on touch |
| 2 | Cross-Task Learnings | learning_value() | GC threshold + supersession | Beta priors from LLM confidence |
| 3 | Blueprints | compute_fitness() | Startup GC + forced retirement | Wilson lower bound success rate |
| 4 | Eigen-Tune | quality_score | Reservoir eviction (5K/tier) | User behavior signals |
| 5 | Tem Anima | confidence (implicit) | 5%/eval confidence decay + buffer caps | Weighted merge from LLM evaluation |
| 6 | Recall Reinforcement | effective_importance() | GC penalty (-0.1/sweep) | +0.3/recall |
| 7 | Memory Dedup | Jaccard + essence similarity | Merge absorbs duplicates | Startup pass |
| 8 | Core Stats | success_rate() | Fixed-size (UPSERT) | Record per invocation |
| 9 | Tool Reliability | success/failure counts | 30-day rolling window | Record per tool execution |
| 10 | Classification Feedback | empirical priors | 500-row retention cap | Record per task completion |
| 11 | Skill Tracking | invocation count | Fixed-size (UPSERT) | Record per skill invoke |
| 12 | Prompt Tier Tracking | tier + outcome correlation | 500-row retention cap | Record per task completion |
| 13 | Consciousness Efficacy | whisper vs no-whisper comparison | 500-row retention cap | Boolean flag per turn |
