# TUI modernization audit and implementation specification

Status: original audit/design, followed by implemented checkpoints in IMPLEMENTATION-STATUS. Connection identity and model-switch implementation: [checkpoint 35](TUI-CONNECTION-IMPLEMENTATION.md). Creator requested modernization; creator selected the compact transcript layout. Preserve Tem's palette, recognizable voice and keyboard-first operation. The current ratatui/TEA implementation is a useful foundation, not a reason for a framework rewrite.

## Observed baseline

`temm1e-tui/src/lib.rs` restores the terminal with a guard and provides onboarding, theme, command completion, mouse selection and clipboard workflows. `input/keybindings.rs` already supports multiline input, scrolling, interrupts and an activity toggle. `widgets/status_bar.rs` separates state, model/cost, and context/git into bounded regions. These should survive.

`agent_bridge.rs:609` processes inbound messages serially, passes a real interrupt flag, but passes `None` for pending-message steering and the cancellation-token argument. Do not claim interrupts do not exist; cancellation propagation to an in-flight process is the separate F08 problem. The bridge emits early/final response events and phase updates. No production emitter of `Event::StreamChunk` was found; a renderer and enum variant alone do not establish live token streaming.

`app.rs:402` reconstructs tool history from phase changes and tool names, with empty arguments and no stable operation IDs. A watch channel may coalesce intermediate phases. `AgentResponse` identifies early messages by zero usage rather than event kind; a valid zero-usage response can be misclassified. `views/chat.rs` assembles all transcript lines before slicing the viewport, creating work proportional to history per redraw. The 20/48-column status allocations constrain small terminals. Subscription quota/reset and compaction state are absent from these event contracts.

These are source findings. No native terminal visual/usability trial was performed; aesthetic quality and platform-specific rendering remain validation tasks, not measured conclusions.

## Proposed layout

Creator-selected direction: compact transcript with progressive disclosure. At 100×30:

```text
Tem  ·  temm1e / modernization                Session ▾

You  Audit the context manager.
Tem  I found a gap in the final token check.

  ✓ Read context.rs                         0.2s
  ▶ Run focused regression test             3.1s
    cargo test ...                 [expand output]

  Goal: context audit       2/4 criteria verified

> Add subscription cache accounting as well.
  Enter queue · Alt+Enter steer · Esc stop
Codex plan · selected model · quota unknown   Context 61%
```

This is a wireframe, not a rendered or usability-validated UI. Optional panels use the same view model; do not duplicate execution state.

On narrow terminals prioritize active state and composer; collapse account/context details into `/status`. On wide terminals optionally show goal/criteria, tool details or diff pane. Avoid permanent decorative blocks consuming transcript space. Keep a no-color/high-contrast mode and textual status labels independent of icons.

## P15: implementation work in order

Dependencies P02/P04/P05/P10/P12 for full runtime-backed behavior; layout and input can land earlier.

1. Extend core events with `event_id, sequence, session_id, turn_id, item_id` and explicit variants: TextDelta, ToolStarted, ToolOutput, ToolFinished, TurnFinished, PartialDelivered, CancellationRequested, Cancelled, QuotaChanged, ContextCompiled, CompactionStarted/Finished, VerificationChanged. Usage cannot determine event kind. Send events over a bounded channel with replay by sequence; coalesce text/telemetry only, never lose lifecycle transitions.
2. Replace bridge-local state inference with a reducer over these events. Tools key by operation ID; simultaneous calls with identical names remain distinct. On reconnect fetch a snapshot plus events after its sequence. Keep display state separate from durable execution state.
3. Wire actual provider text/tool streaming through the runtime. Preserve partial output on transport failure and label it incomplete. A spinner must not be presented as token streaming. Reconcile final content by item ID to avoid duplicated streamed paragraphs.
4. Composer: Enter queues a new turn when work is running; a visible steer action targets the active turn revision. Reject stale target with a clear retry affordance. Esc requests stop and shows Stopping until executor confirms cleanup; closing an overlay uses Esc first. Preserve existing keybindings where feasible; make new bindings configurable and provide terminal-compatible fallback when modified Enter is unavailable. Handle bracketed paste as one insertion, never implicit submission.
5. Transcript: virtualize by logical blocks; cache rendered lines by content revision, width and theme. Reflow only changed blocks on deltas, and invalidate on resize. Preserve viewport anchor when new output arrives if user scrolled up; offer Jump to latest. Bound in-memory rendered history while paging durable events.
6. Tool cards: collapsed name, target, status, duration; expanded redacted args, bounded output, exit code and evidence/diff links. Show Unknown outcome distinctly. Copy preserves original text; no secrets in activity details. Separate command exit success from goal verification.
7. Account panel: connection kind, selected model, plan label if known, quota windows/reset times and source timestamp, cache read/write usage when available. Unknown remains unknown. Paid API switch is explicit, never a hidden fallback. Device login needs timeout/cancel and headless fallback, not credentials pasted into the transcript.
8. Context panel: actual/estimated distinction, output reserve, compaction event, pinned goal/constraints and expandable source ranges. Display cache economics only with normalized evidence. Offer manual compact through the same service; do not clear history from a UI shortcut.
9. Session picker: workspace + branch + active goal + last activity, resume by durable ID; do not reuse a global `tui-tui` identifier across workspaces. Export a redacted transcript and evidence references; deletion has scope visible. Search history without blocking rendering.
10. Testing: ratatui TestBackend snapshots at 40×12, 80×24, 120×40; Unicode wide characters, combining marks and emoji; large paste; resize during stream; two same-name tools; failed tool followed by final prose; zero-token final; queued vs steered input; quota exhaustion; three compactions; crash/reconnect; terminal restored after panic. Benchmark 10,000-message history with at least 100 deltas/sec replay and record p95 event-to-render latency and memory. Proposed local target <100ms p95 under this fixture, to be calibrated on supported hardware. Run native PTY smoke on macOS/Linux/Windows and tmux before declaring parity.

Do not copy competitor branding. Competitor state/event contracts and session controls in [harness research](02-HARNESSES.md) motivate these behaviors; the layout is a Tem-specific design proposal.
