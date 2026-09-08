# Self-Learning Fix Proposal — Mathematically Rigorous Drain Framework

> Companion to `SELF_LEARNING_AUDIT.md`. This document defines a **unified artifact value function** and applies it to fix every non-compliant subsystem.

---

## The Unified Artifact Value Function

Every artifact `a` in every self-learning subsystem has a value at time `t`:

```
V(a, t) = Q(a) × R(a, t) × U(a)
```

Where:

| Component | Symbol | Definition | Domain |
|-----------|--------|------------|--------|
| **Quality** | `Q(a)` | How good is this artifact? | `[0, 1]` normalized |
| **Recency** | `R(a, t)` | How fresh is it? | `(0, 1]` exponential decay |
| **Utility** | `U(a)` | How often has it been useful? | `[1, ∞)` logarithmic growth |

The three components are multiplicative because:
- A high-quality but ancient artifact should fade (Q high, R low → V low)
- A fresh but low-quality artifact should not dominate (Q low, R high → V low)
- A frequently-used artifact earns its place (U amplifies V)

**Drain policy:** For every subsystem, artifacts with `V(a, t) < ε` are eligible for garbage collection, where `ε` is the subsystem's gone threshold.

This is already what lambda memory does implicitly. The proposal is to make it explicit, universal, and extend it to every artifact-producing subsystem.

---

## Applying V(a, t) to Each Subsystem

### Lambda Memory (currently: `score = importance × exp(-λt)`)

Lambda memory's decay score is already a special case of `V(a, t)`:

```
V_λ(a, t) = importance × exp(-λ × hours_since_last_access)
```

Mapping:
- `Q(a) = importance / 5.0` (normalized to [0, 1])
- `R(a, t) = exp(-λ × hours_since_access)`
- `U(a) = 1` ← **THIS IS THE GAP**

Utility is fixed at 1. Recall count (`access_count`) is tracked but never feeds back into the score. The system has the gradient signal and discards it.

### Blueprints (currently: success rate for sorting, no drain)

Blueprints track `times_executed`, `times_succeeded`, `times_failed`, `updated` — all the raw data for V(a, t) — but never compute a composite score or use it for lifecycle decisions.

### Cross-Task Learnings (currently: no scoring at all)

Learnings have `timestamp` and `outcome` but no quality score, no recency weighting, no utility tracking. They are the most primitive artifact type in the system.

### Eigen-Tune Pairs (currently: Beta-Binomial quality, no recency or utility)

Eigen-Tune already has the most sophisticated Q(a) via Beta(α, β) posteriors. But it has no R(a, t) or U(a), and no retention policy.

---

## Fix 1: Lambda Memory — Bayesian Importance with Recall Reinforcement

### The Problem

Importance is a point estimate assigned once by the LLM at creation time. It never updates. A memory rated 2.0 that gets recalled 50 times still has importance 2.0. A memory rated 4.0 that is never recalled still has importance 4.0.

### The Model

Replace the scalar `importance: f32` with a Beta distribution `Beta(α, β)` — the same model Eigen-Tune already uses for quality scoring. This is not arbitrary: Beta distributions are the conjugate prior for Bernoulli trials, which is exactly what "was this memory useful?" is.

**Initialization from LLM score:**

The LLM assigns importance `I ∈ [1.0, 5.0]`. Convert to Beta parameters:

```
α₀ = I
β₀ = 6.0 - I
```

This maps:
| LLM importance | α₀ | β₀ | E[importance] | Interpretation |
|----------------|----|----|---------------|----------------|
| 1.0 | 1.0 | 5.0 | 0.167 | Low confidence, probably unimportant |
| 2.0 | 2.0 | 4.0 | 0.333 | Weak signal |
| 3.0 | 3.0 | 3.0 | 0.500 | Uncertain |
| 4.0 | 4.0 | 2.0 | 0.667 | Likely important |
| 5.0 | 5.0 | 1.0 | 0.833 | Strong signal |

The initial `α₀ + β₀ = 6` represents "the LLM's initial assessment is worth about 6 pseudo-observations." This is deliberately low — a few real recall events should be able to override a bad initial estimate.

**Update on recall (positive evidence):**

```
α ← α + 1.0
```

Each recall is one positive Bernoulli observation: "this memory was worth retrieving."

**Update on GC sweep (weak negative evidence):**

During each GC cycle, for entries with `access_count == 0` since last sweep:

```
β ← β + 0.2
```

Weight 0.2 (not 1.0) because "not recalled" is weak evidence — the memory might be valuable but the topic hasn't come up. This is the asymmetry between signal and absence-of-signal.

**Effective importance for decay score:**

```
effective_importance = 5.0 × α / (α + β)
```

This maps back to the [0, 5.0] range that the existing decay formula expects:

```
score = effective_importance × exp(-λ × hours_since_last_access)
```

**No code changes outside lambda_memory.rs and sqlite.rs.** The rest of the system sees `importance` as before. The Bayesian machinery is internal.

### Properties

1. **Converges with evidence.** After 20 recalls, `α ≈ α₀ + 20`, and the LLM's initial estimate is diluted. Ground truth dominates.
2. **Uncertainty decreases.** `Var[Beta(α, β)] = αβ / ((α+β)²(α+β+1))` shrinks as `α + β` grows. High-recall memories have tight confidence intervals.
3. **Asymmetric updating.** Recall is strong evidence (+1.0). Non-recall is weak evidence (+0.2). This matches the information content: choosing to recall something is a deliberate act; not recalling could mean anything.
4. **Bounded.** `effective_importance ∈ (0, 5.0)` always. No overflow, no negative values.
5. **Compatible with existing system.** The decay function `score = importance × exp(-λt)` is unchanged. Only the importance value is now dynamic.

### Schema Change

```sql
ALTER TABLE lambda_memories ADD COLUMN importance_alpha REAL DEFAULT NULL;
ALTER TABLE lambda_memories ADD COLUMN importance_beta REAL DEFAULT NULL;
```

Migration: for existing entries, `α = importance, β = 6.0 - importance`.

---

## Fix 2: Cross-Task Learnings — Complete Rebuild with Value Function

### The Problem (three-fold)

**A. No drain.** Learnings persist forever with no decay, pruning, or supersession.

**B. Rule-based extraction.** Outcome is determined by keyword matching (`"successfully"`, `"failed"`). Lesson text is a mechanical tool-name concatenation. This violates `feedback_no_keyword_matching.md`.

**C. No quality signal.** All learnings are treated as equally valuable. A learning from a 50-tool-call complex deployment is weighted the same as one from a trivial file listing.

### The Model

Apply the full value function with all three components:

**Quality Q(a): Beta-Binomial, same as Eigen-Tune**

```
Initial: Beta(2, 2)     — uninformative prior (E = 0.5)
```

Quality signals:
| Signal | Weight | Direction | Trigger |
|--------|--------|-----------|---------|
| Learning injected AND task succeeded | 1.0 | Positive | Post-task, if this learning was in context |
| Learning injected AND task failed | 1.5 | Negative | Post-task, if this learning was in context |
| Learning injected AND same mistake repeated | 2.0 | Negative | Post-task, if outcome matches the learning's failure pattern |

Update rule (same as Eigen-Tune):
```
if positive: α ← α + weight
if negative: β ← β + weight
```

This gives quality scores that reflect ground truth: did this learning actually help?

**Recency R(a, t): Exponential decay**

```
R(a, t) = exp(-λ_l × days_since_creation)
λ_l = 0.015    (half-life ≈ 46 days)
```

Why 46 days: learnings are broader than memories (they capture strategy patterns, not specific facts). They should decay slower than lambda memory (half-life ~2.89 days at λ=0.01/hour) but still fade within a quarter.

**Utility U(a): Log-reinforcement on application**

```
U(a) = 1.0 + 0.3 × ln(1 + times_applied)
```

Where `times_applied` is incremented each time the learning is injected into a context that leads to a successful task.

| times_applied | U(a) |
|---------------|------|
| 0 | 1.00 |
| 1 | 1.21 |
| 5 | 1.54 |
| 10 | 1.72 |
| 50 | 2.18 |

Logarithmic growth prevents a frequently-used learning from dominating forever.

**Composite score:**

```
V(a, t) = Q(a) × R(a, t) × U(a)

where:
  Q(a)    = α / (α + β)         — Beta posterior mean
  R(a, t) = exp(-0.015 × days)  — exponential decay
  U(a)    = 1 + 0.3 × ln(1 + times_applied)
```

**Drain policy:**

```
if V(a, t) < 0.05:   mark for GC
if V(a, t) < 0.01:   delete on next GC sweep
```

**Supersession:**

When extracting a new learning for `task_type T`:
1. Query existing learnings for `task_type = T`
2. If found with same outcome direction (both success or both failure):
   - If new learning's initial Q > existing V: **replace** (supersede)
   - Else: **skip** (existing is still stronger)
3. If found with opposite outcome (old says failure, new says success): **always supersede** — the problem was fixed.

This prevents contradicting learnings from coexisting, without requiring an LLM call.

### LLM-Powered Extraction

Replace `determine_outcome()` and `generate_lesson()` with an inplace LLM extraction, following the same pattern as lambda memory's `<memory>` block:

Append to the post-task LLM call:

```
If this task produced a reusable insight, emit a <learning> block:
<learning>
task_type: (e.g., "deployment", "data-pipeline", "browser-automation")
outcome: (success | failure | partial)
lesson: (one actionable sentence: what to do or avoid next time)
confidence: (0.0-1.0: how generalizable is this lesson?)
</learning>
Omit the block if the task was trivial or the lesson is obvious.
```

Parse `confidence` to initialize Beta parameters: `α = 2 + 3×confidence, β = 2 + 3×(1-confidence)`.

This replaces keyword matching with LLM judgment — the task type is semantic (not tool-name concatenation), the outcome is assessed by the LLM (not by grepping for "successfully"), and the lesson is a real insight.

### Retrieval Change

Replace `memory.search("learning:", limit=5)` with a scored retrieval:

```rust
// Compute V(a, t) for all learnings, return top-5 by value
let mut scored: Vec<(f64, &TaskLearning)> = learnings
    .iter()
    .map(|l| (compute_learning_value(l, now), l))
    .filter(|(v, _)| *v > 0.05)   // drop gone learnings
    .collect();
scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
scored.truncate(5);
```

This ensures the 5 injected learnings are the 5 most valuable right now — not just the 5 that match a text search.

### Schema

```sql
CREATE TABLE task_learnings (
    id TEXT PRIMARY KEY,
    task_type TEXT NOT NULL,
    approach TEXT NOT NULL,          -- JSON array
    outcome TEXT NOT NULL,           -- "success" | "failure" | "partial"
    lesson TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    quality_alpha REAL DEFAULT 2.0,
    quality_beta REAL DEFAULT 2.0,
    times_applied INTEGER DEFAULT 0,
    last_applied_at INTEGER DEFAULT NULL,
    session_id TEXT
);
CREATE INDEX idx_tl_task_type ON task_learnings(task_type);
CREATE INDEX idx_tl_created ON task_learnings(created_at);
```

This replaces the current approach of storing learnings as generic `MemoryEntry` records with `learning:` prefix, giving learnings their own table with proper typed columns for the value function.

---

## Fix 3: Blueprints — Fitness-Gated Lifecycle with Body Compression

### The Problem

Blueprints implement Create and Refine but not Delete. Bodies grow unbounded through refinements. Low-success blueprints never retire. No staleness detection.

### The Model

**Blueprint fitness function:**

```
F(bp, t) = S(bp)^α × R(bp, t) × U(bp)
```

Where:

**Success rate S(bp): Wilson lower bound (99% CI)**

Raw success rate `p = times_succeeded / times_executed` is noisy for low sample sizes. Use the Wilson score interval lower bound (same statistic Eigen-Tune uses for evaluation gating):

```
S(bp) = wilson_lower(times_succeeded, times_executed, z=2.576)
```

Where:
```
wilson_lower(s, n, z) = (p + z²/2n - z × sqrt(p(1-p)/n + z²/4n²)) / (1 + z²/n)
```

This is conservative: a blueprint with 1 success in 1 execution gets `S ≈ 0.08` (not 1.0). A blueprint with 10 successes in 12 executions gets `S ≈ 0.55`. Confidence grows with sample size.

The exponent `α = 2` makes fitness quadratically sensitive to success rate. A 50% success blueprint has `S² ≈ 0.25`, a 90% success blueprint has `S² ≈ 0.81`. This creates strong selection pressure.

**Recency R(bp, t): Exponential decay from last execution**

```
R(bp, t) = exp(-λ_b × days_since_last_executed)
λ_b = 0.005    (half-life ≈ 139 days)
```

Why 139 days: blueprints represent procedures that may be needed quarterly (e.g., "deploy to production", "onboard new API"). They should decay much slower than memories or learnings, but still fade within ~6 months of disuse.

**Utility U(bp): Execution frequency**

```
U(bp) = 1.0 + 0.5 × ln(1 + times_executed)
```

More aggressive than learnings (0.5 vs 0.3 coefficient) because blueprint execution is a stronger signal — someone loaded and followed the full procedure.

### Drain Policy

**Retirement threshold:**
```
if F(bp, t) < 0.02:   mark as "retired"
if F(bp, t) < 0.005:  delete on next GC sweep
```

**Forced retirement:**
```
if times_executed >= 5 AND wilson_lower(succeeded, executed) < 0.20:
    retire immediately    // proven bad procedure
```

This catches blueprints that have been tried enough times to know they don't work, regardless of recency or utility.

### Body Compression on Refinement

After each refinement, check body size:

```
if token_count > MAX_BLUEPRINT_TOKENS (6000):
    compress(blueprint)
```

Compression strategy (ordered by information loss):

1. **Truncate execution log** to last 3 entries. Execution logs are append-only during refinement. Keeping the last 3 preserves the most recent lessons while bounding growth.

2. **Collapse failure mode details.** Replace verbose failure descriptions with one-line summaries. Keep the failure mode name and recovery action, drop the diagnostic detail.

3. **If still over budget:** trigger a re-authoring LLM call with the instruction: "Compress this blueprint to under 6000 tokens. Preserve all phase structure and decision points. Summarize verbose sections."

This mirrors lambda memory's fidelity degradation (full → summary → essence) but at the document level rather than the entry level.

### Deduplication

On blueprint creation, before storing:

```rust
let existing = fetch_by_task_signature(memory, &new_bp.task_signature).await;
if let Some(old) = existing {
    if cosine_similarity(&old.semantic_tags, &new_bp.semantic_tags) > 0.8 {
        // Merge: keep old ID, absorb new execution stats, re-author body
        merge_blueprints(old, new_bp);
        return;
    }
}
```

Tag similarity is a cheap proxy — `semantic_tags` are short string lists, cosine similarity on their bag-of-words is O(n) with n < 10.

### Schema Change

```sql
ALTER TABLE memory_entries ADD COLUMN last_executed_at INTEGER DEFAULT NULL;
-- For blueprint entries only, tracked via metadata
```

### GC Schedule

Run `blueprint_gc()` on startup and daily via the existing automation crate:

```rust
pub async fn blueprint_gc(memory: &dyn Memory) -> usize {
    let blueprints = fetch_all_blueprints(memory).await;
    let now = Utc::now();
    let mut pruned = 0;
    
    for bp in &blueprints {
        let fitness = compute_fitness(&bp, now);
        if fitness < 0.005 {
            memory.delete(&bp.id).await;
            pruned += 1;
        } else if bp.times_executed >= 5 
            && wilson_lower(bp.times_succeeded, bp.times_executed) < 0.20 {
            memory.delete(&bp.id).await;
            pruned += 1;
        }
    }
    pruned
}
```

---

## Fix 4: Eigen-Tune Retention — Reservoir Sampling with Quality Weighting

### The Problem

Collection is append-only, unbounded. Graduation pipeline is broken (the one mechanism designed to drain artifacts by converting them to weights doesn't work yet). Until graduation is complete, collection is a monotonically growing storage leak.

### The Model: Quality-Weighted Reservoir Sampling

Classical reservoir sampling (Vitter 1985) maintains a fixed-size uniform sample of a stream. We extend it with quality weighting so high-quality pairs are preferentially retained.

**Configuration:**

```toml
[eigentune]
max_pairs_per_tier = 5000    # Maximum pairs retained per complexity tier
```

**Algorithm:** When inserting pair `p` into tier `T`:

```
if count(T) < max_pairs_per_tier:
    insert(p)
else:
    # Find the lowest-quality pair in this tier
    victim = SELECT * FROM eigentune_pairs 
             WHERE complexity = T 
             ORDER BY quality_score ASC LIMIT 1
    
    if quality_score(p) > quality_score(victim):
        delete(victim)
        insert(p)
    else:
        discard(p)    # new pair isn't better than worst retained
```

This is a **min-heap eviction policy**: the reservoir always contains the `max_pairs_per_tier` highest-quality pairs. New evidence must earn its place.

**Why per-tier:** Eigen-Tune trains separate models for Simple, Standard, and Complex tiers. Each tier needs representative training data independently. A global limit would let high-volume Simple pairs crowd out rare Complex pairs.

**Diversity preservation:** Quality alone would bias toward "easy" pairs the model already handles well. Add a diversity term:

```
retention_score(p) = quality_score(p) + 0.2 × category_rarity(p)
```

Where:
```
category_rarity(p) = 1.0 - (count_category(p.domain) / count_tier(p.complexity))
```

Pairs from underrepresented categories (e.g., "creative" in a tier dominated by "coding") get a retention bonus. This maintains the Shannon entropy diversity gate (`J ≥ 0.75`) that the state machine already requires for training transitions.

### Time-Based Retention Floor

Even with reservoir sampling, very old pairs may reflect obsolete model behavior. Add a soft time floor:

```
if pair.created_at < now - retention_days AND pair.quality_score < 0.5:
    eligible for eviction regardless of reservoir position
```

Default `retention_days = 180` (6 months). High-quality pairs (≥ 0.5) are retained regardless of age — they represent genuinely good examples.

---

## Fix 5: Lambda Memory Merging — Semantic Deduplication

### The Problem

Lambda memory has no deduplication. If the user discusses "deployment to staging" across 5 conversations, they get 5 separate memory entries about staging deployment, competing for the same skull space. None are individually wrong, but collectively they waste tokens by restating similar information at varying fidelity levels.

### The Model: Soft Clustering with Merge Trigger

**Detection (at GC time, not per-turn):**

During `lambda_gc()`, group candidate memories by tag overlap:

```
similarity(a, b) = |tags(a) ∩ tags(b)| / |tags(a) ∪ tags(b)|    (Jaccard index)
```

If `similarity(a, b) > 0.6` AND `essence_text` Levenshtein ratio > 0.5:
- Flag `(a, b)` as merge candidates

**Merge policy:**

```
merged.importance_alpha = max(a.importance_alpha, b.importance_alpha)
merged.importance_beta  = min(a.importance_beta, b.importance_beta)
merged.access_count     = a.access_count + b.access_count
merged.created_at       = min(a.created_at, b.created_at)
merged.last_accessed    = max(a.last_accessed, b.last_accessed)
merged.full_text        = a.full_text     // keep the more recent version
merged.summary_text     = a.summary_text  // keep the more recent
merged.essence_text     = a.essence_text  // keep the more recent
merged.tags             = union(a.tags, b.tags)
merged.explicit_save    = a.explicit_save || b.explicit_save
```

The merge takes the optimistic view on importance (highest α, lowest β → highest expected importance) and the union of evidence (combined access counts, earliest creation, latest access, all tags). The text comes from the more recent entry (assumed to be the most up-to-date expression of the same concept).

**Delete the absorbed entry after merge.**

### Why Jaccard + Levenshtein, Not Embeddings

Embedding similarity would be more accurate but requires a model inference call. Memory GC runs periodically on potentially hundreds of entries. Tag-based Jaccard + essence Levenshtein is O(n²) but with n < 500 candidates and string operations only — microseconds, not milliseconds. This is the right tradeoff for a maintenance task.

---

## Mathematical Consistency Check

All five fixes use the same mathematical primitives:

| Primitive | Used By | Purpose |
|-----------|---------|---------|
| **Beta(α, β) posterior** | Learning quality, Eigen-Tune quality | Bayesian updating of quality with natural uncertainty modeling |
| **Additive reinforcement** | Lambda importance | Recall boost (+0.3/recall, -0.1/GC, clamped) — simpler than Beta, zero mapping risk |
| **Exponential decay** | Lambda recency, Learning recency, Blueprint recency | Time-based fading with configurable half-lives |
| **Logarithmic utility** | Learning utility, Blueprint utility | Diminishing returns on repeated use — prevents single artifacts from dominating |
| **Wilson lower bound** | Blueprint success rate | Conservative confidence interval for success rate with low sample sizes |
| **Reservoir sampling** | Eigen-Tune retention | Fixed-size quality-weighted collection with diversity preservation |

These are not five different systems. They are **one framework** (the artifact value function `V = Q × R × U`) instantiated with appropriate parameters for each artifact type.

### Parameter Summary

| Subsystem | Decay λ | Half-life | Q model | U coefficient | GC threshold |
|-----------|---------|-----------|---------|---------------|-------------|
| **Lambda Memory** | 0.01/hr | ~2.89 days | Beta(I, 6-I) | N/A (via recall reheat) | 0.01 |
| **Learnings** | 0.015/day | ~46 days | Beta(2+3c, 2+3(1-c)) | 0.3 | 0.05 |
| **Blueprints** | 0.005/day | ~139 days | Wilson lower bound² | 0.5 | 0.02 |
| **Eigen-Tune** | N/A | 180 days (floor) | Beta(2, 2) + signals | N/A (reservoir) | reservoir eviction |

The half-lives are ordered by artifact persistence: **memories < learnings < blueprints**. This matches the cognitive hierarchy: specific facts fade faster than strategic lessons, which fade faster than proven procedures. Eigen-Tune pairs don't decay — they're evicted by quality competition in the reservoir.

---

## Implementation Order

**Phase 1 — Drain the drainless (highest impact, lowest risk):**

1. **Learning GC + supersession** — add `V(a, t) < 0.05` pruning and same-task-type supersession. This stops the unbounded growth immediately. Does not require schema changes if we compute V at query time from existing fields + timestamp.

2. **Blueprint GC + forced retirement** — add `blueprint_gc()` with Wilson-gated retirement. Purely subtractive — only deletes, never modifies existing blueprints.

3. **Blueprint body cap** — add post-refinement size check with truncation. Small change in the refinement path.

**Phase 2 — Close the feedback loops (medium impact, medium risk):**

4. **Lambda Bayesian importance** — replace scalar importance with Beta(α, β). Schema migration required. All downstream code sees `effective_importance` which maps back to [0, 5.0].

5. **LLM-powered learning extraction** — replace `determine_outcome()` and `generate_lesson()` with inplace LLM `<learning>` block. Changes the extraction path but not the storage or retrieval path.

6. **Eigen-Tune retention** — add reservoir sampling eviction policy. Pure storage-layer change, no runtime impact.

**Phase 3 — Optimize (lower impact, higher complexity):**

7. **Lambda memory merging** — add Jaccard+Levenshtein dedup at GC time.

8. **Learning quality tracking** — add `times_applied` feedback and Beta quality updates. Requires tracking which learnings were in context when a task completed.

9. **Blueprint deduplication** — add task_signature collision detection on creation.
