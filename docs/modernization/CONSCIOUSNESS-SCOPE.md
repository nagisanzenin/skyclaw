# Scoped observer trajectory

Checkpoint63, September9,2026.

## Reproduced problem

An observer engine attached to a shared runtime kept one global set of notes and one deferred insight. Actual runtime acceptance showed PRIVATE_OBSERVER_NOTE and PRIVATE_OBSERVER_INSIGHT from the first user in the second user's pre-observer prompt, even with a different user identity. The failing before log is implementation-consciousness-scope-before.log; it stops at the first user-boundary failure. No claim that every boundary was individually run before the fix.

## Implementation contract

Runtime supplies SessionContext to the private immutable observer binding. The scope key is a structured JSON tuple of version, canonical workspace path, channel, chat, user, role and session ID/epoch. Empty or >4096byte identities, unresolved workspace, serialization failure or >32KiB key skip optional observation. Never fall back to a global or another user's notes. This changes no user role or tool permission.

Each engine retains at most64scope states in least-recently-used order. Existing scope access reuses its state and updates recency; new scope creation evicts the least-recently-used idle state when full. A turn's Arc view pins its trajectory, so a state with an active view cannot be evicted. If all64states are active, a new scope receives no optional observer for that turn. No provider call is made merely to allocate a scope.

Both observer phases use the same captured state/provider/model/owner/limits. Resource rebinding within the same scope preserves trajectory without changing any earlier view's model. Different role or conversation epoch starts separate state. Explicit new conversation therefore cannot inherit the previous conversation's private observer notes. Checkpoint62's64notes x8KiB bound applies per scope; the approximate maximum retained note text is32MiB per engine, plus insight/key/collection overhead and separate standalone state. This is not a global process-memory quota.

Direct standalone ConsciousnessEngine observation/session methods continue to use its own constructor state; caller-managed standalone scoping remains the host's responsibility. No old on-disk notes need migration because these notes were already ephemeral. Runtime reconstruction can still drop all engine state and is a separate composition issue.

## Validation

Actual runtime6boundary matrix changes only user, chat, channel, role, workspace or epoch per case. All pass with6captured requests each; second pre has no private note/insight. Same-scope two-turn continuity, correct model/owner, failure/drop/disabled accounting from61still pass. Unit64pinned views reject a65th; releasing one permits idle eviction without disturbing the first scope; model rebinding preserves notes and original view model. Canonical equivalent paths share key; delimiter aliases, missing workspace and bad identities do not.

Full799agentunit+81integration pass;3existing ignored docs. Logs implementation-consciousness-scope-tests.log and agent-tests.log. CLI build passed52.49s(four existing conditional warnings); extended smoke normal/disabled/finite/small-output plus `/session-new` all pass with actual HTTP and trajectory assertions. Scoped agent/all-feature/all-target lint passed21.25s and cleaned3.0GiB; full workspace gate deferred to final CI under quota-conscious validation, not claimed for this revision. No real user profile or paid API used.

## Remaining boundaries

Scope relies on truthful SessionContext identity and conversation epoch supplied by entrypoints; this is not independent authentication or an OS sandbox. Same-scope concurrent turns still share trajectory and can consume the one deferred insight; no serialized session execution/lease protocol is added here. Idle eviction discards ephemeral observer notes intentionally; raw chat and durable goals are unaffected. Configuration modes/confidence/intervention limits, calibrated advisory efficacy, monetary knownness and restart persistence are separate work. Scoped advice is not authoritative proof or permission.
