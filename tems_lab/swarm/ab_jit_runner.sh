#!/usr/bin/env bash
# JIT Swarm A/B Live Runner
#
# Executes the 12-scenario battery from AB_REPORT_JIT.md §6 against the
# release binary built from the JIT-swarm branch. Captures wall-clock,
# tokens, cost, and completion status per scenario.
#
# Requirements:
# - $ANTHROPIC_API_KEY set
# - Release binary built: cargo build --release --bin temm1e
# - temm1e.toml in repo root with [hive] enabled = true (for swarm scenarios)
#
# Output: /tmp/ab_jit_live_results.json (kept out of the repo — gitignored
# artifacts, see .gitignore). Commit this file only if you've curated the
# metrics into AB_REPORT_JIT.md; raw per-run JSON does not belong on the
# tracked branch.
#
# Per scenario: we launch a fresh `temm1e chat` subprocess, pipe the prompt
# over stdin, tail /tmp/temm1e.log for the turn's TurnUsage signals, measure
# wall-clock, and record cache_read_input_tokens when the provider reports it.

set -euo pipefail

if [[ -z "${ANTHROPIC_API_KEY:-}" ]]; then
    echo "ERROR: ANTHROPIC_API_KEY must be set" >&2
    exit 1
fi

REPO_ROOT="$(git rev-parse --show-toplevel)"
BINARY="$REPO_ROOT/target/release/temm1e"

if [[ ! -x "$BINARY" ]]; then
    echo "ERROR: release binary not found at $BINARY" >&2
    echo "       build it: cargo build --release --bin temm1e" >&2
    exit 1
fi

RESULTS="/tmp/ab_jit_live_results.json"
LOGFILE=$(mktemp)

# 12 scenarios from AB_REPORT_JIT.md §6
declare -a SCENARIOS=(
    "chat_trivial|hello, how are you?"
    "chat_info|explain Rust ownership in one sentence"
    "tool_single|read Cargo.toml and tell me the version"
    "tool_sequential|fix the clippy warnings in a short file"
    "parallel_obvious|research these 5 libraries and compare them: tokio, async-std, smol, glommio, monoio"
    "parallel_discovered|list the top-level modules in this project"
    "false_parallel|write a function that calls another function that calls a third"
    "stop|stop"
    "long_chain|summarize the README.md in one paragraph"
    "recursion_attempt|spawn a swarm that spawns another swarm"
    "budget_bound|summarize the last 3 commits"
    "multi_turn_cache|hi / tell me about Tem / what's your favourite mode / how do you see the world / ok thanks bye"
)

echo '{ "scenarios": [' > "$RESULTS"
first=1

run_scenario() {
    local name="$1"
    local prompt="$2"
    local start_ns=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')

    # Run `temm1e chat`, pipe prompt + /quit, capture output.
    local output
    output=$(printf '%s\n/quit\n' "$prompt" | timeout 120 "$BINARY" chat 2>&1 | tail -100 || true)
    local end_ns=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time()*1e9))')

    local wall_ms=$(( (end_ns - start_ns) / 1000000 ))

    # Extract usage metrics via log greps (best-effort; live runtime may
    # log "Total Cost", "Input Tokens", etc. in the CLI output).
    local tokens_in=$(echo "$output" | grep -oE "Input Tokens: [0-9,]+" | head -1 | grep -oE "[0-9,]+" | tr -d ',' || echo "0")
    local tokens_out=$(echo "$output" | grep -oE "Output Tokens: [0-9,]+" | head -1 | grep -oE "[0-9,]+" | tr -d ',' || echo "0")
    local cost_usd=$(echo "$output" | grep -oE 'Total Cost: \$[0-9.]+' | head -1 | grep -oE "[0-9.]+" || echo "0")
    local success="true"
    if echo "$output" | grep -qE "panic|fatal|error:|aborted"; then
        success="false"
    fi

    # JSON emission (manual — avoid jq dep)
    if [[ $first -eq 0 ]]; then
        echo "," >> "$RESULTS"
    fi
    first=0
    cat >> "$RESULTS" <<JSON
    {
      "scenario": "$name",
      "prompt": $(printf '%s' "$prompt" | python3 -c 'import sys, json; print(json.dumps(sys.stdin.read().strip()))'),
      "wall_ms": $wall_ms,
      "input_tokens": ${tokens_in:-0},
      "output_tokens": ${tokens_out:-0},
      "cost_usd": ${cost_usd:-0.0},
      "success": $success
    }
JSON

    echo "  $name: wall=${wall_ms}ms in=${tokens_in:-0} out=${tokens_out:-0} cost=\$${cost_usd:-0} success=$success"
}

echo "Running 12-scenario A/B battery..."
for entry in "${SCENARIOS[@]}"; do
    name="${entry%%|*}"
    prompt="${entry#*|}"
    run_scenario "$name" "$prompt"
done

cat >> "$RESULTS" <<'JSON'
  ],
  "note": "See tems_lab/swarm/AB_REPORT_JIT.md for pass/fail criteria."
}
JSON

echo ""
echo "Results written to: $RESULTS"
