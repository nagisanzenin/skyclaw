# Findings register

All repository locations refer to baseline `da503c09ca5c0f41c308c99e42d4736aa3611f8a`. Severity is remediation priority: **P0** = first correctness tranche, **P1** = necessary before stronger production guarantees, **P2** = measured improvement or documentation repair. It is not a CVSS score. Packet IDs refer to [implementation](05-IMPLEMENTATION.md) and [subscription specifications](06-SUBSCRIPTIONS.md).

## F01 — Subscription adapter silently drops volatile instructions

**P0 · Source-confirmed · P01/P05.** `crates/temm1e-codex-oauth/src/responses_provider.rs:44–79` reads `request.system`, not `system_flattened()` or `system_volatile`. The runtime appends mode, user profile, temporal context, consciousness and prompted-tool guidance to the volatile field at `runtime.rs:1494–1572`. The shared request type explicitly defines how to combine those fields.

**Trigger:** use the Codex OAuth provider with any volatile context. **Consequence:** those instructions are absent from the wire request, although other providers can receive them. A better model cannot recover information it was never sent. **Fix:** preserve base and volatile content with one well-defined serialization contract; assert the actual outbound request body in tests.

## F02 — Persistent task queue is not wired into production entry points

**P0 · Source-confirmed call-site gap · P02.** `task_queue.rs` implements creation, checkpoints and incomplete-task queries. `runtime.rs:609` exposes `with_task_queue()`. A repository search finds no production invocation of that builder and no production `TaskQueue::new`; constructors are in tests. `recovery.rs` uses the queue, but that does not establish a running recovery path.

**Consequence:** the queue's unit tests do not substantiate the roadmap's promise that ordinary in-flight goals survive daemon death. Other stores, Perpetuum concerns and conversation persistence still exist; this is not a claim that Tem persists nothing. **Fix:** wire a durable execution service through Start, Chat, TUI, provider rebuilds and workers; test kill/restart through the real composition root.

## F03 — Legacy DONE criteria are never populated or enforced

**P0 · Source-confirmed · P03.** `runtime.rs:1281` creates `_done_criteria`; the only subsequent use is `format_verification_prompt` at line 2235. There is no call that adds or verifies criteria. Even a populated reminder would be appended to `reply_text` after inference, not submitted to the model as another verification round. `done_criteria.rs` unit tests exercise a separate data structure, not this integration.

**Consequence:** this legacy engine does not prove completion. Witness can provide additional checks, so do not describe every Tem turn as entirely unverified. **Fix:** converge on one goal/criterion/evidence contract; retire the empty parallel mechanism after migration.

## F04 — Witness production construction ignores advertised verifier configuration

**P0 · Source-confirmed · P03.** `witness_init.rs:58–94` creates `Witness::new(ledger, current_dir)` and attaches it. `Witness::new` sets Tier 1/2 to `None`. The factory does not wire configured Tier 1/2 providers. It also binds one process CWD rather than the individual session workspace. Config in core defaults Witness on; the older crate-local config defaults it off. Do not use the latter to claim production Witness is disabled.

**Related design gap:** `witness.rs:454` explicitly builds semantic-verifier evidence from workspace path and oath subtask ID, without resolving ledger evidence. Enabling that verifier alone would not make its semantic judgments grounded.

**Fix:** use session-scoped workspace/evidence snapshots, resolve configured tiers through the selected model, and return Inconclusive for absent evidence. Wire capability and configuration tests at the factory.

## F05 — Verification errors and self-audit can preserve unsupported final claims

**P1 · Source-confirmed behavior; policy decision · P03.** `runtime.rs:2312–2318` logs Witness errors and leaves the reply unchanged. The self-audit cleanup at lines 2173–2200 replaces any non-tool audit response, including a malformed or corrective answer, with the cached pre-audit answer. Normal completion updates TaskStatus to Completed at lines 2682–2687 if a queue exists, without consulting the Witness verdict.

**Trigger:** final draft says “done”; verification is unavailable, or the audit returns corrective text without a tool call. **Consequence:** useful uncertainty/correction can be lost while the final narrative remains confident. **Fix:** distinguish delivery, turn termination and verified goal completion. Preserve artifacts; surface unavailable verification and retain open work. Creator decision D02 governs automatic continuation.

## F06 — Perpetuum reports successful maintenance for two no-op activities

**P0 · Source-confirmed · P01/P10.** `crates/temm1e-perpetuum/src/self_work.rs:61–71`: `cleanup_sessions` and `refine_blueprints` ignore their store argument, log “complete,” and return success text without doing work.

**Consequence:** false operational health and unsupported retention/refinement claims. Other GC paths exist, including startup cleanup in `main.rs`; these no-ops do not prove that all cleanup is absent. **Fix now:** explicit NotImplemented/Skipped outcomes. **Implement later:** bounded maintenance with before/after counts, receipts and retention tests.

## F07 — Skull is an allocation policy, not an end-to-end request bound

**P1 · Source-confirmed structure; overflow frequency unmeasured · P04.** `context.rs:54–79` estimates text by bytes and each image as 1,000 tokens. It always retains a minimum recent history and fixed system/tool material. At `context.rs:629–671`, the total is logged, then the dashboard is appended. Runtime volatile additions follow at `runtime.rs:1494–1572`. No final composition-wide rejection/reduction enforces the declared budget. `context.rs` looks up output/model limits without provider-aware custom metadata.

**Trigger:** an oversized user message, numerous tools, images, or accumulated additions. **Consequence:** possible provider rejection or loss of useful context; the absolute “never overflows” promise is unsupported. **Fix:** final accounting after serialization, bounded recovery on overflow, recoverable compaction, consistent capability resolution. An approximate tokenizer is acceptable if uncertainty and overflow handling are explicit.

## F08 — Shell timeout does not terminate the owned work; output is bounded too late

**P0 · Reproduced timeout defect; source-confirmed buffering · P07.** `tools/src/shell.rs:30–45,114–118` runs `Command::output()` under a timeout without `kill_on_drop` or process-tree ownership. A local probe timed out a command at one second; the command wrote a marker later. Recorded `side_effect_after_timeout=true`.

`output()` buffers stdout/stderr until process exit; the 32 KB cap is applied afterwards. It limits model-visible text, not peak subprocess-output memory. **Fix:** owned process groups/job objects, explicit terminate/reap on cancellation, bounded streaming capture with disk-spill limits, structured exit/cancellation state. Test late effects and descendants, not just timeout text.

## F09 — Parallel dependency helper marks conflicting operations independent

**P1 · Reproduced latent defect · P07.** `executor.rs:239–338` uses tool names and JSON path fields. A shell writing `shared.txt` and a file read of it become `[[0],[1]]`; two browser actions do too because `browser` is classified read-only. Probe records both.

**Reachability qualification:** the ordinary runtime loops over tool calls sequentially; the helper's direct callers in this checkout are tests and its public export. This is a latent bug to fix before wiring batch parallelism, not evidence of that race in today's main loop. **Fix:** explicit tool-effect declarations and resource keys; unknown operations are exclusive. Do not turn on the existing helper as a quick efficiency win.

## F10 — Model identity, modality, output limits and pricing are fragmented

**P1 · Source-confirmed · P06.** `core/src/types/model_registry.rs` contains a compiled catalog and treats unknown models as 128K/16,384. `runtime.rs:3754` separately guesses vision support and defaults unknown models to true. `agent/src/budget.rs` has March-2026 substring pricing; OAuth has its own older model list at `responses_provider.rs:604`. Custom model wrappers exist, but core runtime/context calls still use model-only lookups.

**Consequence:** an unknown small local model can receive oversized requests; unsupported images may be sent; quotas and estimated dollar cost can be misleading. Current public model docs show newer families, but their availability in a particular account still needs discovery. **Fix:** one immutable resolved capability snapshot per turn, keyed by provider/connection/model/revision. Unknown is a real state, not a guess of support.

## F11 — OAuth refresh durability and health checks are weaker than their names

**P1 · Source-confirmed design limitations · P05.** `token_store.rs:70–97` serializes refresh within one object. Separate `TokenStore::load()` instances/processes do not share that mutex. Lines 145–160 overwrite the credential file directly, then attempt chmod and ignore errors. Token data derives Debug. A crash mid-write or concurrent rotation can damage/reuse credentials. No actual token leak was observed.

`responses_provider.rs:596` declares health when a token can be obtained; this does not prove backend access or model entitlement. Headless login already validates state/PKCE; do not call it missing CSRF protection. It currently uses manual callback pasting, not a device authorization flow. **Fix:** atomic credential storage, process-level coordination, bounded refresh, redacted diagnostics, separate auth and inference health, account-specific entitlement checks.

## F12 — Coding-plan access is not first-class

**P1 · Source-confirmed + provider documentation · P05/P06.** `providers/src/lib.rs:107–115` defaults Z.ai to the general API endpoint. A custom URL is possible, so coding-plan use is not categorically impossible. The product lacks a distinct plan connection with billing identity, entitlement and quota semantics. The OAuth research document incorrectly frames ChatGPT subscription access as general included API access and presents routes that differ from the actual Codex adapter.

**Fix:** named connection kinds; show what pays for each request; never silently fall back from subscription to paid API. Confirm third-party integration support rather than assuming successful token acquisition establishes it. See [subscription research](06-SUBSCRIPTIONS.md).

## F13 — λ retrieval has no user/tenant scope at its boundary

**P0 for shared deployments · Source-confirmed · P08.** `lambda_memory.rs:194–236` requests candidates/FTS results without a principal argument. `memory/src/sqlite.rs:534–547` selects candidates across the table, with no scope filter. Legacy Knowledge/learning lookups also need scope review; Engram already has user/chat-aware methods.

**Consequence:** a shared memory backend can surface another user's episodic material. This is conditional on deployment sharing; cross-session recall for the same owner is intentional. **Fix:** scope at query and hash-recall authorization, not after prompt assembly. Legacy rows need an explicit ownership migration, not a blanket assignment to whichever user asks first.

## F14 — Cambium's broad pipeline and its production minimal session are different products

**P1 · Source-confirmed / documentation mismatch · P09.** `cambium/src/pipeline.rs:136` marks placeholder self-briefing Passed; code review, security audit and integration testing at lines 349–370 are explicitly Skipped. `main.rs:4270` invokes `run_minimal_session`, which does run cargo checks, clippy and tests on a generated crate. The broad pipeline is not evidence that all advertised growth stages execute in this entry path.

**Fix:** document/select one growth mode, expose per-stage receipts, forbid Skipped→Passed presentation, and define what promotion/installation actually means. Keep existing isolated-generation checks; do not replace them with another superficial checklist.

## F15 — Background cognitive calls bypass shared resource accounting

**P1 · Source-confirmed boundary gap · P10.** `perpetuum/src/cognitive.rs:11–63` exposes `LlmCaller -> String`, invokes the provider directly and discards usage. It cannot return token/cost/quota data to a central controller through this interface. Main-agent and Core budget tracking do not by themselves cover this path.

**Consequence:** always-on monitors/initiative can consume resources outside the visible task budget. Dollar estimates are especially unsuitable as the sole limiter for subscriptions. **Fix:** a shared invocation service records usage for planning, reflection, Witness, Perpetuum and distillation as well as the main loop; enforce token/concurrency limits when monetary data is unknown.

## F16 — Claimed statistical guarantees do not match the examples

**P2 documentation; P1 before local graduation claims · Reproduced arithmetic · P11.** `tems_lab/eigen/LOCAL_ROUTING_SAFETY.md`, Gate 3, says 29/30 passes can establish a 99% Wilson lower bound of at least .95. The actual library yields **.768206**; even 30/30 yields **.818872**. The implementation uses the bound, so the false example does not show that the implementation incorrectly graduates 29/30.

The default embedding judge uses similarity as equivalence (`distill/src/judge/embedding.rs`). Similarity/user continuation are proxies, not verified task correctness. The lab's temperature example calls 21.2°C a fix for 72°F; the correct conversion is 22.222…°C. `ARTIFACT_VALUE_FUNCTION.md` labels .01/hour decay as a ~29-day half-life; it is ~69.31 hours (~2.89 days). **Fix:** executable numeric examples and task-specific outcome tests; withdraw unsupported equivalence/zero-downside language.

## F17 — Read-only anchor permissions do not establish independence from the same OS owner

**P1 if advertised as adversarial integrity · Source-confirmed threat-model gap · P03/P09.** `watchdog/src/main.rs:274–305` writes a sealed file and sets mode 0400. A process with the same filesystem owner can change permissions or replace a file through a writable directory. A separate process and hash chain help detect certain failures, but do not alone establish an immutable anchor against a same-identity actor with host access.

**Fix:** explicitly state the threat boundary. For strong enforcement use separate OS identity/storage ownership or an external append-only witness; personal-host mode may honestly advertise best-effort tamper evidence. No claim of a demonstrated attack is made.

## F18 — Duplicated construction multiplies integration drift

**P1 · Source-confirmed structure; maintenance impact inferred · P12.** `src/main.rs` is 9,492 lines; `runtime.rs` is 4,058. `witness_init.rs` itself documents roughly 25 reconstruction sites. Model changes rebuild runtimes through long builder chains. Earlier architecture docs describe a substantially smaller dependency graph than the current 25-crate workspace.

**Consequence:** new attachments, auth state and limits can be correct in one frontend but absent after a switch elsewhere. File size alone is not a defect; F01/F02/F04 provide concrete integration failures. **Fix:** one runtime factory/service graph and parameterized entrypoint tests before cosmetic file splitting.

## F19 — “No premature completion” lacks an end-to-end evaluation gate

**P1 · Evaluation gap · P11.** Existing library suites pass while F02/F03/F06/F08 remain. Historical lab reports mix tiny samples, simulated comparisons, live-provider observations and architectural assertions. The README and roadmap frequently promote them into universal claims.

**Fix:** measure objective fulfillment and false completion on production entrypoints with independent fixtures, fault injection and artifact checks. A test count, a successful LLM response, passing generated tests or regex absence of `TODO` is insufficient evidence of implementation quality. Keep useful historical results with their original scope.

## Order of work

P01 fixes local truthfulness and request correctness. P02/P03 establish durable state and evidence. P05/P06 address subscription/model access alongside P04 context integrity. P07/P08 close tool and memory boundaries. P09/P10 then make self-growth and perpetual operation trustworthy. P11 supplies the evaluation gates throughout; P12 consolidates entrypoints incrementally after behavior is pinned.

## F20 — Observability advertises export without transport; histograms grow indefinitely

**P1 · Source-confirmed · P13.** `crates/temm1e-observable/src/otel.rs` explicitly stubs OTLP transport; configured endpoint health is not observed collector health. `metrics.rs` retains every histogram observation in a `Vec<f64>`. `create_observable` has no production composition caller found. A working local collector is useful, but does not substantiate distributed tracing/export claims. Replace with bounded aggregation, actual exporter wiring, flush deadlines and measured delivery health. Test a local collector and a million observations with bounded memory.

## F21 — Cloud-scale roadmap overstates runnable infrastructure

**P1 · Source-confirmed call-site gap · P13.** `core/src/orchestrator_impl.rs` supplies `UnimplementedDockerClient` in the Docker factory; Kubernetes is a placeholder. `TenantManager` and gateway `OAuthIdentityManager` have no production construction found. These are separate from working local channel allowlists and Codex account login. Provisioning also checks capacity before awaited creation and later insertion: reserve capacity atomically before implementing concurrent provisioning. Preserve the cloud vision, mark these capabilities experimental until real entrypoint tests pass.

## F22 — Slack upload endpoint retired; polling is not complete history ingestion

**P1 · Source-confirmed plus official compatibility notice · P13.** `channels/src/slack.rs:535` calls `files.upload`, retired November 12, 2025. Replace with external-upload URL acquisition, byte upload and completion. History polling requests up to 100 messages without cursor traversal; bursts can exceed a page. Persist cursor/watermark, deduplicate and obey the actual app's rate tier. Polling every two seconds is not a universal rate entitlement. [Slack upload documentation](https://docs.slack.dev/reference/methods/files.upload/), [history documentation](https://docs.slack.dev/reference/methods/conversations.history/).

## F23 — Shutdown implementation differs from roadmap

**P1 · Source-confirmed in main daemon path · P02/P13.** `src/main.rs:6200` awaits Ctrl-C, then bounded notification and handle joins (five seconds each). No Unix SIGTERM listener was found in this path; the other SIGTERM reference sends a signal to a process. The roadmap's 30-second task checkpoint/drain guarantee is not established. Implement one cancellation supervisor for Ctrl-C, SIGTERM and app shutdown, using durable task leases and an explicit configured drain deadline. Do not treat a sent shutdown notice as a persisted checkpoint.

## F24 — Eigen statistical machinery has material decision/calibration defects

**P1 · Source-confirmed · P11; math detail in MATH-AUDIT.** `distill/src/stats/sprt.rs::decision` forces H1 at maximum samples whenever log likelihood is positive, even below the acceptance boundary. Nominal alpha/beta guarantees do not survive arbitrary truncation. Its comment says closest boundary but sign zero is not their midpoint for asymmetric errors. `stats/power.rs` uses a two-sided confidence lookup for one-sided power: .80 maps to 1.282 instead of approximately .842. Entropy over observed categories measures evenness, not coverage of missing categories. Repair decision semantics, quantiles, inputs and benchmark interpretation before stronger graduation claims.

## F25 — Activity probability conditions on activity itself

**P1 · Reproduced plus source-confirmed arithmetic · P10.** `perpetuum/src/store.rs:526–562` inserts only buckets with count >=1, then estimates probability as positive rows / returned rows. This is 1 for all nonempty ordinarily recorded data. `LIMIT 28` does not enforce a four-week calendar cutoff; `_weekday` is ignored. Use observed exposure hours as denominator, distinguish offline/unknown from inactive, and apply an explicit timezone and prior. Test one active day among 28 observed days, no exposure, DST and stale data.

## F26 — Code snapshot changes staging and does not represent complete restore

**P1 · Source-confirmed · P07.** `tools/src/code_snapshot.rs::action_create` executes `git add -A` against the real index before `write-tree`. Restore uses `read-tree` and `checkout-index -a -f`; files introduced after the snapshot are not thereby removed. This is not an isolated complete-workspace checkpoint. Use a temporary `GIT_INDEX_FILE`, manifest tracked/untracked paths and content hashes, preserve original index bytes, preview deletions, and journal restore. Multi-file patch rollback on ordinary errors is likewise not crash-atomic without a recovery journal.

## F27 — MCP discovery/content adaptation loses protocol information

**P1 · Source-confirmed · P07.** `mcp/src/client.rs::list_tools` fetches one page and ignores `nextCursor`. `call_tool` flattens text and raw data into a string, losing image MIME/type and structured/resource content. Initialization stores the returned protocol version without a supported-version check. Implement bounded cursor traversal with cycle detection, negotiated versions and typed content preservation. Do not pass tool annotations straight into trusted authorization decisions. [MCP tools specification](https://modelcontextprotocol.io/specification/2025-11-25/server/tools).

## F28 — Retries and multipart cleanup need resource contracts

**P1 · Source-confirmed · P07/P10/P13.** Provider rate-limit jitter is a deterministic per-attempt sequence, synchronizing independent clients on the same retry number. Its Retry-After cap can shorten a server's requested pause. Use random per-request jitter, a separate absolute not-before deadline, and account-level coordination. `filestore/src/s3.rs::store_stream` returns early on multipart errors without aborting the upload; add explicit abort on errors plus durable cleanup records/lifecycle expiry for crash cases. Chunk buffering must also have a bound independent of producer chunk size.

## F29 — Anima's raw measurements are not language-neutral evidence

**P2 · Source-confirmed · P11.** `anima/src/facts.rs` calls UTF-8 byte length `char_count`, divides character punctuation by bytes, counts code-fence delimiters as blocks and ZWJ/variation selectors as emoji. Language detection is explicitly unknown; greeting/command heuristics are largely English. Preserve personalization and its ethics thresholds, correct measurement units, expose unknown values and test graphemes/multilingual text. A model's confidence field and 50-turn persona experiment do not establish calibrated personality inference.

## F30 — RBAC and specialist workspace defaults do not compose safely for shared hosting

**P1 for shared hosting · Source-confirmed; no exploit claimed · P08/P12.** `main.rs::get_user_role` falls back to Admin for missing role, file error or unknown user. `core/types/rbac.rs` denies four tool names, permitting newly introduced tools by default. `cores/src/runtime.rs` constructs an Admin session; `invoke_tool.rs` chooses process CWD ahead of caller workspace. Ordinary User role explicitly blocks invoke_core, so this alone is NOT a demonstrated User-to-Admin bypass. Carry principal, workspace and capabilities through delegation and use explicit owner bootstrap. Preserve personal-host autonomy pending creator decision; reject missing identity in shared-host profile.

## F31 — Screen change is an observation, not success or failure of intent

**P2 · Source-confirmed design risk · P07/P11.** `gaze/src/diff.rs` uses channel/pixel and region-change thresholds. No-change text urges retry; a successful clipboard action or invisible submission need not change pixels, while animation can change pixels without satisfying a task. Keep cheap diff as an observation aid; record semantic postconditions separately and require reconciliation before retrying uncertain non-idempotent actions. Test scale transforms, multi-monitor origins and UI animations on each supported native platform.

## F32 — Existing compaction drops semantics; existing caching loses accounting

**P0 accounting / P1 continuity · Source-confirmed · P04/P14.** `agent/src/context.rs::generate_dropped_summary` retains only the first three user topic prefixes (about 50 bytes each) and tool names. The separate chat digest excludes tool evidence and truncates assistant text. These are useful low-cost hints, not a durable task handoff. `history_pruning::prune_history` has no production call found, although its grouping and orphan-removal helpers are used. Do not attribute its semantic-scoring claims to the active path.

Anthropic already marks the stable base prompt cacheable; Tem is not starting from zero. However `AnthropicUsage` and core `Usage` omit cache creation/read counts. Anthropic's input_tokens excludes both categories, so current accounting loses actual input and associated charges. OpenAI/Gemini automatic caching may work even without explicit controls; source alone does not establish hit rate. See [dedicated audit](07-CONTEXT-CACHING.md).

## F33 — Search cache key omits sort order

**P1 · Reproduced key collision · P14.** `tools/src/web_search/cache.rs::CacheKey` includes query, filters and result budgets but not `SearchRequest.sort`; dispatcher sorts by relevance or publication time before caching output. A repeated query with a changed sort can receive the earlier cached ordering. Add sort and backend/config/account identity where output depends on them. Test identical queries with opposite sort policies and distinct timestamps. Preserve the useful TTL and entry-capacity bounds; add a byte bound.

## F34 — TUI has rendering scaffolding without complete runtime event wiring

**P1 · Source-confirmed / UX proposal · P15.** No production `Event::StreamChunk` emitter found; bridge sends early/final replies and watched phases. Tool history inferred by name can miss coalesced phase updates. Zero usage is used to infer an early reply. Pending-message steering is not supplied by the bridge; interrupts are supplied. Transcript rendering walks all lines before viewport slicing. Replace inferred UI state with durable item events, real streaming and viewport caching. [Dedicated TUI audit and specification](08-TUI.md).

## F35 — Installer can say checksum verified without computing it

**P1 · Source-confirmed · P13.** `install.sh:110–132` sets ACTUAL=EXPECTED when neither checksum utility exists, then prints Checksum verified. Missing checksum file/entry also allows installation after a warning. Preserve straightforward installation, but fail normal verified install when verification is unavailable; an explicit development override must say unverified. A checksum downloaded from the same release is integrity checking, not independent publisher authentication.

## F36 — Vigil UTF-8 truncation can panic before checking a boundary

**P1 · Reproduced caught panic · P07/P13.** `perpetuum/src/bug_reporter.rs::format_issue_body` slices `body[..MAX_ISSUE_BODY]` before using char_indices to seek a safe boundary. If byte60000 falls inside a multibyte character, the slice panics first. Compute boundary on the unsliced string (or decrement until is_char_boundary), then slice. Fixture: long Unicode error/triage positioned across byte60000, report stays valid UTF-8 and within limit. Keep redaction/approval evidence tests separate; no actual external report was submitted during audit.

## Transport identity follow-up (September 2026)

The server's worker slots and pending amendments were keyed only by chat ID. Cross-platform ID collisions could share active state; pending messages were reduced to text and later reconstructed with a random ID and the preceding request's author. In addition, shared outbound tools were bound to the primary channel. The modernization follow-up carries structural transport/chat keys, original inbound records and session channel into tools, and resolves outgoing tools from that authenticated transport. Direct fixture coverage is recorded in `IMPLEMENTATION-STATUS.md`. Durable amendment admission, full fake-server dispatch/restart and per-user authorization of mid-turn steering remain separate requirements.

The DONE preamble was also observed during a live recall request. Runtime promotion of duplicated user text into System history has been removed; proportional stable planning guidance replaces mandatory checklist formatting. A prompt or a classifier difficulty is not an independent completion proof. F03's original empty-object observation is historical; the remaining architectural requirement is typed goal evidence, not reinstating the unused object.
