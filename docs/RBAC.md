# Role-Based Access Control (RBAC)

TEMM1E enforces role-based access control across all messaging channels. Every allowed user has a **role** that determines what commands they can run and what tools the agent can use on their behalf.

## Roles

| Role | Description |
|------|-------------|
| **Admin** | Full access to all commands, tools, and user management |
| **User** | Full agent chat with most tools, blocked from dangerous tools and system commands |

### Admin capabilities

- All slash commands (including `/addkey`, `/model`, `/provider`, `/reload`, `/reset`, etc.)
- All agent tools (including `shell`, `key_manage`, `invoke_core`, `desktop`)
- User management (`/allow`, `/revoke`, `/users`, `/add_admin`, `/remove_admin`)

### User capabilities

- Full agent chat — ask questions, get help, run tasks
- Most tools: `file_read`, `file_write`, `file_list`, `git`, `browser`, `web_fetch`, `send_message`, `send_file`, `memory_manage`, `use_skill`, and others
- Safe slash commands: `/help`, `/status`, `/usage`, `/mode`, `/memory`, `/quit`

### Blocked for User role

| Category | Blocked items | Reason |
|----------|--------------|--------|
| Tools | `shell` | Arbitrary command execution on host |
| Tools | `key_manage` | API credential management |
| Tools | `invoke_core` | TemDOS specialist cores (system access) |
| Tools | `desktop` | OS-level screen capture + input simulation |
| Commands | `/addkey`, `/removekey`, `/keys` | Credential management |
| Commands | `/model`, `/provider` | System configuration |
| Commands | `/allow`, `/revoke`, `/users` | User management |
| Commands | `/add_admin`, `/remove_admin` | Admin management |
| Commands | `/reload`, `/reset`, `/restart` | System lifecycle |

## How roles are assigned

### First user (auto-promotion)

The first user to message the bot on any channel is automatically promoted to **Admin** and becomes the **original owner**. This happens exactly once per channel.

### Adding users

Admins can add users with `/allow <user_id>`. New users get the **User** role by default.

### Promoting/demoting admins

```
/add_admin <user_id>     — Promote a user to admin (user must already be on allowlist)
/remove_admin <user_id>  — Demote an admin to regular user
```

The **original owner** (first user) cannot be demoted or removed. This is a safety invariant.

## Storage

Roles are stored per-channel in TOML files under `~/.temm1e/`:

| Channel | File |
|---------|------|
| Telegram | `~/.temm1e/allowlist.toml` |
| Discord | `~/.temm1e/discord_allowlist.toml` |
| Slack | `~/.temm1e/slack_allowlist.toml` |
| WhatsApp | `~/.temm1e/whatsapp_allowlist.toml` |
| CLI | No file (always Admin) |

### File format

```toml
admin = "123456789"              # Original owner (backward compat)
admins = ["123456789", "987654"] # All admin user IDs
users = ["123456789", "987654", "111222333"] # All allowed user IDs
```

- `admin` — the original owner. Always an admin. Cannot be removed.
- `admins` — all users with Admin role.
- `users` — all allowed users (admins + regular users). If you're in `users` but not in `admins`, you're a User.

### Backward compatibility

The format is backward-compatible with the legacy `AllowlistFile { admin, users }`. When a new binary reads an old file that lacks the `admins` field, it automatically migrates by seeding `admins` from `admin`. No manual migration needed.

## Enforcement layers

RBAC is enforced at three layers (defense in depth):

### Layer 1: Channel gate

`Channel::is_allowed(user_id)` — binary allow/deny at the messaging channel level. If you're not on the allowlist at all, your messages are silently rejected. This is unchanged from the original system.

### Layer 2: Command gate

Slash commands are checked against the user's role before dispatch. If a User tries to run `/addkey`, they get "You don't have permission to use this command." The check happens in `main.rs` at the top of the command dispatch chain.

### Layer 3: Tool gate

Agent tool access is filtered in two places:
1. **Runtime** (`runtime.rs`): Before sending a request to the LLM, blocked tools are filtered out of the tool definitions. The LLM never even sees dangerous tools for User-role sessions.
2. **Executor** (`executor.rs`): Defense-in-depth — before executing any tool, the executor checks the session role. If blocked, returns `PermissionDenied`.

## User identification

Each channel uses platform-specific numeric IDs:

| Channel | ID format | Example |
|---------|-----------|---------|
| Telegram | Numeric user ID | `123456789` |
| Discord | Snowflake ID | `987654321098765432` |
| Slack | User ID | `U12345678` |
| WhatsApp | Phone number | `1234567890` |
| CLI | Fixed | `local` (always Admin) |

Per security rule CA-04: only numeric IDs are matched, never usernames (which can be changed).

## Code reference

| Component | File | Purpose |
|-----------|------|---------|
| `Role` enum | `crates/temm1e-core/src/types/rbac.rs` | Role definition + permission logic |
| `RoleFile` struct | `crates/temm1e-core/src/types/rbac.rs` | On-disk storage format |
| `load_role_file()` | `crates/temm1e-core/src/types/rbac.rs` | Shared I/O for all channels |
| `Channel::get_role()` | `crates/temm1e-core/src/traits/channel.rs` | Per-channel role lookup |
| `SessionContext.role` | `crates/temm1e-core/src/types/session.rs` | Role propagated through session |
| Tool filtering | `crates/temm1e-agent/src/runtime.rs` | LLM never sees blocked tools |
| Tool execution gate | `crates/temm1e-agent/src/executor.rs` | Defense-in-depth check |
| Command gate | `src/main.rs` | Slash command permission check |

## Extending with new roles

To add a new role (e.g. `PowerUser`):

1. Add the variant to `Role` enum in `rbac.rs`
2. Add its `blocked_tools()` and `allowed_commands()` entries
3. Update `RoleFile::role_of()` to recognize the new role
4. Update the TOML format if needed (e.g. add a `power_users` field)
