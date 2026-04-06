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

Each channel uses platform-specific numeric IDs. Per security rule CA-04: only numeric/stable IDs are matched, never usernames (which can be changed and would allow allowlist bypass).

| Channel | ID format | Example |
|---------|-----------|---------|
| Telegram | Numeric user ID | `123456789` |
| Discord | Snowflake ID | `987654321098765432` |
| Slack | User ID | `U12345678` |
| WhatsApp | Phone number (digits only) | `1234567890` |
| CLI | Fixed | `local` (always Admin) |

### How to find user IDs per platform

#### Telegram

**Option A — Use a bot:**
1. Search for `@userinfobot` or `@getmyid_bot` in Telegram
2. Start a chat with the bot and send any message
3. The bot replies with your numeric user ID (e.g. `123456789`)

**Option B — From the TEMM1E logs:**
1. Have the user send any message to your TEMM1E bot
2. Check the logs: `grep "user_id" /tmp/skyclaw.log`
3. The log line shows `user_id=123456789`

**Option C — Telegram API:**
1. Forward a message from the target user to `@JsonDumpBot`
2. Look for `"from": { "id": 123456789 }` in the JSON output

> Note: Telegram user IDs are permanent and never change, even if the user changes their username or display name.

#### Discord

**Option A — Enable Developer Mode:**
1. Open Discord Settings > Advanced > toggle **Developer Mode** on
2. Right-click on any user's name or avatar
3. Click **Copy User ID**
4. The ID is a numeric snowflake (e.g. `987654321098765432`)

**Option B — From the TEMM1E logs:**
1. Have the user send a message in a channel where TEMM1E is active
2. Check the logs for `user_id=987654321098765432`

**Option C — Discord slash command:**
1. Type `\@username` in any Discord chat (backslash + mention)
2. Discord shows `<@987654321098765432>` — the number is the user ID

> Note: Discord snowflake IDs are permanent. They encode the account creation timestamp.

#### Slack

**Option A — Profile view:**
1. Click on the user's name in any Slack channel
2. Click **View full profile**
3. Click the **more** (...) button
4. Click **Copy member ID**
5. The ID looks like `U12345678` or `U0A1B2C3D4`

**Option B — From the TEMM1E logs:**
1. Have the user send a message in a channel where TEMM1E is listening
2. Check the logs for `user_id=U12345678`

**Option C — Slack API:**
1. Go to `https://api.slack.com/methods/users.list/test`
2. Select your workspace and execute
3. Find the user in the response — their `id` field is the user ID

> Note: Slack user IDs start with `U` and are permanent per workspace.

#### WhatsApp

WhatsApp uses phone numbers as user identifiers. TEMM1E normalizes them to digits only.

**Finding the phone number:**
1. Open the contact in WhatsApp
2. Tap the contact name at the top to view their profile
3. The phone number is shown with country code (e.g. `+1 234 567 8900`)
4. Strip all non-digits for the allowlist: `12345678900`

**From the TEMM1E logs:**
1. Have the user send a message to your TEMM1E WhatsApp number
2. Check the logs for `user_id=12345678900`

> Note: Use the full international number without `+`, spaces, or dashes. TEMM1E's `normalize_phone()` handles this automatically for incoming messages.

#### CLI

The CLI channel always runs as **Admin** with a fixed user ID of `local`. No configuration needed — if you have terminal access to the machine running TEMM1E, you have full admin access.

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
