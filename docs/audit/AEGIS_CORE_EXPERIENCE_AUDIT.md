# C. Core Experience Audit

**Scope:** individual Core identity, personality, memory, autonomy, embodiment,
wardrobe, objects, relationships, intimacy, activities, voice.
**Repo:** `D:\Aegis` @ `2bd2513` (dirty). **Read-only audit.**

## C.1 Identity & approved Cores

| Core | Requirement | Actual | Classification |
|---|---|---|---|
| #1 Sofía | UUID `a9a0a604…`, Constitution `F30AFEDF…`, fresh DB, preserved persona/appearance/voice | Name + voice-reference string; no Constitution/UUID enforcement; no canonical personality/assets; fox ears/tail absent | PARTIALLY IMPLEMENTED / MAJOR GAPS |
| #2 Mack | Approved male identity, personality, warm voice, workwear | Profile has traits/appearance text; missing approved height/weight; voice reference not materialized | PARTIALLY IMPLEMENTED |
| #3 Nova | Approved female identity, rebellious/playful, appearance | Profile has traits/appearance; incomplete canonical detail | PARTIALLY IMPLEMENTED |
| #4/#5 | Twins, design-only, no invented identity | Prohibited in roster/profiles/install | COMPLIANT |
| #6 Vera | Spartan identity, measurements, armor, voice | External-spec pointer only; no data/assets; authorization conflict in docs | DESIGNED/EXTERNAL ONLY |
| #7 | Blank; no invented identity/appearance/voice/personality | Blank profile; owner name required; infra identity-agnostic | COMPLIANT |

**Critical:** `core install` accepts arbitrary UUIDs (test roots show #1 under
non-canonical UUIDs). The roster has no UUID field. The Constitution exists only
as a capability enum. Approved identity is not executable invariant.

## C.2 Personality, preferences, autonomy

- Personality profile, tone/values/traits/growth history persist in `memory.db`.
- Runtime effect is **prompt-only** (tone/values text); traits do not drive
  behavior. Mack/Nova traits have no runtime effect.
- Development = caller-supplied full profile revision; no reflection, evidence,
  confidence, contradiction, proposal/approval.
- Preferences: only `MemoryKind::Preference`; no Core-owned preference model.
- Independent opinions / disagreement / goals / interests: **MISSING**.
- Bounded autonomy, social initiation, scheduler, budgets, quiet periods:
  **MISSING**.
- Classification: **PARTIALLY IMPLEMENTED (description/persistence), MISSING
  (autonomous behavior).**

## C.3 Emotion & continuity

- Six-dimensional emotion state, deterministic decay (2700 s half-life), history,
  continuity, elapsed offline, activities/routines, outfit decisions, event
  ledger in `personal.db`.
- Idempotent `interaction:<session>:<message>` event applies once; relationship
  events exist; engine updates continuity after turns.
- No random startup emotion; no cross-Core contamination (tested).
- Gap: event claim/effects not atomic; richer experience→emotion wiring is
  caller-driven; no reflection/meaning-making.
- Classification: **IMPLEMENTED AND VERIFIED (foundation).**

## C.4 Memory & conversation

- Per-Core `memory.db`/`conversation.db`; isolation/ownership bound; memory kinds
  Fact/Preference/Episode; provenance/confidence/tags; events append-only;
  consolidation and session summaries; bounded context.
- General Chat (Platform-owned shared space) **MISSING**.
- Retrieval keyword/semantic; no procedural memory, importance scoring,
  eviction/retention policy; oversized-token path can leave an orphan user row.
- Classification: **IMPLEMENTED NOT FULLY VERIFIED.**

## C.5 Wardrobe & inventory (original core requirement)

- Aegis: per-Core `assets.db` wardrobe assets, `Appearance.wardrobe: Vec<String>`,
  tag-scored `select_outfit`, outfit decision history.
- Missing: shared catalog, item-instance ownership/custody, wearing vs owning,
  sizing/fit/compatibility, gifts/loans/trades, public/private/owner-only,
  favorites, saved outfits, modular armor/undersuit, autonomous persisted
  changes, UI.
- Historical 275 garments / 23 outfits / 47 private-only: present only in legacy
  (`outfits.json` has 23); no Aegis destination schema.
- Classification: **PARTIALLY IMPLEMENTED (adjacent assets), MISSING (the system).**

## C.6 Appearance & avatar

- Config: avatar asset id, theme, named expressions/gestures, wardrobe ids, voice
  profile; SHA-256 asset integrity; owner-proof CLI customization.
- Missing: hair/eyes/face/ears/tails/species/makeup/tattoos/presets/base rig,
  decoded art, native/GPU renderer, layered garment slots, autonomous changes,
  fox feature persistence. Rendering is schematic CPU BMP rectangles.
- Classification: **PARTIALLY IMPLEMENTED (config + synthetic frames).**

## C.7 Objects & possessions

- **MISSING in Aegis.** No object/space/holder/container/affordance/state model.
- Legacy had `world_model.py`, `world_store.py`, `LabWorld`, interaction ops,
  project/conversation linkage, privacy/audience.
- Classification: **MISSING (was coded in legacy).**

## C.8 Gestures, expression, animation

- Six expressions/gestures, keyframe interpolation, deterministic state
  projection, offline BMP frame sequence + manifest, text→mouth-frame heuristic.
- Missing: rig/skeleton/morphs, full-body/body language, blend trees, playback
  loop, action registry, interruption/cancellation, object/social gestures,
  voice/animation sync, GUI integration.
- Classification: **PARTIALLY IMPLEMENTED (primitive), FOUNDATION ONLY.**

## C.9 Relationships & social

- Per-Core `social.db` relationships with `trust:u8`, label/notes/tags,
  revisioned upsert + append-only events; Platform private groups with
  membership enforcement; bounded awareness; privacy tests.
- Missing: structured dimensions (affection/love/jealousy/conflict/unresolved),
  asymmetry representation, owner relationship semantics, invitations/accept/
  decline/leave/mute/block, quiet periods/budgets, direct Core-Core channels,
  delivery/federation, group memory policy, owner-admin read audit, social UI.
- Classification: **PARTIALLY IMPLEMENTED.**

## C.10 Adult romance, intimacy, consent

- **MISSING.** No romance/attraction/consent/adult capability, table, gate, or
  action category. A-04 eligibility provisional. "Fail-closed" is absence, not an
  enforced gate.
- Required (blueprint §29): per-Core optional adult capability, affirmative
  interaction-specific revocable consent, boundaries/refusal, privacy/log/backup
  exclusion, action registry (romantic/adult), STOP coupling. "Aftercare" is not
  in the approved text.
- Core #7: no romantic/sexual preference may be invented.
- Classification: **DESIGNED BUT NOT IMPLEMENTED / DEFERRED.**

## C.11 Activities & routines

- `Activity`/`Routine` records persist; continuity includes latest 8 activities;
  conversation/embodiment consume caller-supplied activities.
- Missing: write path, status transitions, due/trigger/recurrence, scheduler,
  autonomy, project/hobby model, routine consumption; routines are inert.
- Classification: **PARTIALLY IMPLEMENTED (storage only).**

## C.12 Voice

- Real: SAPI TTS (rate/volume style), file-based SAPI STT round-trip; per-Core
  voice asset isolation; provider-neutral traits.
- Missing: microphone capture, playback, device routing, streaming, barge-in,
  VAD, confidence/fallback, voice selection/cloning; emotion projection not wired
  to CLI; voice identity is metadata; Sofía reference unverified.
- Classification: **PARTIALLY IMPLEMENTED (Windows file TTS/STT).**

## C.13 World interaction

- World clock/weather exist and are consumed by conversation/presentation when
  injected. No automatic IPC/background distribution, no Core-side polling,
  calendar empty, UI snapshot emitter missing.
- Classification: **PARTIALLY IMPLEMENTED.**

## C.14 Core individuality verdict

The universal substrate is genuinely identity-agnostic and isolates per-Core data
well. However, the requested **individuality** (opinions, disagreement, autonomy,
structured relationships, embodied identity, wardrobe/objects, adult agency) is
mostly absent or prompt-only. Extra care is required not to let generic templates
erase approved Core identity — which is already a risk because UUID/Constitution
are unenforced and appearance is synthetic.
