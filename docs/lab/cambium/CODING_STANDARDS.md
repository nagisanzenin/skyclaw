# Cambium: Coding Standards

> **Status:** Mandatory rules for all cambiumn code  
> **Date:** 2026-04-08  
> **Purpose:** Every line of code produced by cambium must comply with these standards. The verification pipeline checks these mechanically.

---

## Language and Toolchain

- **Edition:** Rust 2021
- **Minimum Rust version:** 1.91.1
- **Profile:** `panic = "unwind"` in release. NEVER change to `"abort"`.
- **Formatter:** `cargo fmt --all` (rustfmt defaults)
- **Linter:** `cargo clippy --workspace --all-targets --all-features -- -D warnings` (zero warnings)

---

## Error Handling

- All fallible operations return `Result<T, Temm1eError>`.
- `Temm1eError` is defined in `crates/temm1e-core/src/types/error.rs`. Use the appropriate variant: `Config`, `Provider`, `Channel`, `Memory`, `Tool`, `FileTransfer`, etc.
- NEVER use `.unwrap()` or `.expect()` in non-test code. Use `?` propagation or explicit error mapping.
- NEVER use `panic!()` in library code.

---

## Async Traits

- Use `#[async_trait]` from the `async_trait` crate for all async trait definitions and implementations.
- Import: `use async_trait::async_trait;`

---

## Logging

- Use the `tracing` crate: `tracing::info!`, `tracing::debug!`, `tracing::error!`, `tracing::warn!`.
- Include structured fields: `tracing::info!(id = %entry.id, "Stored entry")`.
- NEVER log API keys or tokens at info level. Use debug level with masking.
- NEVER use `println!` or `eprintln!` in library code.

---

## Serialization

- Use `serde` with `#[derive(Serialize, Deserialize)]` for all data types.
- JSON via `serde_json`. TOML via `toml` for configuration.
- All config structs must support deserialization from TOML.

---

## Naming Conventions

- **Structs:** PascalCase with domain prefix. Examples: `TelegramChannel`, `AnthropicProvider`, `SqliteMemory`, `ShellTool`.
- **Traits:** Bare names without prefix. Examples: `Channel`, `Provider`, `Memory`, `Tool`.
- **Functions:** snake_case. Factory functions use the `create_*()` pattern.
- **Modules:** snake_case matching the primary type they contain.
- **Constants:** SCREAMING_SNAKE_CASE.

---

## Module Structure

- Each crate has a `lib.rs` that re-exports public types.
- Internal modules are private unless they expose types needed by other crates.
- Feature-flagged modules use `#[cfg(feature = "...")]` on both `mod` and `pub use` declarations.

---

## Factory Pattern

- Each crate that provides multiple implementations MUST expose a `create_*()` factory function.
- The factory dispatches by name string and returns `Result<Box<dyn Trait>, Temm1eError>`.
- Examples: `create_provider()`, `create_memory_backend()`, `create_filestore()`, `create_observable()`.

---

## Feature Flags

- Platform-specific channels (Telegram, Discord, WhatsApp, Slack) and tools (browser, desktop-control) are behind Cargo feature flags.
- NEVER import their SDKs unconditionally.
- Conditional compilation uses `#[cfg(feature = "...")]` at both the module declaration and the re-export.

---

## Testing

- Unit tests go in a `#[cfg(test)] mod tests` block at the bottom of each file.
- Use `#[tokio::test]` for async tests.
- Every new public function MUST have at least one test.
- SQLite tests use in-memory databases: `SqliteMemory::new("sqlite::memory:")`.
- File-based tests use `tempfile::tempdir()` for temporary directories.
- Use `temm1e-test-utils` for mock providers, channels, memory, and tools.
- Tests MUST NOT hit real external APIs. Use mocks or recorded responses.

---

## Safety

- **No unsafe blocks.** Self-grown code MUST NOT contain `unsafe` in any form.
- **UTF-8 safety:** NEVER use `&text[..N]` on any string that could contain user input. Always use `char_indices()` to find safe byte boundaries.
- **No path traversal:** Sanitize file names by stripping directory components.
- **Credential isolation:** Tools must declare resource needs in `ToolDeclarations`. The sandbox enforcer validates these.
- **Empty allowlists deny all users.** Match on numeric user IDs only, never usernames.

---

## Dependencies

- **No new external dependencies without approval.** Self-grow MUST NOT add new entries to `[workspace.dependencies]` in the root `Cargo.toml`. This is a Level 0 file.
- Use only dependencies already declared in the workspace.
- If a new dependency is genuinely needed, cambium must propose it as a Level 1 change (approval required).

---

## Architecture Rules

1. **Traits in core, implementations in crates.** All shared traits are defined in `temm1e-core/src/traits/`. Implementations go in their respective crates.
2. **No cross-implementation dependencies.** Leaf crates (providers, channels, tools, memory backends) MUST NOT depend on each other. Shared types live in `temm1e-core`.
3. **Provider-agnostic principle.** NEVER shape the system to specifically adapt to one LLM provider. All features, tool names, schemas, and behaviors must work across all providers. Provider-specific handling belongs in the provider adapter layer.
4. **No stub code.** Every function must be fully wired and usable. Placeholder implementations that return `todo!()` or `unimplemented!()` are forbidden.
5. **No keyword matching for semantic decisions.** NEVER use keyword/string matching where semantic understanding is needed. Use LLM calls for classification.
6. **No hardcoded max_tokens on LLM output.** Always set `None` for output token limits. Input budget management is handled by the context system.

---

## Cross-Platform

- All code MUST work on Windows, macOS, and Linux.
- Use OS-agnostic solutions by default.
- If platform-specific code is unavoidable, use `#[cfg(unix)]` / `#[cfg(windows)]` with implementations for both.
- NEVER use Unix-only APIs (exec, signals, /tmp hardcoded paths) without a Windows fallback.

---

## Compilation Gate

Every cambium change must pass all four gates before it can be committed:

```bash
cargo check --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo test --workspace
```

All four must exit with status 0. Any failure means the change is rejected. No exceptions.
