# Cambium: Architecture Reference

> **Status:** Reference document for cambium crate map  
> **Date:** 2026-04-08  
> **Purpose:** Tem's self-knowledge of its own architecture (Principle 6)

---

## Workspace Overview

TEMM1E v4.6.1 is a Cargo workspace with 22 crates plus a root binary.
Edition: Rust 2021. Minimum Rust version: 1.91.1.

---

## Message Flow

```
Channel.start()
  -> inbound message via mpsc::channel
    -> Gateway router (SkyGate)
      -> Agent runtime loop (AgentRuntime)
        -> Provider.complete() or Provider.stream()
        <- CompletionResponse (may contain tool_use)
        -> Tool.execute() if tool_use
        <- ToolOutput fed back to provider
      <- Final response
    -> Channel.send_message(OutboundMessage)
```

---

## Crate Map

### Level 0 -- Immutable Kernel

These crates define the foundational contracts. Self-grow MUST NOT modify them.

#### temm1e-core

- **Purpose:** Shared traits, types, error enum, config loader -- the contract layer for the entire system.
- **Key public types:** `Channel`, `Provider`, `Memory`, `Tool`, `FileStore`, `Vault`, `Orchestrator`, `Tenant`, `Observable`, `Identity`, `Tunnel`, `Usage`, `Setup`, `Peripheral`, `FileTransfer`, `Temm1eError`, `CompletionRequest`, `CompletionResponse`, `InboundMessage`, `OutboundMessage`, `MemoryEntry`, `SessionContext`, `ToolInput`, `ToolOutput`, `ToolDeclarations`, `ModelRegistry`
- **Trust level:** Level 0 (immutable). All trait definitions live here. Modifying any trait changes the contract for every crate in the workspace.

#### temm1e-vault

- **Purpose:** Encrypted secrets management with ChaCha20-Poly1305, credential detection, and `vault://` URI resolution.
- **Key public types:** `LocalVault`, `DetectedCredential`, `VaultUri`, `detect_credentials`, `resolve`, `is_vault_uri`, `parse_vault_uri`
- **Trust level:** Level 0 (immutable). Handles all secret storage and credential isolation. A flaw here compromises every user's API keys.

### Level 1 -- Approval Required

These crates implement core runtime logic. Changes require explicit approval and full pipeline verification.

#### temm1e-gateway

- **Purpose:** HTTP/WebSocket server, routing, session management, health checks, dashboard.
- **Key public types:** `SkyGate`, `OAuthIdentityManager`, `SetupTokenStore`, `init_start_time`
- **Trust level:** Level 1 (approval required). Entry point for all network traffic. Security boundary.

#### temm1e-agent

- **Purpose:** Agent runtime loop, context management, executor, budget tracking, complexity classification, prompt optimization, history pruning, self-correction, task decomposition, blueprint system, circuit breaker, streaming.
- **Key public types:** `AgentRuntime`, `AgentTaskStatus`, `AgentTaskPhase`, `Blueprint`, `BlueprintPhase`, `BudgetTracker`, `CircuitBreaker`, `DelegationManager`, `DoneCriteria`, `ToolCall`, `ToolCallResult`, `execute_tools_parallel`, `detect_dependencies`
- **Trust level:** Level 1 (approval required). The brain of the system. Controls how messages become actions.

#### temm1e-providers

- **Purpose:** AI provider integrations -- Anthropic (native), OpenAI-compatible (OpenAI, Grok, OpenRouter, MiniMax, Z.ai), Gemini (native).
- **Key public types:** `AnthropicProvider`, `OpenAICompatProvider`, `GeminiProvider`, `create_provider()`
- **Trust level:** Level 1 (approval required). Manages API key handling, request construction, response parsing. Errors here break all AI functionality.

#### temm1e-perpetuum

- **Purpose:** Perpetual time-aware entity framework -- scheduling, monitoring, volition, conscience state machine, cognitive LLM-driven scheduling.
- **Key public types:** `Perpetuum`, `Chronos`, `Cognitive`, `Conscience`, `ConscienceState`, `Cortex`, `Pulse`, `PulseEvent`, `Store`, `WakeTrigger`, `SelfWorkKind`
- **Trust level:** Level 1 (approval required). Controls autonomous behavior. A bug here means Tem acts without user intent.

#### temm1e-memory

- **Purpose:** Persistent memory backends -- SQLite (production), Markdown (OpenClaw-compatible), ResilientMemory (failover decorator).
- **Key public types:** `SqliteMemory`, `MarkdownMemory`, `ResilientMemory`, `SqliteUsageStore`, `FailoverConfig`, `MemoryHealthStatus`, `create_memory_backend()`
- **Trust level:** Level 1 (approval required). Data persistence layer. Corruption here loses conversation history.

### Level 2 -- Autonomous with Full Pipeline

Self-grow may modify these crates autonomously, provided all compilation gates pass (cargo check, clippy, fmt, test).

#### temm1e-tools

- **Purpose:** Agent tool implementations -- shell, file ops, browser, web fetch, git, credential scrubbing, Prowl blueprints, custom tools, desktop control, messaging tools.
- **Key public types:** `ShellTool`, `FileReadTool`, `FileWriteTool`, `FileListTool`, `BrowserTool`, `BrowserPool`, `GitTool`, `KeyManageTool`, `LambdaRecallTool`, `CheckMessagesTool`, `CustomToolRegistry`, `SelfCreateTool`, `WebFetchTool`, `credential_scrub`
- **Trust level:** Level 2 (autonomous full pipeline). Leaf crate. New tools add capability without changing contracts.

#### temm1e-skills

- **Purpose:** Skill registry and execution -- discovers, parses, and indexes Markdown skill files with YAML frontmatter.
- **Key public types:** `Skill`, `SkillRegistry`
- **Trust level:** Level 2 (autonomous full pipeline). Leaf crate. Adding skills extends behavior without modifying core.

#### temm1e-channels

- **Purpose:** Messaging channel implementations -- CLI, Telegram, Discord, WhatsApp (Cloud API), WhatsApp Web, Slack. Feature-flagged.
- **Key public types:** `CliChannel`, `TelegramChannel`, `DiscordChannel`, `SlackChannel`, `WhatsAppChannel`, `read_file_for_sending`, `save_received_file`
- **Trust level:** Level 2 (autonomous full pipeline). Leaf crate. Each channel is isolated behind feature flags.

#### temm1e-cores

- **Purpose:** TemDOS specialist sub-agent cores -- architecture, code-review, test, debug, web, desktop, research, creative. Cores inform but never steer.
- **Key public types:** `CoreDefinition`, `InvokeCoreTool`, `CoreRegistry`, `CoreRuntime`, `CoreResult`, `CoreStats`
- **Trust level:** Level 2 (autonomous full pipeline). Leaf crate. Cores are sandboxed sub-agents.

#### temm1e-anima

- **Purpose:** Social intelligence -- emotional intelligence, user profiling, personality system, communication style, ethics layer.
- **Key public types:** `SocialStorage`, `UserModel`, `Personality`, `CommunicationStyle`, `Ethics`
- **Trust level:** Level 2 (autonomous full pipeline). Leaf crate. Affects tone and style, not core behavior.

#### temm1e-observable

- **Purpose:** OpenTelemetry tracing, metrics collection, file logging, OTLP export.
- **Key public types:** `MetricsCollector`, `OtelExporter`, `create_observable()`
- **Trust level:** Level 2 (autonomous full pipeline). Leaf crate. Observation-only; does not affect runtime behavior.

#### temm1e-hive

- **Purpose:** Stigmergic swarm intelligence -- parallel worker coordination via SQLite blackboard and pheromone signal field.
- **Key public types:** `Hive`, `Queen`, `Worker`, `Blackboard`, `Pheromone`, `TaskDAG`, `HiveConfig`
- **Trust level:** Level 2 (autonomous full pipeline). Isolated subsystem behind feature flag.

#### temm1e-distill

- **Purpose:** Eigen-Tune self-tuning distillation engine -- collects request/response pairs, scores quality, curates datasets, trains local models, monitors drift.
- **Key public types:** `EigenTuneEngine`, `EigenTuneCollector`, `EigenTuneScorer`, `EigenTuneStore`, `EigenTuneRouter`, `EigenTuneConfig`, `EigenTier`, `EigenTuneStatus`, `QualitySignal`, `RouteDecision`
- **Trust level:** Level 2 (autonomous full pipeline). Closed-loop subsystem. Failures degrade to cloud provider.

### Level 2 -- Autonomous (continued)

#### temm1e-mcp

- **Purpose:** Model Context Protocol client -- connects to external MCP servers (stdio/HTTP), discovers tools, bridges them as native TEMM1E tools via JSON-RPC 2.0.
- **Key public types:** `McpManager`, `McpConfig`, `McpServerConfig`, `McpManageTool`, `SelfAddMcpTool`
- **Trust level:** Level 2 (autonomous full pipeline). External integration adapter. Tool names are sanitized at the bridge layer.

#### temm1e-codex-oauth

- **Purpose:** ChatGPT Plus/Pro subscription via OAuth PKCE -- enables OpenAI Responses API access without an API key.
- **Key public types:** `CodexResponsesProvider`, `CodexOAuthTokens`, `TokenStore`
- **Trust level:** Level 2 (autonomous full pipeline). Feature-flagged. If OpenAI blocks third-party OAuth, this crate compiles away to nothing.

#### temm1e-gaze

- **Purpose:** Desktop vision control -- cross-platform screen capture (xcap) and input simulation (enigo) for full computer control with SoM overlay.
- **Key public types:** `DesktopController`
- **Trust level:** Level 2 (autonomous full pipeline). Platform-specific, feature-flagged.

#### temm1e-tui

- **Purpose:** Interactive terminal UI -- Markdown rendering, syntax highlighting, real-time agent observability, onboarding wizard, slash commands. Uses TEA (Elm Architecture) pattern with ratatui.
- **Key public types:** `App`, `AppState`, `AgentBridge`, `TuiChannel`
- **Trust level:** Level 2 (autonomous full pipeline). UI-only crate. Does not affect agent logic.

#### temm1e-automation

- **Purpose:** Heartbeat runner, task scheduling, autonomous agent execution.
- **Key public types:** `HeartbeatRunner`
- **Trust level:** Level 2 (autonomous full pipeline). Scheduling infrastructure.

#### temm1e-filestore

- **Purpose:** File storage backends -- local filesystem and S3-compatible (AWS S3, R2, MinIO). Feature-flagged.
- **Key public types:** `LocalFileStore`, `S3FileStore`, `create_filestore()`
- **Trust level:** Level 2 (autonomous full pipeline). Leaf crate. Storage-only.

#### temm1e-test-utils

- **Purpose:** Shared test utilities -- mock implementations of core traits, factory helpers, fluent config builder.
- **Key public types:** `MockProvider`, `MockChannel`, `MockMemory`, `MockTool`
- **Trust level:** Level 2 (autonomous full pipeline). Test-only crate. Never compiled into release binary.

### Level 3 -- Autonomous Basic

Not crates, but file zones where cambium operates with minimal oversight:

- `docs/**` -- Documentation files
- `tests/**` -- Integration test files
- `~/.temm1e/skills/*.md` -- User-installed skill definitions
- `~/.temm1e/cores/*.md` -- User-installed core definitions

---

## Root Binary

#### src/main.rs

- **Purpose:** CLI entry point (clap). Wires all crates together: config loading, channel startup, gateway server, agent runtime initialization.
- **Trust level:** Level 1 (approval required). The wiring layer. Changes here affect how every crate interacts.

---

## Dependency Rules

1. **Traits in core, implementations in crates.** All shared traits are in `temm1e-core/src/traits/`. Implementations live in their respective crates.
2. **No cross-implementation dependencies.** Leaf crates (providers, channels, tools, memory) must never depend on each other.
3. **Feature flags for optional dependencies.** Platform-specific channels and tools are behind Cargo feature flags.
4. **Factory pattern.** Each crate exposes a `create_*()` factory function that dispatches by name string.
