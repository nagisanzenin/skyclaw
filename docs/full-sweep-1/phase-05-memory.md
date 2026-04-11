# Phase 5: Memory Backend Durability

## SWEEP-501 — SQLite main memory backend: no WAL mode, no busy_timeout

**Phase:** 5.1 — Write Durability

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | Adding 2 PRAGMAs after pool creation — zero behavior change |
| Runchanged | 25% | Under concurrent multi-channel load, SQLITE_BUSY errors on reads |
| **RC** | **25.0** | |
| Agentic Core | **DIRECT** | Main conversation history store, used by agent for context |
| Blast Radius | GLOBAL | All users on all channels |
| Reversibility | DEPLOY | |
| Data Safety | WRITE | Operations fail under contention |
| Concurrency | HIGH | Every message reads/writes this database |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | In-memory tests don't exercise concurrency |
| User Visibility | ERROR | Agent fails to retrieve conversation history |
| Fix Complexity | TRIVIAL | `PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;` |
| Cross-Platform | UNIVERSAL | SQLite WAL works on all platforms |
| Incident History | THEORETICAL | Other stores (anima, perpetuum) already have WAL — this one was missed |
| Recovery Path | SELF_HEALING | Individual operations fail, but no data corruption |

**Priority Score:** (25 x 7 x 5 x 1) / (0 x 1 + 1) = 875 / 1 = **875** -> **P0 EMERGENCY**

**Proposed fix:** After `SqlitePool::connect_with(opts).await?`, add:
```rust
sqlx::query("PRAGMA journal_mode=WAL").execute(&pool).await?;
sqlx::query("PRAGMA busy_timeout=5000").execute(&pool).await?;
```

---

## SWEEP-502 — Markdown backend: no file locking on concurrent writes

**Phase:** 5.1

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | |
| Runchanged | 15% | Concurrent writes lose data |
| **RC** | **15.0** | |
| Agentic Core | INDIRECT | Markdown is secondary to SQLite |
| Blast Radius | ISOLATED | Only markdown logs |
| Reversibility | DEPLOY | |
| Data Safety | WRITE | Silent data loss |
| Concurrency | HIGH | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | |
| Fix Complexity | MODERATE | Use `OpenOptions::append(true)` or file locking |
| Cross-Platform | NEEDS_VERIFY | File locking behavior differs |
| Incident History | THEORETICAL | |
| Recovery Path | NONE | Lost entries are unrecoverable |

**Priority Score:** (15 x 1 x 1 x 10) / (0 x 2 + 1) = 150 -> P1

---

## SWEEP-503 — Failover search uses different semantics than primary

**Phase:** 5.2

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | |
| Runchanged | 5% | Only during primary failure |
| **RC** | **5.0** | |
| Agentic Core | INDIRECT | Search results feed into agent context |
| Blast Radius | ISOLATED | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | LOW | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | PARTIAL | |
| User Visibility | DEGRADED | Different search quality |
| Fix Complexity | MODERATE | Mirror word-split AND matching |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | SELF_HEALING | Primary recovery restores normal search |

**Priority Score:** (5 x 1 x 3 x 1) / (0 x 2 + 1) = 15 -> P2

---

## SWEEP-504 — Lambda store: no transaction around main table + FTS5 updates

**Phase:** 5.1

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | |
| Runchanged | 3% | Only on crash between two writes |
| **RC** | **3.0** | |
| Agentic Core | INDIRECT | Lambda memory feeds into context |
| Blast Radius | ISOLATED | |
| Reversibility | DEPLOY | |
| Data Safety | WRITE | FTS5 index becomes inconsistent |
| Concurrency | LOW | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | DEGRADED | Some entries not searchable |
| Fix Complexity | MODERATE | Wrap in `BEGIN/COMMIT` |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | MANUAL | FTS5 rebuild required |

**Priority Score:** (3 x 1 x 3 x 5) / (0 x 2 + 1) = 45 -> P2
