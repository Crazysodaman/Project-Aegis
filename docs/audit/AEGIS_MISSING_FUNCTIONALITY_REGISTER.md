# G. Missing Functionality Register

Grouped by priority. Classification uses the audit vocabulary; no completion
percentages are asserted. Evidence paths are in the domain reports.

## P0 — Critical (identity, safety, data integrity)

1. **Sofía UUID/Constitution not enforced.** `core install` accepts arbitrary
   UUIDs; Constitution digest/asset absent. Risk: silent identity drift.
2. **D20-02** `verify-backup` does not validate Platform/Safe application markers.
3. **D20-03** `safe.db` deletion recreates inactive STOP and empty audit.
4. **Uncommitted tree** (`2bd2513` + dirty) — no reproducible release checkpoint.
5. **Sidecar ownership checks omit database-instance UUID** for memory (and
   backup verification for sidecars); several stores adopt a DB with a missing
   ownership row. Risk: same-UUID instance mixups.
6. **No OS module sandbox** and no publisher signatures/trust.
7. **GUI session token can drive owner-authorized actions** when mutations are on;
   not a true owner proof boundary.

## P1 — Required for release / deployment

8. Artemis agent deployment/enrollment + two-host authenticated inference.
9. Debian 13 / Sofía VM build, systemd install, reboot recovery, Linux owner proof.
10. Native GUI window (current HTTP dashboard/TUI only) and missing Core
    workspaces (memory/personality/voice/appearance/relationships/groups).
11. General Chat (Platform-owned shared space) — absent.
12. World IPC/background distribution + `run/status` emitter; calendar source.
13. Microphone STT, playback, streaming, interruption, voice selection.
14. Module catalog (55) + full lifecycle/signatures/upgrade-rollback.
15. Backup of `modules/`; uninstall deletion coverage; single atomic cross-DB
    snapshot; retention policy.
16. Documentation reconciliation (README/CURRENT_STATE/TEST_EVIDENCE to 942; remove
    contradictions).
17. Owner acceptance + genuine independent external QA.

## P2 — Original Aegis vision (the living environment)

18. Shared wardrobe catalog + per-Core item instances/ownership/custody; sizing/
    compatibility; gifts/loans/trades/swaps; saved outfits (23); favorites;
    privacy/audience (47 private-only); modular armor/undersuit; outfit history
    integration; autonomous persisted selection.
19. Appearance customization: hair/eyes/face/ears/tails/species/makeup/tattoos/
    presets/base rig; canonical Sofía assets and fox anatomy; persistence across
    outfits/avatar updates.
20. Interactive objects/possessions/spaces: object identity/state/holder/
    affordances/containers; conversation/project linkage; shared scene; privacy.
21. Gesture/expression/animation: rigged/full-body animation, species gestures,
    action registry, sequencing, interruption/cancellation, voice sync, GUI.
22. Structured relationships (affection/love/jealousy/conflict/unresolved,
    asymmetry, boundaries); owner-Core semantics; social agency (invitations/
    accept/decline/leave/mute/block/quiet periods/budgets); group memory policy;
    direct Core-Core channels; federation/delivery.
23. Adult romance/intimacy/consent: per-Core opt-in, eligibility (A-04),
    affirmative interaction-specific revocable consent, boundaries/refusal,
    private records with log/summary/backup exclusion, action categories
    romantic/adult, STOP coupling. (Aftercare is not in the approved text.)
24. Autonomy: independent opinions/disagreement, goals, interests/hobbies,
    personal projects, routines/scheduler with due/trigger, reflection/
    personality-development approval, bounded social/background budgets.
25. General Chat and Core-to-Core awareness/governance (legacy voluntary
    governance/collective decisions, if owner wants it retained).
26. Memory: procedural/working domains, importance/retention/forgetting policy,
    verified cross-backup deletion.

## P3 — Enhancements

27. Native/mobile clients; developer workspace; accessibility/reduced-motion.
28. Artifact Library cross-host replication/redundancy (Sofia VM placement).
29. Dual cognition/model routing; richer diagnostics/observability.
30. Visual polish, additional modules/integrations.

## Explicit non-missing

- Platform/Core DB separation, per-Core ownership/isolation, memory/emotion/
  continuity foundation, world/weather, backup v2 live-safe snapshots, installer
  Windows flow, Fleet HMAC, GPU admission/yield, module process framework, CLI
  breadth, SAPI file TTS/STT are real and tested.
