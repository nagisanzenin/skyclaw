# Engram numeric integrity

Checkpoint42 repairs numeric edge cases without changing the documented defaults or user-pin priority. It is not a claim that all Engram design features are wired.

## Reproduced failures

The previous scorer propagated NaN from seed, could produce NaN from EMA infinity multiplied by zero, accepted reversed promotion/demotion thresholds, and used unchecked `used + cost` in the permanent-memory packer. New pre-change tests fail on NaN, inverted thresholds and a real debug overflow (`implementation-engram-numeric-before-tests.log`). A separate loader test demonstrates that TOML `p_max_frac = nan` was accepted (`implementation-engram-numeric-before-config.log`).

## Implemented contract

- Nonfinite seed/current importance becomes zero. A nonfinite new judgment or EMA weight does not change a valid current importance. Finite EMA inputs are clamped before blending, so extreme out-of-contract values cannot overflow the intermediate expression. Valid inputs retain the same convex-combination formula.
- Effective importance clamps the stored score before applying the exponential. Nonpositive/nonfinite tau gives zero for unpinned facts; user pins always return5, including with invalid numeric inputs. Clock-skew saturation remains. This does not mutate or delete stored facts.
- Invalid tier input or threshold ordering returns Active instead of authorizing promotion/archival from that invalid tier decision. Permanent visibility rejects nonfinite scores and invalid thresholds while retaining user pins. These are narrow scoring outcomes, not evidence of factual truth.
- The packer filters nonfinite/negative relevance and compares cost against the remaining budget before adding. Extreme usize costs cannot wrap, and equal ordinary finite scores retain source order. It remains a descending-relevance greedy algorithm, not an optimal knapsack solver. The token estimator remains approximate.
- TOML and YAML profile loading call EngramConfig::validate after parsing. Fraction/eta must be finite in[0,1], thresholds finite in[0,5] with down<=up, and tau finite>0. Errors name the offending property and stop startup before provider creation. Disabled Engram still requires valid numeric configuration. Direct embedders constructing config without the loader should explicitly call this validator; pure scorer guards also apply.

Configuration comments now distinguish the actual automatic-memory switch from the design's unimplemented MEMORY.md fallback. The only scheduled curator modes remain substantive/off; every:N/session-end are design work, not advertised as operating implementations.

## Remaining vision-to-runtime gaps

`ema_update` and `tier` are currently library functions; production permanent-block construction uses effective importance, pin-aware visibility and packing. The tool upsert replaces importance, and the automatic curator currently assigns4.0 rather than performing a judged EMA update. The eta field is therefore not evidence of active reinforcement. A scoped transactional rejudgment API with provenance, contradiction handling and configured eta must be wired to tool/curator updates before claiming that lifecycle.

The design's hysteresis is approximated in the active read path by pin state, not a persisted tier transition. Curator eligibility still uses greater-than40bytes, and cadence/session-end, MEMORY.md fallback, scoped fact-recall/touch behavior and complete deletion/retention acceptance remain open. Preserve user pins and existing data when implementing those pieces; changing host/principal defaults remains outside this numeric checkpoint.

## Validation

Validation: pre-change tests reproduced3scoring/overflow failures and separately a loader accepting NaN. After repair791agent tests and261core tests pass(one ignored), including TOML/YAML invalid numeric policy. Actual CLI rejects four invalid fields with zero provider requests and byte-identical seeded memory; valid boundary configuration makes one successful foreground request. Logs `implementation-engram-numeric-{before-tests,before-config,after-tests,cli-smoke}.log`. Full workspace/all-feature/all-target clippy passed1m48s and cleaned2.5GiB (`implementation-engram-numeric-workspace-clippy.log`); existing proc-macro-error2 future-compatibility notice remains. No Cargo is active; no paid calls.
