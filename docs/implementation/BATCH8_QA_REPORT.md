# Batch 8 — QA Report

**Date:** 2026-11-26

## Independence statement

**Independent external QA: NOT AVAILABLE.** M7 was an internal adversarial QA
subagent written by the same implementation team. It is not an external reviewer
and does not constitute independent acceptance.

## Workspace gates (actual)

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo build --workspace` | exit 0 |
| `cargo test --workspace` | **798 passed, 0 failed, 1 ignored** |
| Live-gated artifact test (`--ignored`) | passed (131,072 B, SHA verified) |

Logs: `.tmp/b8-*.log`.

## M7 internal QA matrix (210 scoped tests, 16 new)

| Scenario | Result |
|---|---|
| Shared world snapshot + distinct per-Core emotions across reopen | PASS |
| Cross-Core `personal.db` copy rejected (ownership/instance) | PASS |
| Continuity elapsed from fixed past; bounded lists; reopen | PASS |
| World+personal+conversation restart/recovery, no loss | PASS |
| Failing weather provider: no invented data, honest freshness | PASS |
| America/Chicago DST spring-forward/fall-back | PASS |
| Same world + different personality/emotion => different bounded prompts | PASS |
| Different wardrobe/emotion => different outfit/expression/voice; no leakage | PASS |
| Backup→restore all per-Core DBs; ownership/integrity; A≠B | PASS |
| Core identity unchanged across restart/restore | PASS |

## Live verification (Lead)

- Real Open-Meteo weather via `aegis world weather` on `America/Chicago`.
- Real Ollama turn used world/continuity/emotion context; assistant referenced
  "mid-afternoon… rainy autumn day in Chicago".
- Fresh-process continuity: elapsed 7 s, same session; second turn elapsed reset;
  no personal-update warning.
- Backup included core/memory/conversation/personal DBs; restore preserved
  continuity timestamps/session and core integrity `verified`.

## Defects found

**None** in product code during M7. (Earlier, the Lead found and fixed only a
pre-existing test-timing artifact in Batch 6; not a Batch 8 regression.)

## Unverified / blocked

- Artemis / multi-machine distribution (owner authorization unavailable).
- Automatic emotion progression from real experiences/relationships (API exists,
  runtime not wired).
- SAPI consuming voice projection; native GUI/avatar renderer.
- Live gameplay/FPS (unrelated to this batch).
- Long-run stability, concurrent-write backup safety, cross-machine owner.proof.
- Owner/human acceptance; independent external QA.

## Recommendation

**Internal scope: ACCEPT.** **Independent external QA: NOT AVAILABLE.**
**Production: NO-GO.** Batch 9 should wire event→emotion, IPC distribution, and
SAPI/UI consumption, then seek independent review and owner acceptance.
