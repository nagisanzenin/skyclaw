# TUI connection identity — checkpoint 35

## Problem and change

The old TUI startup selected a configured provider and key, but `spawn_agent` then read the saved active provider's key pool and endpoint without checking identity. It also ignored `AgentSetup.base_url`. A configured OpenAI connection could therefore be combined with saved Anthropic credentials/URL. Model switching repeated this lookup instead of retaining the live connection, rejected registered custom models for known providers, and could leave no working runtime after a failed replacement.

`connection.rs` now resolves one connection snapshot before provider construction. Explicit configuration wins as before; saved fallback reads one parsed credentials file. `AgentSetup` carries provider, endpoint, all selected keys and model. The bridge builds only from those fields and performs no further saved-key lookup. Extra headers can contain secrets, so they are inherited only from matching configured provider/endpoint identity. Saved Codex OAuth routes may have no API key; the existing token-store provider handles authentication when its feature is enabled. This does not create a token or prove subscription entitlement.

Onboarding retains a saved rotation pool only when provider, endpoint and the newly selected key all match one record. Otherwise it uses only the selected key. It never borrows another connection's keys. The public headless smoke example uses the same resolver as the interactive startup.

## Model switching

The live handle retains its setup. Switching clones it and changes the model/personality only; saved credentials cannot redirect the replacement. Registered custom models, explicit custom endpoints and OpenRouter can use IDs beyond menu suggestions. The old loop and background work must drain before replacement. A failed replacement attempts to recreate the previous setup; if that also fails the UI explicitly reports disconnection rather than claiming a successful switch.

Only a still-active, uniquely matching saved provider/endpoint/key record has its model updated. Other fields and key pools are retained. Config-owned connections switch for the current session and say so; changing a saved file would not override their configured model on restart. Save failures are reported as session-only. Model acceptance remains local construction until a real completion: a menu/registration entry is not live access verification.

## Evidence and limits

Pure regression tests cover conflicting configured/saved identities, mutable saved data after capture, multiple keys, wrong-provider headers, keyless Codex, ambiguous/changed selections and onboarding route/key matching. The Unix PTY fixture is extended to use a deliberately unrelated saved provider at a second local HTTP endpoint, send real input, switch to a custom model, send another turn, restart/history restore, resize and check exact terminal restoration. Results are recorded in IMPLEMENTATION-STATUS after execution; no actual account is used.

The credential file format still identifies `active` by provider name rather than a unique endpoint/account ID; legacy saved startup still selects its first matching record. Persistence is an atomic private replacement but does not provide global compare-and-swap against unrelated external writers. Provider construction is not transactional across all background initializers; rollback is best effort. Saved OAuth token validity and Windows native terminal behavior need separate acceptance. Checkpoint36 adds owning-session budget continuity through replacement and shares TemDOS/JIT/Perpetuum accounting; see SHARED-ACCOUNTING-IMPLEMENTATION.md for the separate acceptance evidence. This checkpoint does not claim any of those broader contracts complete.


Checkpoint39 moves startup resolution to `core/config/connection.rs`, reused by CLI/server/TUI. See [shared resolution](CONNECTION-RESOLUTION-IMPLEMENTATION.md) for actual entrypoint tests and remaining reload/factory boundaries.
