<p align="center">
  <img src="assets/banner.png" alt="SkyClaw" width="100%">
</p>

# SkyClaw

Cloud-native Rust AI agent runtime. 38K lines, 905 tests, zero warnings.

## What It Does

SkyClaw is an autonomous AI agent that lives on your server and talks to you through messaging apps. It runs shell commands, browses the web, reads/writes files, fetches URLs, understands images, delegates sub-tasks, self-heals, and learns from its own mistakes — all controlled through natural conversation.

No web dashboards. No config files to edit. Deploy, paste your API key in Telegram, and go.

## Highlights

| Metric | Value |
|--------|-------|
| **Language** | Rust (Edition 2021, MSRV 1.82) |
| **Codebase** | 38,126 lines across 96 source files |
| **Tests** | 905 passing, 0 clippy warnings |
| **Crates** | 13 workspace crates + 1 binary |
| **Features** | 35 implemented across 7 phases |
| **Agent modules** | 20 (AGENTIC CORE) |

## 3-Step Setup

### Step 1: Get a Telegram Bot Token

1. Open Telegram and search for [@BotFather](https://t.me/BotFather)
2. Send `/newbot`
3. Choose a name and a username (must end in `bot`)
4. BotFather replies with your bot token
5. Copy it

### Step 2: Deploy

```bash
git clone https://github.com/nagisanzenin/skyclaw.git
cd skyclaw
cargo build --release
export TELEGRAM_BOT_TOKEN="your-token-here"
./target/release/skyclaw start
```

### Step 3: Activate

1. Open your bot in Telegram
2. Send any message — SkyClaw asks for your API key
3. Paste your key (Anthropic, OpenAI, or Gemini)
4. SkyClaw validates it against the real API and goes online

## Supported Providers

Paste any of these API keys in Telegram — SkyClaw detects the provider automatically:

| Key Pattern | Provider | Default Model |
|------------|----------|---------------|
| `sk-ant-*` | Anthropic | claude-sonnet-4-6 |
| `sk-*` | OpenAI | gpt-5.2 |
| `AIzaSy*` | Google Gemini | gemini-3-flash-preview |

## Channels

| Channel | Status | Feature Flag |
|---------|--------|-------------|
| **Telegram** | Production | `telegram` |
| **Discord** | Production | `discord` |
| **Slack** | Production | `slack` |
| **WhatsApp** | Production | `whatsapp` |
| **CLI** | Built-in | — |

## Tools

| Tool | Description |
|------|-------------|
| **Shell** | Run any command on your server |
| **Browser** | Headless Chrome — navigate, click, type, screenshot, extract text |
| **File ops** | Read, write, list files on the server |
| **Web fetch** | HTTP GET with token-budgeted response extraction |
| **Git** | Clone, pull, push, commit, branch, diff, log |
| **Messaging** | Send real-time updates during multi-step tasks |
| **File transfer** | Send/receive files through messaging channels |

## AGENTIC CORE

SkyClaw's intelligence layer — 20 modules that make it autonomous:

| Category | Modules |
|----------|---------|
| **Resilience** | Circuit breaker, channel reconnection, graceful shutdown, streaming responses |
| **Intelligence** | Task decomposition, self-correction, DONE criteria, cross-task learning |
| **Self-Healing** | Watchdog, state recovery, health-aware heartbeat, memory failover |
| **Efficiency** | Output compression, system prompt optimization, tiered model routing, history pruning |
| **Autonomy** | Parallel tool execution, agent-to-agent delegation, proactive task initiation, adaptive system prompt |
| **Multimodal** | Vision / image understanding (JPEG, PNG, GIF, WebP) |

## Vision Support

SkyClaw can see and understand images. Send a photo through any channel — the runtime automatically:

1. Downloads the image to workspace
2. Base64-encodes it
3. Includes it as an image content part in the provider request
4. The LLM sees and analyzes the image

Supports Anthropic and OpenAI vision formats natively.

## Architecture

13-crate Cargo workspace:

```
skyclaw (binary)
├── skyclaw-core         Traits (13), types, config, errors
├── skyclaw-gateway      HTTP server, health, dashboard, OAuth identity
├── skyclaw-agent        AGENTIC CORE (20 modules)
├── skyclaw-providers    Anthropic, OpenAI-compatible
├── skyclaw-channels     Telegram, Discord, Slack, WhatsApp, CLI
├── skyclaw-memory       SQLite + Markdown with failover
├── skyclaw-vault        ChaCha20-Poly1305 encrypted secrets
├── skyclaw-tools        Shell, browser, file ops, web fetch, git
├── skyclaw-skills       Skill registry (SkyHub v1)
├── skyclaw-automation   Heartbeat, cron scheduler
├── skyclaw-observable   OpenTelemetry, 6 predefined metrics
├── skyclaw-filestore    Local + S3/R2 file storage
└── skyclaw-test-utils   Test helpers
```

## Security

- **Auto-whitelist**: First user to message gets whitelisted. Everyone else denied.
- **Numeric ID only**: Allowlist matches on Telegram user IDs, not usernames.
- **Vault encryption**: ChaCha20-Poly1305 with vault:// URI scheme for secrets.
- **Path traversal protection**: File names sanitized, directory components stripped.
- **Force-push blocked**: Git tool blocks destructive operations by default.

## Self-Configuration

Tell SkyClaw to change its own settings through natural language:

- "Change model to claude-opus-4-6"
- "Switch to GPT-5.2"

Config lives at `~/.skyclaw/credentials.toml` — SkyClaw reads and edits this file itself.

## CLI Reference

```
skyclaw start              Start the gateway daemon
skyclaw chat               Interactive CLI chat
skyclaw status             Show running state
skyclaw config validate    Validate configuration
skyclaw config show        Print resolved config
skyclaw version            Show version info
```

## Development

```bash
cargo check --workspace                                    # Quick compilation check
cargo build --workspace                                    # Debug build
cargo test --workspace                                     # Run all 905 tests
cargo clippy --workspace --all-targets --all-features -- -D warnings  # Lint (0 warnings)
cargo fmt --all                                            # Format
cargo build --release                                      # Release build
```

## Requirements

- Rust 1.82+
- Chrome/Chromium (for browser tool)
- A Telegram bot token

## License

MIT
