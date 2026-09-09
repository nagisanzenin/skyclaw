# Bounded Consciousness observation

Checkpoint62, September9,2026.

## Vision and reproduced failures

The observer remains optional trajectory advice for the main agent. The creator-derived `tems_lab/consciousness/IMPLEMENTATION.md` describes brief observation and bounded intervention; enhancement proposals contain hypotheses about benefit, not measured acceptance criteria or authorization for a model to acquire new control. This change preserves existing activation/defaults and adds resource bounds.

Before tests on the actual engine prove that a transport response marked length-truncated was injected as an instruction, and a user observation larger than64KiB reached the provider and consumed a previous insight. Both fail before the repair and pass afterward. The original log is retained as implementation-consciousness-bounds-before.log.

## Implemented contract

1. Constructor and immutable runtime binding capture registry context/output limits. Runtime also supplies its effective configured input limit. Output is min(1024, declared model output, half model window); zero capacity skips before provider admission.
2. Pre user/category/difficulty bytes are summed with saturation and limited to64KiB before formatting. Post previews/category/difficulty/tool names/results have the same aggregate bound; each tool/result list has at most128entries. Oversized inputs skip optional observation rather than truncating the user's request. The raw conversation remains unchanged.
3. The complete serialized CompletionRequest must fit64KiB. The existing final context estimator reserves output and10%input margin, with the captured model window and configured input limit. This is an estimate of normalized input, not an exact provider tokenizer or HTTP-envelope guarantee.
4. Completion is wrapped in30seconds of Tokio timeout. The owning MeteredProvider is inside the timeout, so cancellation records unavailable usage once. Rejected admission calls no provider; successful transport usage survives rejection of the answer. This is a per-call deadline, not a total turn deadline or a USD reservation.
5. Text fragments total at most8KiB. Tool/image/result content and explicit length/max_tokens/max_output_tokens/incomplete finish reasons(case-insensitive) are unusable advice. Opaque provider replay state is ignored. Truncated text never enters notes or the main-agent prompt; successful response usage still folds into the return value. Native Gemini MAX_TOKENS is covered.
6. The existing notes retain at most64entries, each at most8KiB. Long notes have a UTF-8-safe ` [excerpt]` marker; oldest entries are evicted. Pre observation clones only the latest5entries instead of cloning the entire history. The single deferred insight is also bounded by answer validation. Reset clears this bounded state.

## Validation

Full797agent unit+80integration tests pass, with3existing ignored docs. The old2dummy structure tests were replaced by5behavior tests. They cover actual truncated response/usage, overlarge input/no call/prior insight, small output capacity/zero allowance/context rejection, Unicode note retention, nontext/oversized/native truncation, and a paused-clock30second timeout recording1unknown attempt. Focused final5tests pass after native finish-reason review.

The first expanded test build omitted ToolUse.thought_signature in a new fixture; it is repaired with None, and the failing log is retained. No production assertion was weakened. Logs: implementation-consciousness-bounds-agent-tests.log(fixture failure), agent-tests-v2.log(full pass), final-tests.log.

Minimal CLI+TUI build passed48.26s(four existing conditional warnings). Actual CLI normal6calls/disabled2foreground/finite1pre0foreground/small-output256 model6calls pass. Extended scripts/consciousness_resources_smoke.py verifies actual HTTP max_tokens for both observer phases and retained trajectory; implementation-consciousness-bounds-cli.log. Full workspace/all-feature/all-target lint passed1m52s and cleaned4.0GiB; existing dependency future-compat notice retained. No live paid provider used.

## Boundaries retained

The bounded notes remain engine-wide, not principal-scoped or durable. Rebinding preserves notes; rebuilding a fresh engine can reset them. A failed context fit or transport call may consume the deferred insight as before; only initial oversized-input rejection preserves it without entering observation. Model classification/activation mode, confidence and max-intervention settings require a separate coherent policy; this checkpoint does not pretend to implement them. Legacy scalar observer cost prompts still cannot express all unavailable/subscription knownness. Returned TurnUsage retains successful responses while owner accounting also includes failed/cancelled attempts. Provider-side generation can continue after local cancellation; internal retries/account quota are not measured here. Bounded advice is still model-authored text, not trusted execution evidence or independent authority.
