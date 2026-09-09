---
id: bp_prowl_login
name: Web Login Flow
trigger_patterns: ['log into a website', 'sign in to a service']
semantic_tags: ["web", "login", "authenticate", "sign in", "account"]
task_signature: "log into {service}"
success_count: 0
failure_count: 0
---
## Objective
Authenticate to a web service for the user.

## Phases

### Phase 1: Check existing session (independent)
1. `browser(action="restore_web_session", service="{service}")` — try stored session
2. Verify the intended account with positive page evidence; a missing login prompt alone does not establish authentication. Stop only after verification.

### Phase 2: Vault credentials (depends: Phase 1, only if session invalid)
1. `browser(action="navigate", url="{service_login_url}")`
2. `browser(action="authenticate", service="{service}")` — vault injection
3. If vault has credentials, they are injected automatically
4. `browser(action="observe")` — verify authenticated state

### Phase 3: OTK capture (depends: Phase 2, only if no vault credentials)
1. Ask the user to invoke `/login {service}` in a supported messaging channel. The browser tool has no `method="otk"` option.
2. The host command supplies the interactive login link; do not invent a link or request a password in chat.
3. Wait for the user to complete the interactive flow. If this channel does not support `/login`, explain that limitation.
4. Restore the captured session and verify the intended account from positive page evidence.

## Failure Recovery
- CAPTCHA detected: send screenshot to user, ask them to solve via OTK session
- 2FA required: escalate to OTK session (user handles 2FA directly)
- Wrong credentials: notify user, offer to update via `/addcred {service}`
- Session expired mid-task: re-run from Phase 1

## Verification
- Accessibility tree shows authenticated state (no "Sign In" / "Log In" buttons)
- User's account info visible in tree (username, avatar, dashboard elements)
