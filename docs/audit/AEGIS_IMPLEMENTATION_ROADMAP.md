# J. Implementation Roadmap (dependency-aware batches)

**Status:** PROPOSED ONLY. No implementation began. Owner authorization required.

Each batch lists scope, subsystems, dependencies, acceptance, tests, risks.
File ownership/agents to be assigned after owner approves priorities.

## Batch A — Release-critical defects (P0)
- Fix D20-02 (verify markers), D20-03 (Safe anchoring/recovery), D20-18
  (instance-UUID sidecar checks), D20-19 (audit triggers), D20-20 (world
  freshness), D20-06 (orphan message), D20-05 (single-flight), D20-07 (atomic
  event effects), D20-13 (module example), D20-15/16 (UI emitter/scoping),
  D20-17 (GUI auth), D20-04 (docs).
- Subsystems: backup, safe, personal/events, platform/world, conversation, ui,
  modules, docs.
- Acceptance: adversarial tests for each; full suite green; no regressions;
  live backup/restore with marker-swap rejection.
- Risk: touching security/backup; require owner review.

## Batch B — Identity invariants & capability truth
- Bind approved Core numbers to canonical UUIDs and (where approved) Constitution
  digest/asset; reject mismatches; single canonical roster source; per-Core
  capability manifests; #6 authorization resolution; Vera canonical data import
  decision; preserve #4/#5/#7 rules.
- Acceptance: negative install tests (wrong UUID/name/#4/#5/#7); drift tests;
  no approved identity changes without owner decision.
- Blocks: Sofía 1.0 claim; any Core packaging.

## Batch C — Shared contracts for the living environment
- Versioned definition/instance IDs, provenance, revision, tombstones,
  idempotency; item catalog/instance; world object/space; action registry;
  consent/eligibility; autonomy policy/plan. Design once in `aegis-contracts`.
- Acceptance: contract tests, migration/backup registration, privacy fields.
- Blocks: Batches D–H.

## Batch D — Wardrobe system (P2)
- Shared catalog; per-Core inventory/instances; ownership vs wearing; sizing/
  compatibility; gifts/loans/trades/swaps; privacy/audience; favorites; saved
  outfits (port/clean the legacy 23 or re-author with owner); modular armor/
  undersuit (Vera/Sofía); outfit history; autonomous persisted selection;
  backup/restore; CLI/UI.
- Acceptance: transfer atomicity/rollback, no cross-Core leak, weather/identity
  safety, restore preserves inventory/history.

## Batch E — Objects, spaces, actions, animation (P2)
- World object/space model; holders/containers/affordances/state; conversation/
  project linkage; action registry/execution (preconditions/permissions/STOP/
  interruption/cancel); rigged/full-body animation; gestures/social/object
  actions; GUI/avatar integration.
- Acceptance: persistent object state, two-principal concurrency/idempotency,
  STOP cancels dependent animation, no object leakage across privacy scopes.

## Batch F — Consent & adult systems (P2, owner-gated)
- Per-Core adult enable/disable; eligibility (A-04); affirmative interaction-
  specific revocable consent; boundaries/refusal; private records with log/
  summary/backup exclusion; romantic/adult action categories; STOP coupling;
  UI gating. No Core #7 identity/preferences invented.
- Acceptance: consent cannot be inferred from trust/relationship; withdrawal is
  immediate; STOP halts actions; privacy/backup policy tests.
- Hard dependency: Batch C contracts + Batch E action runtime.

## Batch G — Autonomy, activities, social agency (P2)
- Goals/interests/projects; plans/routines/scheduler; budgets/quiet periods/
  approval thresholds; social initiation/invitations/accept/decline/leave/mute/
  block; structured relationships; group memory policy; owner relationship.
- Acceptance: bounded autonomy, no loops/harassment, per-Core isolation, STOP.

## Batch H — Voice, vision, world distribution, GUI (P2/P1)
- Mic STT/playback/streaming/interruption/device routing; per-Core voice
  selection; voice/avatar sync; TTS emotion wiring; IPC/background world
  distribution + snapshot emitter; calendar provider; native GUI shell and full
  workspaces (chat/groups/wardrobe/objects/consent/activities/admin/dev).
- Acceptance: real audio loop, automatic world updates across Cores, UI performs
  real operations (no fake controls), cross-Core snapshot scoping.

## Batch I — Modules & packaging (P1/P3)
- Recover/publish 55-entry catalog with design IDs; implement module packages
  incrementally (start companions/IT modules); signatures/trust; full lifecycle;
  upgrade/rollback; OS sandbox/resource enforcement; per-Core grants; Artifact
  Library cross-host (Sofia VM).
- Acceptance: install/activate/update/rollback/uninstall without data damage;
  sandbox denial tests; catalog acceptance matrix.

## Batch J — Deployment & acceptance (P1)
- Artemis agent install/enroll (owner auth); Debian 13 Sofía VM build/systemd/
  reboot/Linux owner proof; cross-host Fleet/inference; clean-room Windows
  install; backup/restore under service; commit checkpoint; independent external
  QA; owner acceptance.

## Priority ordering recommendation

P0 Batch A + B first (safety/identity truth). Then C (contracts), D/E/F/G in
dependency order, H, I, J. Do not build adult (F) or autonomy (G) before action/
consent contracts (C/E). Do not build wardrobe transfers (D) before item
instances (C). Do not build UI workflows (H) before contracts stabilize.

## Completion evidence per batch

- Named requirement IDs, files, tests (unit/integration/runtime/live), migration/
  backup proof, security tests, and a documented residual-risk list.
- No batch is "complete" from documentation or unit tests alone.
