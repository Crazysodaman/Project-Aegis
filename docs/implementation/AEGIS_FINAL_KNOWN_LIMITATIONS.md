# AEGIS FINAL — Known Limitations

**HEAD:** `2bd2513` · not production-ready

## External / owner-gated

1. **Artemis** not deployed/enrolled; no cross-host Fleet or remote inference.
2. **Debian 13 / Sofia VM** not built, installed, or reboot-tested; Linux owner
   proof untested.
3. **Native GUI** not implemented (loopback HTTP dashboard only).
4. **Microphone STT / playback** not verified (file-based SAPI STT only).
5. **GPU avatar renderer** not implemented (schematic CPU BMP frames only).
6. **Owner acceptance** and **independent external QA** absent; no production
   deployment.

## Product limitations

7. World snapshots are injected by CLI/caller; no IPC/background distribution to
   Cores or hosts.
8. GUI/TUI World/Personal sections require operator-written
   `run/status/{world,personal}.json`; no snapshot emitter is wired.
9. Automatic emotion influence is a fixed interaction event; richer
   experience/relationship/world influences need explicit events.
10. Event ledger claim and effects are not a single transaction; a crash can
    claim without all effects (never duplicates).
11. Memory consolidation is explicit; no importance scoring/retention policy;
    keyword retrieval scans Core rows.
12. Wardrobe scoring is tag/keyword heuristics; no fabric/thermal model; caller
    must persist outfit decisions.
13. Modules have no OS sandbox (trusted local code).
14. OS service registration/reboot recovery not implemented (process manager).
15. Backup: per-DB `VACUUM INTO` is live-safe but not a single atomic snapshot
    across all DBs; `owner.proof` is DPAPI/machine-bound; integrity is not
    authenticity.
16. `verify-backup` marker-swap gap (D20-02) and `safe.db` recreation (D20-03).
17. README/completion docs stale (D20-04).
18. Long-run soak and multi-host/hostile-disk tests unperformed.

## Non-claims

- Not gaming-safe (no gameplay FPS evidence).
- Not cross-host verified.
- Not Debian-verified.
- Not independently accepted.
- Not production-ready.
