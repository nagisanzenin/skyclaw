# SkyClaw Features

> All implemented features across the SkyClaw agent runtime. 905 tests, 0 clippy warnings.

---

## Phase 0 — Harden the Foundation

| # | Feature | Module | Status |
|---|---------|--------|--------|
| 0.1 | Graceful Shutdown | `src/main.rs` | Done |
| 0.2 | Provider Circuit Breaker | `skyclaw-agent/src/circuit_breaker.rs` | Done |
| 0.3 | Channel Reconnection with Backoff | `skyclaw-channels/src/telegram.rs` | Done |
| 0.4 | Streaming Responses | `skyclaw-agent/src/streaming.rs` | Done |
| 0.5 | Raised max_turns/max_tool_rounds | `skyclaw-agent/src/runtime.rs` | Done |

## Phase 1 — The Agentic Core

| # | Feature | Module | Status |
|---|---------|--------|--------|
| 1.1 | Verification Engine | `skyclaw-agent/src/runtime.rs` | Done |
| 1.2 | Task Decomposition | `skyclaw-agent/src/task_decomposition.rs` | Done |
| 1.3 | Persistent Task Queue with Checkpointing | `skyclaw-agent/src/task_queue.rs` | Done |
| 1.4 | Context Manager — Surgical Token Budgeting | `skyclaw-agent/src/context.rs` | Done |
| 1.5 | Self-Correction Engine | `skyclaw-agent/src/self_correction.rs` | Done |
| 1.6 | DONE Definition Engine | `skyclaw-agent/src/done_criteria.rs` | Done |
| 1.7 | Cross-Task Learning | `skyclaw-agent/src/learning.rs` | Done |

## Phase 2 — Self-Healing Agent

| # | Feature | Module | Status |
|---|---------|--------|--------|
| 2.1 | Watchdog | `skyclaw-agent/src/watchdog.rs` | Done |
| 2.2 | State Recovery | `skyclaw-agent/src/recovery.rs` | Done |
| 2.3 | Health-Aware Heartbeat | `skyclaw-automation/src/heartbeat.rs` | Done |
| 2.4 | Memory Backend Failover | `skyclaw-memory/src/lib.rs` | Done |

## Phase 3 — Efficiency & Intelligence

| # | Feature | Module | Status |
|---|---------|--------|--------|
| 3.1 | Output Compression | `skyclaw-agent/src/output_compression.rs` | Done |
| 3.2 | System Prompt Optimization | `skyclaw-agent/src/prompt_optimizer.rs` | Done |
| 3.3 | Tiered Model Routing | `skyclaw-agent/src/model_router.rs` | Done |
| 3.4 | History Pruning with Semantic Importance | `skyclaw-agent/src/history_pruning.rs` | Done |

## Phase 4 — Ecosystem

| # | Feature | Module | Status |
|---|---------|--------|--------|
| 4.1 | Discord Channel | `skyclaw-channels/src/discord.rs` | Done |
| 4.2 | Git Tool | `skyclaw-tools/` | Done |
| 4.3 | Skill Registry (SkyHub v1) | `skyclaw-skills/src/lib.rs` | Done |
| 4.4 | Slack Channel | `skyclaw-channels/src/slack.rs` | Done |
| 4.5 | Web Dashboard (Minimal) | `skyclaw-gateway/src/dashboard.rs` | Done |

## Phase 5 — Cloud Scale

| # | Feature | Module | Status |
|---|---------|--------|--------|
| 5.1 | S3/R2 FileStore Backend | `skyclaw-filestore/src/s3.rs` | Done |
| 5.2 | OpenTelemetry Observability | `skyclaw-observable/src/` | Done |
| 5.3 | Multi-Tenancy with Workspace Isolation | `skyclaw-core/src/tenant_impl.rs` | Done |
| 5.4 | OAuth Identity Flows | `skyclaw-gateway/src/identity.rs` | Done |
| 5.5 | Horizontal Scaling via Orchestrator | `skyclaw-core/src/orchestrator_impl.rs` | Done |

## Phase 6 — Advanced Agentic Core

| # | Feature | Module | Status |
|---|---------|--------|--------|
| 6.1 | Parallel Tool Execution | `skyclaw-agent/src/executor.rs` | Done |
| 6.2 | Agent-to-Agent Delegation | `skyclaw-agent/src/delegation.rs` | Done |
| 6.3 | Proactive Task Initiation | `skyclaw-agent/src/proactive.rs` | Done |
| 6.4 | Adaptive System Prompt — Self-Tuning | `skyclaw-agent/src/prompt_patches.rs` | Done |

## Phase 7 — Multimodal

| # | Feature | Module | Status |
|---|---------|--------|--------|
| 7.1 | Vision / Image Understanding | `skyclaw-core/src/types/message.rs`, `skyclaw-providers/`, `skyclaw-agent/src/runtime.rs` | Done |

---

## Feature Details

### 0.1 Graceful Shutdown
Traps SIGTERM/SIGINT, drains active ChatSlot workers, flushes pending memory writes. Tasks that can't complete within 30s are checkpointed for resume.

### 0.2 Provider Circuit Breaker
State machine: Closed → Open (after N failures) → Half-Open (after cooldown). Exponential backoff with jitter on transient errors (429, 500, 503). Provider failover when multiple configured.

### 0.3 Channel Reconnection
Supervised retry loop with exponential backoff for Telegram long-poll. Health-checks connection via heartbeat. Logs all reconnection attempts.

### 0.4 Streaming Responses
`StreamBuffer` + `StreamingConfig` + `StreamingNotifier`. Uses `Provider::stream()` for final text responses. Edit-in-place on Telegram (throttled at 30 edits/min). Status updates during tool rounds.

### 0.5 Raised Limits
`max_turns=200`, `max_tool_rounds=200`, `max_task_duration=1800s`. Configurable via `AgentRuntime::with_limits()`.

### 1.1 Verification Engine
After every tool execution, injects verification hint into tool result: "Did the action succeed? What evidence confirms this?" Zero API call overhead — prompt injection only.

### 1.2 Task Decomposition
`TaskGraph` with `SubTask` nodes and dependency edges. Topological sort for execution order. Status tracking per subtask (Pending/Running/Completed/Failed/Blocked). Cycle detection prevents infinite loops.

### 1.3 Persistent Task Queue
SQLite-backed `TaskQueue`. `TaskEntry` stores task_id, chat_id, goal, status, checkpoint_data (serialized session JSON). After each tool round, runtime checkpoints session state. Survives process restarts.

### 1.4 Context Manager
Priority-based token budgeting across 7 categories: system prompt (always), tool definitions (always), task state (if present), recent 4-8 messages (always), memory search (15% cap), cross-task learnings (5% cap), older history (fill remaining). Dropped messages get summary injection.

### 1.5 Self-Correction Engine
`FailureTracker` counts consecutive failures per tool name. After threshold (default 2), injects strategy rotation prompt: "This approach has failed N times. Try a fundamentally different approach."

### 1.6 DONE Definition Engine
Detects compound tasks (multiple verbs, numbered lists, "and"/"then" connectors). Injects DONE criteria prompt for the LLM to articulate verifiable completion conditions. Appends verification reminder on final reply.

### 1.7 Cross-Task Learning
`extract_learnings()` analyzes completed history — tools used, failures, strategy rotations — produces `TaskLearning` with task_type, approach, outcome, lesson. Stored in memory with `learning:` prefix. Injected into future context at 5% budget.

### 2.1 Watchdog
Monitors subsystems (provider, memory, channel, tools). `WatchdogConfig` with check intervals and failure thresholds. `HealthReport` with per-subsystem status. Auto-restarts degraded subsystems.

### 2.2 State Recovery
`RecoveryManager` detects corrupted state (broken sessions, orphaned tasks). Generates `RecoveryPlan` with actions: Restart, Rollback, Skip, Escalate. Integrates with task queue checkpoints.

### 2.3 Health-Aware Heartbeat
Heartbeat checks subsystem health via watchdog. Reports degraded/failed subsystems. Adjusts interval based on system health.

### 2.4 Memory Backend Failover
Automatic failover from primary to secondary memory backend on failure. Configurable primary/secondary pair. Auto-recovery when primary returns.

### 3.1 Output Compression
Compresses large tool outputs before storing in context. Extracts key information, discards verbose noise. Keeps first/last N lines of shell output with summary.

### 3.2 System Prompt Optimization
`SystemPromptBuilder` for composable prompt construction. Injects workspace path, tool names, file protocol, verification rules, DONE criteria rules, self-correction rules. Token estimation.

### 3.3 Tiered Model Routing
`ModelRouter` routes tasks to `ModelTier` (Fast/Standard/Premium) based on `TaskComplexity` analysis. Simple questions use cheap/fast models. Multi-step tasks use premium models.

### 3.4 History Pruning
`score_message()` assigns `MessageImportance` (Critical/High/Medium/Low) based on role, content, and tool results. `prune_history()` removes lowest-importance messages first. Preserves conversation coherence.

### 4.1 Discord Channel
Full `Channel` + `FileTransfer` implementation via serenity/poise. Slash commands, message splitting, allowlist enforcement, attachment handling. Behind `discord` feature flag.

### 4.2 Git Tool
Typed git operations: clone, pull, push, commit, branch, diff, log. Safety: blocks force-push by default, requires explicit confirmation for destructive operations.

### 4.3 Skill Registry
`SkillRegistry` scans `~/.skyclaw/skills/` and workspace `skills/`. Parses YAML frontmatter from Markdown. Keyword-based relevance matching. Injects skill instructions into system prompt when relevant.

### 4.4 Slack Channel
`SlackChannel` implementing Channel + FileTransfer. Poll-based retrieval (conversations.list + conversations.history every 2s). chat.postMessage, files.upload. Message splitting at 4000 chars, allowlist, rate limiting. Behind `slack` feature flag.

### 4.5 Web Dashboard
4 handlers: dashboard_page (HTML), dashboard_health (JSON), dashboard_tasks (JSON), dashboard_config (redacted JSON). HTMX-based, dark theme, <50KB, polls health every 10s. Served at `/dashboard`.

### 5.1 S3/R2 FileStore
`S3FileStore` via aws-sdk-s3. Supports R2/MinIO (custom endpoint + force_path_style). Multipart upload for `store_stream()`, presigned URLs, paginated listing. Behind `s3` feature flag.

### 5.2 OpenTelemetry Observability
`MetricsCollector` with atomic counters, RwLock gauges/histograms. `OtelExporter` wrapping it with OTLP endpoint. 6 predefined metrics (provider latency, token usage, tool success rate, etc.).

### 5.3 Multi-Tenancy
`TenantManager` implementing Tenant trait. Per-tenant workspace isolation (workspace/, vault/, memory.db). Rate limiting with day rollover. `ensure_workspace()` creates isolation dirs.

### 5.4 OAuth Identity
`OAuthIdentityManager` with in-memory user store, PKCE support. start_oauth_flow(), complete_oauth_flow(), refresh_token(). Multi-provider (GitHub, Google, AWS). Agent sends OAuth URL in chat, user clicks, callback to gateway, token stored.

### 5.5 Horizontal Scaling
`DockerOrchestrator` with DockerClient abstraction. Max instances safety limit, no privilege escalation. `KubernetesOrchestrator` stub. `create_orchestrator()` factory.

### 6.1 Parallel Tool Execution
`execute_tools_parallel()` with Semaphore-based concurrency limit (max 5). `detect_dependencies()` using union-find grouping — read-read independent, write-write/write-read dependent, shell always dependent.

### 6.2 Agent-to-Agent Delegation
`DelegationManager` with `AtomicUsize`-based spawn counter. `plan_delegation()` decomposes tasks via 4 heuristic strategies (numbered lists, semicolons, "then", "and"). `SubAgent` with scoped model/tools/timeout. Sub-agents cannot spawn further sub-agents (no recursion). Max 10 per task, max 3 concurrent.

### 6.3 Proactive Task Initiation
`ProactiveManager` with `TriggerRule` system. 4 trigger types: FileChanged, CronSchedule, Webhook, Threshold. Disabled by default (global opt-in required). Rate limiting (10 actions/hour). Per-rule cooldowns. `requires_confirmation` flag for destructive operations.

### 6.4 Adaptive System Prompt
`PromptPatchManager` with 5 patch types (ToolUsageHint, ErrorAvoidance, WorkflowPattern, DomainKnowledge, StylePreference). All patches start as Proposed. Only Approved patches injected. Auto-approve only for low-risk types above 0.8 confidence. Underperforming patches auto-expire. Max 20 patches.

### 7.1 Vision / Image Understanding
`ContentPart::Image` variant with base64 data + media_type. Runtime detects image attachments (JPEG, PNG, GIF, WebP), reads from workspace, base64-encodes, and includes as image content parts in provider requests. Anthropic format: `{"type": "image", "source": {"type": "base64", ...}}`. OpenAI format: `{"type": "image_url", "image_url": {"url": "data:...;base64,...", "detail": "auto"}}`. Context budgeting estimates ~1000 tokens per image.
