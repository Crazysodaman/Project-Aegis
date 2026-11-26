# AEGIS FINAL — Performance & Reliability Report

**HEAD:** `2bd2513`

## Evidence

- Full workspace: **942 passed, 0 failed, 1 ignored** (live artifact run passed).
- Reliability tests (Agent 19): concurrent world readers share one snapshot with
  one provider call; personal/conversation/embodiment stores remain Core-isolated
  under concurrency; repeated open/reopen releases handles (temp-root removal
  succeeds); prompts stay bounded across 40 context-heavy turns; large-but-bounded
  inputs truncate/reject at contract edges.
- GPU: live inference-pressure measured earlier (idle ~1.9 GB → ~8.7 GB during
  inference → ~1.9 GB after unload; 6.8 GB reclaimed). No gameplay/FPS data.
- Backup: `VACUUM INTO` per DB; repeated backup/restore cycles tested.

## Findings / risks (not all fixed)

| Risk | Severity | Evidence | Disposition |
|---|---|---|---|
| Concurrent stale world refresh duplicates provider calls | Low | 4 racers → 4 fetches (Agent 19) | Add single-flight/backoff later |
| Unavailable weather always stale → repeated re-poll | Low | refresh retries each trigger | Add backoff |
| Oversized retrieval token persists user row then rejects → orphan message | Low | Agent 19 | Fix in a future batch |
| Event claim/effects use separate SQLite connections | Low | crash window (Agent 07) | Add transactional outbox |
| SQLite concurrent writer contention | Low | tolerated in tests | Monitor |
| No long-run soak / 24h stability | Medium (unverified) | none performed | Owner/CI soak |

## Not measured

- Long-running memory/CPU trends, 24h+ soak, sustained multi-Core load,
  GUI rendering performance (no native GUI), cross-host latency (no Artemis),
  gameplay FPS/frame-time (owner participation required).

## Verdict

Performance/reliability is adequate for development and short homelab sessions;
**production performance acceptance is NO-GO** pending soak, multi-host, and
gameplay evidence.
