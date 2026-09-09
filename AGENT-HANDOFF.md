# Temm1e modernization — agent handoff

Updated September 9, 2026. The creator is asleep and explicitly authorized autonomous work through completion. Read this file first. Every implementation commit must update this handoff, `docs/modernization/IMPLEMENTATION-STATUS.md` and its focused implementation document, then push. Do not stop merely because a checkpoint passes.

## Authority and release conditions

- Repository: `temm1e-labs/temm1e`; branch `codex/modernize-temm1e-research`; draft PR74: <https://github.com/temm1e-labs/temm1e/pull/74>.
- Immutable A/B baseline: `da503c09ca5c0f41c308c99e42d4736aa3611f8a`, version5.8.1. Do not modify or move the baseline to improve comparisons.
- Implement the whole modernization, preserving the creator's documented vision and auditing every feature/implementation/math. Sources include `VISION.md`, `FEATURES.md`, `docs/TEMM1E_VISION.md`, feature docs and `tems_lab`.
- Preserve the selected model, messaging-first product, Tem palette/personality and accepted compact transcript TUI with expandable tools/optional panels. Do not silently disable growth or replace personal-host privilege/default stopping semantics.
- Main merge and6.0.0 release are authorized only after broader A/B shows no material regression, real-user readiness and release checks pass. Superiority is not required. Version remains5.8.1; no main merge, tag or release has occurred.
- No messages to others or external bug reports. Only Z.ai coding-plan `glm-5.3-flash` is live-authorized. Other discovered credentials are not permission to use those accounts.
- Secrets remain outside Git. Use `TEMM1E_ZAI_KEY_FILE` pointing to the existing private local file. Never print or commit its contents. Subscription quota is not zero-dollar API usage.
- D01 personal/shared-host semantics and D02 delivery vs automatic durable pursuit remain open in `DECISIONS.md`. Preserve existing defaults while adding safe foundations. D04 growth-specific trust/effect evidence remains a separate boundary; generic verified prose must not confer unrelated capability.
- Do not spawn agents unless current user/developer/AGENTS instructions explicitly authorize delegation. An agent handoff file is not a delegation request.

## Current position

Checkpoint **63** scopes observer trajectory by canonical workspace, channel, chat, user, role and session epoch. Structured serialized key prevents delimiter aliases; invalid/empty/oversized identities or unresolved workspace skip optional observation. Per-engine64scope LRU evicts only idle states; immutable turn views pin state, and full active capacity skips the new optional observer. Current route/owner/limits remain bound per turn. Standalone constructor state remains separate. See `CONSCIOUSNESS-SCOPE.md`.

Actual before Runtime leaked private observer notes/insight from one user to another. After all6independent boundaries pass(user/chat/channel/role/workspace/epoch), same-scope trajectory from61still passes. Unit active64views/no65th/idle eviction/pinned preservation/rebinding model/canonical path/delimiter/invalid identity pass. Full799agentunit+81integration pass;3existing ignored docs. MinimalCLI+tui build passed52.49s(four existing conditional warnings); all5actual CLI controls pass, including `/session-new` preventing prior insight/notes. Scoped agent/all-feature/all-target lint passed21.25s and cleaned3.0GiB. No active Cargo/CLI. Commit/push63. No paid calls.

62 `f1f5454` pushed; CI34302657096 pending.61 `a7342c9` CI34301808114 fully green.60 `2eb1dad` CI34301157770 Windows test job and other checks passed, full build still pending last inspection;58/59 historical Windows error5 failures remain recorded. PR74 body through61. User checked progress; estimate65/100 was explicitly rough overall, not measured checklist completion. No release claim or changed gates.

**Latest user steering:** work faster to conserve Codex quota. Batch related production fixes; use targeted tests/lint for changed packages, relying on full final CI/workspace gates rather than repeating all-feature workspace builds every small checkpoint. Keep per-push handoff and honest release gates. Do not keep expanding optional observer work before closeout.

**Next64:** batch remaining release-critical resource/call-accounting and advertised CLI model selection, then freeze and execute the broader A/B comparison. CoreRuntime runtime.rs:165 still records only successful provider completions; failed/dropped child calls need owner-bound accounting without duplicate success. CLI `/model` still reaches LLM rather than selecting a model. Fix coherently without destroying runtime state or changing selected-model semantics; examine Perpetuum/Eigen-Tune bindings as part of the same batch. Distinguish release blockers from documented legacy limitations; do not mislabel deferrals as completed implementation. Existing original implementation docs propose rules_first/rules_only/confidence caps, later LLM engine/docs emphasize separate reasoning; resolve actual intended policy from vision/feature docs, document conflicts instead of inventing efficacy. Runtime rebuilds may omit/reset engine; Perpetuum author and Eigen-Tune binding also remain. CLI `/model` interception is still pending. Do not clone AgentRuntime (Drop owns background cancellation). Preserve D01/D02 defaults. Continue broader feature/goal/migration/A-B/release gates; don't stop at a checkpoint.

## Recent implementation contracts — retain these

- **Witness55:** `model_verification_max_calls` is optional. None preserves the prior Tier-0 fallback because per-goal USD reservation is unavailable;0disables configured model tiers;1–8is an explicit alternative call policy, not a USD-overhead or subscription-account quota guarantee. Planner allowance is separate. Finite/nonnegative percentage and call bounds validate in TOML/YAML and direct factory, even disabled.
- Configured verifiers use turn-local immutable bindings, shared atomic Tier1+Tier2 call allowance, owning MeteredProvider, estimated final context fit, output cap and30second call timeout. Custom host attachments are retained. Usage is recorded before JSON parsing; error/drop knownness is recorded once; attempts are not refunded. Global USD budget is still a threshold, not a reservation.
- **Witness53/54:** exact sealed file refs; regular UTF-8 files<=16KiB,8refs,32KiB raw bundle,64KiB serialized evidence; goal/rubric16/8KiB. Missing/foreign/unsupported sources abstain without provider calls. Unix nofollow/nonblock handles leaf symlink/FIFO; no complete sandbox or ancestor-race proof. No command/network effects merely to manufacture evidence.
- Snapshots retain exact content/hash and scope after source change/removal. Active goal assessments retain them; old version1records without snapshots remain ungrounded. Model judgments do not prove tests executed or full user-goal completion. Legacy numeric verdict cost stays serialized for ledger compatibility; readout does not present its zero placeholder as free model usage.
- **Goal46–51:** original objective, execution identity, scoped criteria and immutable tool/evaluator evidence persist with CAS/hash checks. User-command inspection is bounded and calls no model. A returned reply stays AwaitingEvidence; interrupted work stays Recovering/unknown, never success inferred from prose. One execution's goal record is not a complete long-lived multi-turn goal manager.
- **Authority48:** nested arbitrary command predicates inherit actual shell authority; restricted copies cannot be re-elevated. The User tool-name policy is not a complete capability system or OS sandbox.
- **Markdown56:** flush waits for queued Tokio writes before store returns;75memory unit (one existing ignored)+7integration and exact Engram CI regression pass. This is not fsync, concurrent-write atomicity or power-loss durability. Scoped lint passed33.04s/clean1.4GiB.

## Validation evidence and known failures

Logs live in the checkout's parent `work` directory, outside disposable target output. Preserve failures and genuine limits.

-55 final: `implementation-witness-policy-tests-v3.log` passes792agent unit+77integration,281core unit (one ignored),95Witness unit+48integration. Added failure/drop case passes with all3wrapper tests, making793agent unit cases before57. `implementation-witness-policy-accounting-tests.log`.
-55 actual CLI: `implementation-witness-policy-cli-enabled-v2.log` passes6requests (2planner/2foreground/2review) across model change, exact evidence bytes, unavailable cost and restart0calls. Zero/omitted policy controls pass4/2; finite-owner controls1planner/0foreground/0review. Build1m31; full workspace lint1m44/clean6GiB.
-55 earlier fixture failures are retained: Tokio mutex `.unwrap` compile errors; missing anti-stub condition prevented Oath sealing; `/model` in CLI chat was forwarded as user input and caused extra unsealed-turn HTTP fixture errors. Corrected actual CLI uses its established `proxy ... model:...` configuration flow. Do not claim this validates server/TUI slash commands.
-54 parser before-test accepted a JSON example inside denying prose;95Witness unit+48integration and full workspace lint1m57/clean2.1GiB pass locally. CI's separate Markdown failure is repaired56.
-53 real Runtime/planner/reviewer/SQLite acceptance verifies changed/deleted source replay, foreign scopes and inner tampering even with recomputed outer document hash. Old empty-evidence positive fixtures were corrected, not deleted. Full workspace lint1m46/clean3.6GiB.
-52 Windows run34291280676: token drop20.7us, legacy40.05ms, deadline1.003s; contended admission1.275s followed by deadline997.64ms and persistence8.33ms. Original48 Windows2s whole-operation timeout cause was not instrumented; do not pretend it is known exactly. Admission outside cancellation/deadline and later durable finalization remain production limits.
- Older real failures:44 outdated MockMemory fixture repaired45;29 duplicate Hive readiness/completion race repaired32. See chronological status/focused docs for evidence.

## Previously implemented foundations — do not redo

The chronological status and focused docs cover: source-backed frontier research and65-family feature mapping; modern model catalog/pricing knownness and Z.ai coding plans; private OAuth refresh; native OpenAI Responses, Anthropic and Gemini replay/streaming; provenance-backed compaction/recall/final fit; scoped conversation heads/history/import/recovery; final delivery outbox; Mission Control and owned background/process lifecycle; browser profile ownership/origin-bound credential submission; recoverable Git snapshots; statistical repairs; verified updater and runnable persistent Docker; Unix PTY-tested compact TUI; coherent connection/credential snapshots through replacements; shared owner accounting and RuntimePolicy/RuntimeResources; Engram identity, numeric policy and backend capability truthfulness; Hive transactional completion. README and18uniform pixel illustrations were regenerated. None of this list alone means full feature acceptance.

Read next: `IMPLEMENTATION-STATUS.md`, `FEATURE-COVERAGE.md`, `03-FINDINGS.md`, `05-IMPLEMENTATION.md`, `MATH-AUDIT.md`, `DECISIONS.md`, and the relevant focused implementation document under `docs/modernization`.

## Remaining architecture and release gates

1. Finish current checkpoint and verify its CI. Continue per-goal USD reservations/knownness, actual durable verifier attempts, execution-bound non-file evidence, queue leases/reconciliation/child lineage and multi-turn goal semantics. Preserve full-coverage unknownness.
2. Complete resource/accounting composition in remaining CoreRuntime errors/cancellation, Consciousness, Perpetuum and Eigen-Tune paths. Provider retry attempts, unknown outcome, durable global accounting and exact whole-wire/model capability/cache contracts remain separate from logical-call counters.
3. Complete remaining feature-family acceptance and classify genuine blockers vs documented boundaries. Open areas include channel/principal namespaces, memory/vault/browser ownership, attachments/interim/control delivery, server/channel restart, retention/GC, Windows process ownership/PTY, MCP lifecycle/rich results, Cambium growth-specific trust, cloud capacity/adapters, S3 abort, Slack paging, OTLP export, and Engram EMA/cadence/Markdown fallback/provenance.
4. Custom-model atomic writes/strict mutations are repaired58. Bounded reads, full capability-knownness propagation, field-level merge semantics and current command-provider identity remain separate; entrypoint identity is repaired59; CLI model switching and broader resource composition remain next.
5. Freeze broader paired held-out A/B corpus, margins, sample and stopping rule before results. Same authorized GLM/model settings, endpoint/account, resources and tools on immutable baseline vs final candidate. Proposed80independent scenarios×3repeats is not a fixed required minimum; repetitions are not independent tasks. Actual entrypoint/factory acceptance must supplement the legacy core harness.
6. The four-task pilot02 passed4/4artifact checks on both versions. A257.003s/B241.266s foreground, A27/B29requests; not broad noninferiority or cost proof. Baseline cache reads unknown; subscription token counts are not invoice savings. False test-success prose occurred in B, so artifact success is not truthfulness. Pilot01 had unfinished background calls and is not efficiency evidence.
7. Release only after final CI/fmt/test/lint/MSRV, CLI/server/TUI and old-profile/update acceptance, broader A/B and `docs/RELEASE_PROTOCOL.md`. Then finalize README/release notes/assets, bump6.0.0, merge and release under the creator's conditional authorization. Keep PR draft until ready. Refresh final output ZIP/patch/manifests last; old bundles are stale.

## Storage, processes and per-push discipline

The creator's disk is small. Every local build/check/test/clippy must use:

```sh
TEMM1E_DATA_DIR=<isolated-regression-profile> python3 scripts/cargo_guard.py -- <cargo args>
```

The guard owns `target/guarded`, disables debug/incremental output, reserves8GiB free, caps target at8GiB and cleans afterward. `--keep-cache` only for a short test/binary batch; finish CLI scripts before cleanup removes the binary. Exit75means unfinished. Never run concurrent guards or unmanaged Cargo. `cargo fmt` is safe outside the guard. Do not build Docker locally. Preserve source, user profiles, secrets, baseline binaries and logs/evidence. Recent cleaned free space was20GiB; recheck before expensive work.

The private companion `work/MODERNIZATION-RESUME.md` contains machine paths and only the secret file location, never its content. Use it to recover active local command IDs after interruption. Last quota read:60%used/40%remaining; no reset authorization or consumption.

For every push: finish scoped implementation and appropriate checks; record actual changes, failures, limits and next steps; inspect staged diff; use normal hooks; commit and push this branch; record commit/CI status in the next update. Continue useful work after the push. Keep Vietnamese progress updates meaningful and do not promise a completion date without evidence.
