# CLI reference

```
temm1e setup                 Interactive first-time setup wizard
temm1e tui                   Interactive TUI — Full-screen terminal interface
temm1e start                 Start the gateway (foreground or -d for daemon)
temm1e start --personality none  No personality, minimal identity prompt
temm1e stop                  Graceful shutdown
temm1e chat                  Interactive CLI chat (basic, no TUI)
temm1e status                Show running state
temm1e update                Pull latest + rebuild
temm1e auth login            Codex OAuth (browser or --headless)
temm1e auth status           Check token validity
temm1e auth logout           Clear stored tokens
temm1e config validate       Validate temm1e.toml
temm1e config show           Print resolved config
temm1e reset --confirm       Factory reset with backup
```

**In-chat commands:**

```
/help                Show available commands
/model               Show current model and available models
/model <name>        Switch to a different model
/memory              Show current memory strategy
/memory lambda       Switch to λ-Memory (decay + persistence)
/memory echo         Switch to Echo Memory (context window only)
/keys                List configured providers
/addkey              Securely add an API key
/usage               Token usage and cost summary
/mcp                 List connected MCP servers
/mcp add <name> <cmd>  Connect a new MCP server
/eigentune           Self-tuning status and control
/login <service>     OTK browser login (100+ services or custom URL)
/timelimit           Show current task time limit
/timelimit <secs>    Set hive task time limit (e.g. /timelimit 3600)
```

---


## Local conversation history (CLI and TUI)

Tem keeps the full native conversation in the selected profile's canonical workspace, shown by CLI at startup. The default remains `~/.temm1e/workspace`; launching from another directory does not select a different project. Each interface has its own conversation identity.

- `/history-import` previews the preserved legacy chat and shows a confirmation command. Import copies it into an empty conversation, leaves the source untouched and never replays tools. Old chats have no reliable workspace identity, so Tem does not guess where they belong.
- `/session-new` starts a fresh conversation epoch and preserves the old history and execution evidence.
- `/session-recover` inspects a turn interrupted by process death. Its confirmation restores the saved evidence, including unknown tool outcomes, without executing tools or resending replies. Inspect external state before retrying an uncertain effect.
- TUI `/clear` clears the display only. It does not erase or reset agent memory.

A second local process cannot dispatch another turn into the same conversation while it is locked. Invalid or oversized history produces an explicit error; it is not silently replaced with an empty chat. The current active-history limit is 32 MiB / 100,000 messages. This is not a total profile disk quota or a multi-host lock protocol. Server channel migration is still in progress on the modernization branch.
