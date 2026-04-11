# Phase 7: Tool Execution Safety

## SWEEP-701 — file_read: arbitrary path traversal (NO containment)

**Phase:** 7.1 — File Operations Safety

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | Adding workspace containment check is purely additive |
| Runchanged | 100% | Every agent invocation can read any file via `file_read` tool |
| **RC** | **100.0** | Maximum urgency |
| Agentic Core | **DIRECT** | Tool declarations processed by agent runtime |
| Blast Radius | **SYSTEM** | Any file accessible by TEMM1E process |
| Reversibility | **IRREVERSIBLE** | Exposed credentials cannot be un-exposed |
| Data Safety | **CREDENTIAL** | SSH keys, env vars, config files with API keys |
| Concurrency | HIGH | Any tool call |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | Agent reads files silently, sends contents in conversation |
| Fix Complexity | MODERATE | Canonicalize path, verify starts_with workspace |
| Cross-Platform | NEEDS_VERIFY | Path canonicalization differs |
| Incident History | NEVER | But exposure is permanent for any LLM with tool access |
| Recovery Path | **NONE** | Leaked credentials require rotation |

**Priority Score:** (100 x 10 x 1 x 10) / (0 x 2 + 1) = **10000** -> **P0 EMERGENCY**

**Proposed fix:** In `resolve_path()`, after resolving the path:
```rust
let canonical = std::fs::canonicalize(&resolved)?;
let workspace = std::fs::canonicalize(&self.workspace)?;
if !canonical.starts_with(&workspace) {
    return Err(Temm1eError::Tool(format!(
        "Path {} is outside workspace {}", resolved.display(), workspace.display()
    )));
}
```

---

## SWEEP-702 — file_write: arbitrary path traversal (NO containment)

**Phase:** 7.2

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | Same fix as SWEEP-701 |
| Runchanged | 100% | Agent can write to any path |
| **RC** | **100.0** | |
| Agentic Core | **DIRECT** | |
| Blast Radius | **SYSTEM** | Can overwrite system files, install backdoors |
| Reversibility | **IRREVERSIBLE** | System compromise |
| Data Safety | **CREDENTIAL** | Can write authorized_keys, crontabs |
| Concurrency | HIGH | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | |
| Fix Complexity | MODERATE | Same workspace containment as SWEEP-701 |
| Cross-Platform | NEEDS_VERIFY | |
| Incident History | NEVER | |
| Recovery Path | **NONE** | System compromise requires rebuild |

**Priority Score:** (100 x 10 x 1 x 10) / (0 x 2 + 1) = **10000** -> **P0 EMERGENCY**

---

## SWEEP-703 — Shell tool: no sandboxing, arbitrary command execution

**Phase:** 7.1 — Shell Tool Safety

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 30% | Adding sandbox enforcement is a significant change |
| Runchanged | 100% | Agent can execute any command |
| **RC** | **3.23** | High Rchange reduces urgency vs SWEEP-701/702 |
| Agentic Core | **DIRECT** | Tool execution path |
| Blast Radius | **SYSTEM** | Full system access |
| Reversibility | **IRREVERSIBLE** | |
| Data Safety | **CREDENTIAL** | |
| Concurrency | HIGH | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | |
| Fix Complexity | **COMPLEX** | Requires sandbox enforcement layer, ToolDeclarations checking |
| Cross-Platform | NEEDS_VERIFY | Sandboxing differs dramatically between platforms |
| Incident History | NEVER | |
| Recovery Path | **NONE** | |

**Priority Score:** (100 x 10 x 1 x 10) / (30 x 4 + 1) = 10000 / 121 = **82.6** -> P1

**Note:** Lower priority than SWEEP-701/702 because the fix is complex and the shell tool is an intentional power feature. But the lack of ANY guard (not even command filtering) is a gap.

---

## SWEEP-704 — String::truncate() UTF-8 panic in shell/file/web_fetch tools

**Phase:** 7.4

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | Safe truncation is strictly better |
| Runchanged | 10% | Multi-byte output near 32KB boundary |
| **RC** | **10.0** | |
| Agentic Core | **DIRECT** | Tool output enters agent context |
| Blast Radius | ISOLATED | Single tool call |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | HIGH | Every tool call with large output |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | ERROR | Tool call returns error instead of result |
| Fix Complexity | TRIVIAL | `char_indices()` safe truncation |
| Cross-Platform | UNIVERSAL | |
| Incident History | **HAS_OCCURRED** | Same class as Vietnamese text crash |
| Recovery Path | SELF_HEALING | catch_unwind |

**Priority Score:** (10 x 1 x 5 x 1) / (0 x 1 + 1) = 50 -> P1

---

## SWEEP-705 — String::truncate() UTF-8 panic in Perpetuum self_work

**Phase:** 7.5

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | |
| Runchanged | 10% | |
| **RC** | **10.0** | |
| Agentic Core | INDIRECT | Perpetuum subsystem |
| Blast Radius | ISOLATED | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | LOW | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | |
| Fix Complexity | TRIVIAL | |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | SELF_HEALING | Perpetuum catch_unwind |

**Priority Score:** (10 x 1 x 1 x 1) / (0 x 1 + 1) = 10 -> P2

---

## SWEEP-706 — Credential scrubber missing common key patterns

**Phase:** 7.6

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | Adding regex patterns |
| Runchanged | 15% | AWS/Stripe/Slack/Azure credentials pass through |
| **RC** | **15.0** | |
| Agentic Core | INDIRECT | Scrubber runs before context injection |
| Blast Radius | ISOLATED | |
| Reversibility | DEPLOY | |
| Data Safety | **CREDENTIAL** | Unscrubbed credentials enter LLM context |
| Concurrency | HIGH | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | PARTIAL | Existing key types tested |
| User Visibility | SILENT | |
| Fix Complexity | TRIVIAL | Add regex patterns for AKIA, sk_live_, xoxb-, etc. |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | NONE | Leaked credentials require rotation |

**Priority Score:** (15 x 1 x 1 x 10) / (0 x 1 + 1) = 150 -> P1

---

## SWEEP-707 — BrowserPool::new() uses assert! for config validation

**Phase:** 7.7

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | Replace assert with Result |
| Runchanged | 2% | Only on misconfiguration |
| **RC** | **2.0** | |
| Agentic Core | NONE | |
| Blast Radius | GLOBAL | Process crash on startup |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | NONE | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | FATAL | |
| Fix Complexity | TRIVIAL | |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | RESTART | Fix config, restart |

**Priority Score:** (2 x 10 x 10 x 2) / (0 x 1 + 1) = 400 -> P1

---

## SWEEP-708 — BrowserPool get_page() uses assert! for slot bounds

**Phase:** 7.8

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | |
| Runchanged | 1% | Only on logic bug in slot management |
| **RC** | **1.0** | |
| Agentic Core | INDIRECT | |
| Blast Radius | GLOBAL | Panic (caught by unwind) |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | HIGH | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | ERROR | |
| Fix Complexity | TRIVIAL | |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | SELF_HEALING | |

**Priority Score:** (1 x 7 x 5 x 1) / (0 x 1 + 1) = 35 -> P1
