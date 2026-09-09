# CLI reference

```
temm1e setup                 Interactive first-time setup wizard
temm1e tui                   Interactive TUI — Full-screen terminal interface
temm1e start                 Start the gateway (foreground or -d for daemon)
temm1e start --personality none  No personality, minimal identity prompt
temm1e stop                  Graceful shutdown
temm1e chat                  Interactive CLI chat (basic, no TUI)
temm1e status                Show running state
temm1e update                Download and verify the latest release binary
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
/model <name>        Select model (CLI/server: running instance only)
/memory              Show current memory strategy
/memory lambda       Switch to λ-Memory (decay + persistence)
/memory echo         Switch to Echo Memory (context window only)
/keys                List configured providers
/addkey              Securely add an API key
/usage               Token usage and cost summary
/mcp                 List connected MCP servers
/mcp add <name> <cmd>  Connect a new MCP server
/eigentune           Self-tuning status and control
/login <service>     Browser login flow (service or custom URL)
/timelimit           Show current task time limit
/timelimit <secs>    Set hive task time limit (e.g. /timelimit 3600)
```

---


## Conversation history

Tem keeps the full native conversation in the selected profile's canonical workspace, shown by CLI at startup. The default remains `~/.temm1e/workspace`; launching from another directory does not select a different project. Each local interface has its own conversation identity. Server conversations include channel identity and chat identity; admitted group members keep sharing the group history. Server management commands require the existing Admin role.

- `/history-import` previews the preserved legacy chat and shows a confirmation command. Import copies it into an empty conversation, leaves the source untouched and never replays tools. Old chats have no reliable workspace identity, so Tem does not guess where they belong.
- `/session-new` starts a fresh conversation epoch and preserves the old history and execution evidence.
- `/session-recover` inspects a turn interrupted by process death. Its confirmation restores the saved evidence, including unknown tool outcomes, without executing tools or resending replies. Inspect external state before retrying an uncertain effect.
- TUI `/clear` clears the display only. It does not erase or reset agent memory.

A second local process cannot dispatch another turn into the same conversation while it is locked. Invalid or oversized history produces an explicit error; it is not silently replaced with an empty chat. The current active-history limit is 32 MiB / 100,000 messages. This is not a total profile disk quota or a multi-host lock protocol. Server workers use the same store and no longer delete messages beyond the last 200. Successful final replies are saved before delivery on the modernization branch.

## Saved replies and interrupted delivery

- `/delivery-status` lists saved replies and whether a send was attempted. “Channel accepted it” does not prove that you saw it.
- `/delivery-show <id> [offset]` displays a bounded review copy without changing its delivery state.
- `/delivery-resume <id>` sends a never-attempted reply from the current conversation, without rerunning the model or tools. Replies from earlier conversations can be reviewed but cannot be resumed.
- `/delivery-ack <id>` records your statement that you received or read the reply. Use it only after reviewing that reply; it does not certify the overall task.

When a process dies during a send, Tem cannot know whether the destination received all of it. It preserves the uncertainty and does not automatically send another copy. Review the saved reply, then acknowledge receipt or start a new conversation. `/session-new` preserves the old evidence. These commands use the same local-owner/server-Admin authorization as conversation recovery. Interim notices and tool-initiated sends do not yet have this final-reply protection.

Logs now live in the selected profile's `logs` directory on Windows too. Existing logs under the older Windows LocalAppData location are left untouched.

## Model selection and execution evidence

CLI/server `/model` reports the actual running connection. `/model <id>` changes that instance without modifying saved defaults or making a validation request; account access is checked on the next real request. A server instance shares its selected model across chats and rejects replacement while busy. TUI configuration has a separate persistence flow. See the [upgrade guide](modernization/UPGRADING.md).

`/goal-status` inspects recorded execution state; `/goal-assessment` shows saved checks and evidence for the current scope. Returned prose alone does not mark the objective complete. These inspection commands do not call the model.
