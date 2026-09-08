# Isolated application data profiles

Set `TEMM1E_DATA_DIR` to a dedicated directory before launching Temm1e. With no override, existing users continue to use `~/.temm1e`.

```bash
TEMM1E_DATA_DIR=/absolute/path/to/test-profile temm1e tui
TEMM1E_DATA_DIR=/absolute/path/to/test-profile temm1e chat
TEMM1E_DATA_DIR=/absolute/path/to/test-profile temm1e start
```

This redirects application-owned credentials, OAuth tokens, user configuration, browser sessions, allowlists, memory locations selected by the runtime, Witness, Cambium, Perpetuum, skills and specialist configuration. Relative profile paths resolve against the process working directory. Explicit configuration paths and explicit database/workspace overrides still take precedence.

This is data separation, not an operating-system sandbox. It does not change `HOME`, the selected workspace, the shell's access, Chrome's own profile, explicit user paths, or system/workspace configuration discovery. Use isolated workspaces and matched explicit configuration for A/B. Do not reuse a real user's data directory for destructive migration fixtures.

Credential and OAuth writes use atomic replacement with owner-only temporary files on Unix. Windows inherits the parent directory's ACL; Windows credential isolation still requires validation. Cross-process OAuth refresh coordination remains a separate unfinished item.

Validation uses injected path arguments for unit tests (no process-global environment races) and an external environment override for process-level regression runs. An override must be carried into every child Tem process. Legacy data remains readable at the default location; no automatic data migration is performed by this setting.
