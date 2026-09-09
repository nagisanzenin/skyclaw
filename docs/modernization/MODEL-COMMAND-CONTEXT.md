# Model commands use the effective provider

Checkpoint 59 fixes `/addmodel`, `/removemodel` and `/listmodels` ignoring a working `config.toml` connection. Previously each command independently read the credentials store's active provider. A config-only runtime could report no provider, and an unrelated saved active account could receive the wrong scoped model update.

## Shared selection

Both CLI and server command sites now pass a short `ModelCommandContext` containing provider, model and whether a runtime exists. A live runtime's actual provider/model wins. When no runtime exists, the same core connection resolver used at startup selects the explicit configuration or a coherent saved connection, so offline metadata editing remains possible. The helper does not call a provider or expose keys/endpoint credentials in the context.

Server workers capture configuration for this fallback, snapshot the runtime identity under its read lock, and release that lock before model-file writes or message delivery. Model add/remove use the supplied provider scope. They retain the existing owner command gate and the locked atomic storage contract from58.

`/listmodels` still includes saved providers for browsing and adds the effective provider when it is absent from that file. It uses the actual current model for an active runtime. Offline selection is marked configured; inactive remembered model choices are marked saved. Saved metadata does not masquerade as the current runtime. The menu uses “Built-in” for supplied models. This does not establish account entitlements or live model availability, and provider-name/model storage remains distinct from endpoint/account identity.

## Validation

The actual pre-repair CLI failure is retained in `implementation-custom-model-storage-cli-v2.log`: it announced a config-only connection to openai, then `/addmodel` reported no active provider. The root binary's79unit tests pass after repair (`implementation-model-command-context-root-tests-v2.log`). Minimal CLI+TUI build passed22.90s with four existing feature-conditional unused-mut warnings.

Four actual Unix CLI scenarios pass:

- A saved route configured through `proxy`.
- A working config-only connection with no credentials file.
- A config-only connection while an unrelated saved provider is marked active.
- Offline explicit configuration, with no initialized runtime and an unrelated saved active provider.

Each exercises add/list/remove, current/configured markers, preservation of other provider entries, an independent writer lock, malformed-file preservation and0600mode. All make zero provider requests. In both foreign-saved cases the credentials file remains byte-identical. Logs are `implementation-model-command-context-cli-{saved,config,foreign,offline}.log`; the script is `scripts/custom_model_storage_smoke.py` with the corresponding flags. Server uses the same selection and handlers but no live channel messages were sent; do not claim these CLI runs are live server/channel acceptance.

Full workspace/all-feature/all-target clippy passed1m47s and cleaned2.5GiB (`implementation-model-command-context-workspace-clippy.log`); the existing dependency future-compatibility notice remains. CLI chat still lacks interception for the advertised `/model` switch command; that separate issue needs coherent model/resource switching, not forwarding a configuration instruction to the LLM or re-reading unrelated credentials. No version bump or release readiness is claimed.
