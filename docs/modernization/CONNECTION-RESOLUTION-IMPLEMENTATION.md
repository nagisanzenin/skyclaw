# Shared connection resolution — checkpoint 39

## Actual defect

Server and CLI selected a provider/key/model from explicit configuration, then independently loaded the saved active provider's key pool and preferred its endpoint. This could redirect a configured connection or use another provider's credentials. TUI had the same defect until checkpoint35, but its repair was a separate implementation. Root startup also ignored configurations containing only a key pool, and treated saved keyless Codex as missing API credentials.

## Shared contract

`core/config/connection.rs` now resolves configuration or one parsed saved snapshot into one ProviderConfig. Explicit configured keys retain their provider, endpoint, order/pool and model. Saved fallback takes those fields together from one record. Default model selection is unchanged. Explicit and saved Codex can carry no API key; existing token-store authentication handles that route when the feature is available. This does not create/validate a subscription or prove model entitlement.

Root Start/Chat consume the resolved snapshot without a later saved-key/endpoint lookup. TUI delegates to the same resolver and keeps its existing live snapshot for switching. Extra headers may contain credentials, so saved/onboarded routes inherit them only when provider/endpoint identity matches. Explicit configuration with the historical default Anthropic name retains its own headers. Config-owned TUI model selection, including explicit keyless Codex, remains session-only unless the configuration itself is changed.

Setup messages now say configured/received rather than claiming an API key was verified when a custom-endpoint probe was skipped. Other probe failures still follow the existing setup policy; allowing configuration after a transient/parameter error is not authentication proof. Validation debug logs no longer print raw endpoint URLs or assert a failed probe proves validity. Provider/model errors are described as connection setup errors rather than necessarily invalid keys.

## Real acceptance

Core260tests (one ignored), TUI50tests and root79minimal-feature tests pass. Tests cover conflicting saved data, key pools without a scalar key, keyless Codex, scoped headers and legacy behavior. Three actual entrypoint fixtures pass:

- `cli_budget_smoke.py` now includes a different saved provider at a second HTTP endpoint. All requests use the configured endpoint/key, none reach the saved trap, and budget continuity still rejects the exhausted replacement.
- `tui_pty_smoke.py` retains its conflicting-endpoint, model-switch, budget, history, Unicode, resize and exact terminal-restoration checks.
- `connection_startup_smoke.py` starts the real server with a configured key pool and unrelated saved invalid endpoint. Its actual runtime/dashboard reports the selected provider/model, saved data stays unchanged, startup makes zero provider calls and SIGTERM exits0. This proves factory selection/readiness, not live authentication or message delivery through a messaging service.

Logs and final lint status are in IMPLEMENTATION-STATUS. No paid provider call was made.

## Remaining boundaries

The saved `active` field still identifies a provider name, not a unique endpoint/account ID; legacy fallback uses its first matching record. Root post-start model/credential reload and setup paths still have separate reconstruction code and require the same snapshot contract. A unified runtime factory must preserve non-budget feature settings and bound service identities too. In particular, explicit Engram configuration is currently applied to only two root startup paths and is absent from CLI/TUI/rebuilds; honoring disabled memory/curator settings consistently is the next concrete task. Account discovery, typed probe outcomes, all-feature rebuilding, durable goal/evidence and broad release A/B remain separate gates.
