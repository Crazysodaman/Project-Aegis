# Batch 7 — P0 Universal Core World & Continuity: Audit and Integration Plan

**Date:** 2026-11-26
**Status:** AUDIT ONLY — no implementation performed; no conflicting files modified.
**Baseline:** HEAD `2bd2513`, Batch 7 working tree (all Batch 3–7 work preserved).
**Authority:** Project Aegis Universal Core World & Continuity requirement (P0).

---

## 1. Executive summary

The P0 requirement is **architecturally consistent with the existing blueprint**
(`Project_Aegis_Master_Architecture_Blueprint_v0.2.0.md` §3, §10, §11), which
already specifies shared world context, per-Core continuity, and an emotion
system. It does **not** require a redesign; it requires completing unfinished
foundations and fixing one serious persistence gap.

Current state: the platform has strong **identity, memory, personality,
conversation, appearance/wardrobe bindings, voice (TTS/STT), avatar scene
rendering, relationships, and per-Core DB isolation**, but it has **no shared
world/time/weather service, no emotion state/history, no continuity/elapsed-time
model, no activity/routine model, and no context-aware wardrobe/expression/voice
selection.** Emotions and activities exist only as capability identifiers.

**Critical finding:** backup currently copies only `platform.db`, `safe.db`,
`owner.proof`, and per-Core `core.db` (+ integrity manifest/marker). It does **not**
copy `memory.db`, `conversation.db`, `assets.db`, `social.db`, `embeddings.db`.
P0 durability/recovery cannot be satisfied until backup/restore covers all
per-Core state (and the new personal state store).

## 2. Existing implementation inventory

| Area | Existing | Location | Status |
|---|---|---|---|
| UTC timestamp helper | `now_rfc3339()` (chrono) | `aegis-storage` | Implemented, UTC only |
| Platform config | generic KV `platform_config` (`installation.*` only) | `aegis-installer` / `platform.db` | Implemented; no timezone/world keys |
| Platform identity | `platform_identity` | `aegis-platform` | Implemented |
| Core identity/DB | `core_identity`, per-Core `core.db`, integrity manifest | `aegis-core-storage`, `-integrity` | Implemented, tested |
| Personality | `PersonalityProfile` (tone/values/traits/growth policy); `core_personality` + append-only `personality_growth` in `memory.db` | `aegis-core-sdk` | Implemented; static/descriptive, not mood |
| Memory | `memory_entries` Kind = Fact/Preference/Episode; confidence, tags, provenance, timestamps, revisions; append-only events; import/export | `aegis-core-sdk` (`memory.db`) | Implemented, tested; no structured emotion state |
| Conversations | per-Core `conversation.db`: sessions/messages/events; `created_at`/`updated_at`, message_count, state | `aegis-conversation` | Implemented; no elapsed/offline/continuity summary |
| Turn assembly | system prompt = identity + personality + private memory + history; bounded (40 history, 6 memories, 8000 chars) | `aegis-conversation/engine.rs` | Implemented; no world/personal context |
| Appearance | theme, named expressions/gestures (asset-bound), `wardrobe: Vec<asset_id>`, voice profile, revision | `aegis-embodiment` | Implemented; no context-aware selection/history |
| Assets | per-Core SHA-256 asset library, search/audit/recovery manifest | `aegis-embodiment` | Implemented, tested |
| Voice | Windows SAPI TTS + file-based SAPI STT (live-verified); per-Core voice profile | `aegis-embodiment/voice.rs` | Implemented/live; not emotion-driven |
| Avatar | scene/layers, animation player, BMP rasterizer, expression/gesture maps, transcript→mouth-frame hook, frame sequence | `aegis-embodiment` | Implemented/schematic; not world/emotion-driven; no GPU/window |
| Social | Platform groups/members/messages/events; per-Core relationships with revision history | `aegis-social` | Implemented, tested; no automatic awareness/trust |
| Runtime | state/generation/PID/heartbeat freshness, started_at | `aegis-core-runtime` | Implemented; no "last active cognition" continuity |
| Capabilities | `CoreCapability::{Emotions,Activities,Wardrobe,Expressions,Gestures,Voice,Appearance,Relationships,...}` | `aegis-core-sdk/capability.rs` | **Declarative identifiers only** |
| UI/TUI | external snapshot loader `run/status/{fleet,gpu,inference,service}.json`, 60 s freshness | `aegis-ui` (Batch 7) | Implemented; no world/personal snapshots or emitter |
| Diagnostics | platform/safe/Cores/integrity/modules | `aegis-diagnostics` | Implemented; no world/emotion/continuity |
| Backup | platform/safe/owner.proof/Core `core.db`+manifest+marker | `aegis-backup` | **Incomplete for personal state** |

## 3. Requirement coverage matrix

Legend: DONE / PARTIAL / MISSING.

| P0 requirement | Status | Evidence / gap |
|---|---|---|
| Platform-owned authoritative time | PARTIAL | UTC helper only; no platform clock service |
| Configured timezone + local time/offset | MISSING | No timezone config or fixed-offset model |
| Time of day / seasons / calendar | MISSING | Not implemented |
| Sunrise/sunset (where available) | MISSING | No provider |
| Weather + forecast with provenance/freshness | MISSING | No provider, no storage, no honesty layer |
| Shared context distribution to Cores | MISSING | No world IPC/SDK surface |
| Missing/stale weather honest, provider-failure safe | MISSING | Needs explicit unavailable/stale states |
| Per-Core persistent emotion state | MISSING | Only capability identifier |
| Multiple influences, intensity, history, decay | MISSING | No model/history tables |
| Emotion continuity across sessions | MISSING | No persistence |
| Distinguish internal vs expressed emotion | MISSING | No expression projection |
| Session existence continuity | PARTIAL | Sessions/messages persist; no last-active/elapsed/checkpoint summary |
| Distinguish before/offline/current/planned | MISSING | No continuity model |
| Time-aware behavior (routines, greeting, activities) | MISSING | No policy layer |
| Weather/time/season/emotion-aware wardrobe | MISSING | Only static wardrobe asset list |
| Voice/expression respond to emotion/context | PARTIAL | Voice/expressions exist; no mapping |
| Personality shapes interpretation | PARTIAL | Personality prompt exists; no environment interpretation |
| Per-Core DB isolation for personal state | PARTIAL | Isolation strong; no personal state store |
| Durable, recoverable personal state | MISSING | No store; backup excludes memory/conversation/assets/social/embeddings |
| Migrations without destructive reset | PARTIAL | Additive patterns exist; no personal migration |
| Cross-system event propagation | MISSING | No world/emotion events |
| Core #7 gets infra without invented identity | READY | Roster/profile prohibits invention; infra is identity-agnostic |
| Preserve Sofía identity/assets | READY | Profiles are preservation-first; no rewrite planned |

## 4. Architectural conflicts / duplicated implementations

- No duplicate world/emotion/continuity systems exist (none implemented).
- `Emotions`/`Activities` capability identifiers must not be treated as
  implementations; do not claim them.
- `memory.db` already owns personality + memory + events; adding personal state
  there would couple mood to memory and complicate the memory contract.
- `core.db` has an integrity manifest (SHA-256). Adding tables there would
  invalidate manifests; avoid.
- The Batch 7 `run/status/*.json` snapshot channel is the natural UI bridge but
  has **no emitter**; world/personal snapshots should reuse this channel.
- Backup's allowlist is a hard architectural constraint that currently omits
  most per-Core state; this is not a new design, it is a missing coverage bug.

## 5. Overlap with active Batch 7 agents

| Agent | Overlap | Coordination |
|---|---|---|
| 01 Fleet | Shared world context across hosts; Artemis blocked | Distribute world snapshot via Fleet/IPC after approval; no file overlap |
| 02 GPU | None | World context may include host load later; not required |
| 03 Services | Time authority could live near Platform service | Put clock in a new world module/Platform, not service lifecycle |
| 04 GUI | `run/status` snapshots | Add `world.json`/`personal` later via the same loader; no rewrite |
| 05 TUI | `run/status` loader | Extend loader schemas; no conflict |
| 06 Voice | Emotion→voice parameters | New projection module; do not rewrite `voice.rs` |
| 07 Avatar | Emotion→expression/animation/lip-sync | New projection into existing expression/lipsync hooks |
| 08 Artifact | Wardrobe assets | Reuse asset library; outfit decisions reference asset ids |
| 09 Modules/Social | Activities/routines, relationships→emotion | New pure policy; do not invent trust/awareness |
| 10 Security/Core | New personal DB, backup, ownership/migrations | Requires its isolation/recovery tests |

No Batch 7 agent is currently writing files; Batch 7 integration is complete.
P0 implementation should be a **new coordinated batch**, not interleaved edits.

## 6. Proposed architecture (minimal, no new third-party deps)

### 6.1 Platform-owned shared world — new crate `aegis-world`
- `WorldClock`: timezone label + fixed `utc_offset_minutes` (from
  `platform_config` keys `world.timezone_label`, `world.utc_offset_minutes`);
  computes local date/time, `TimeOfDay`, season, day/night. (Fixed offset avoids
  a new tz-database dependency; DST handled by owner-configured offset or a
  later approved tz crate.)
- `WorldSnapshot { schema, generated_at, clock, season, sun (optional), weather
  (optional), calendar, source, freshness }` with explicit `available=false`
  + reason; never invented weather.
- `WeatherProvider` trait; `UnavailableWeather` default. Optional HTTP provider
  is a separate owner-approved change (network + API key), not part of M1.
- `WorldStore` in `platform.db`: additive `world_context` (current) and
  append-only `world_events` (history). Backed up with `platform.db`.
- Distribution: read via Platform IPC (new `world.status` request) and/or a
  `WorldContextProvider` passed into the Core SDK; no Core opens `platform.db`.

### 6.2 Per-Core personal state — new `<core>/personal.db`
- Ownership row bound to `core_uuid` + `database_instance_uuid` (reject copies).
- Tables: `emotion_state`, `emotion_history` (append-only), `continuity`,
  `activities`, `routines`, `outfit_decisions` (append-only), `personal_events`.
- Rationale: preserves `core.db` integrity manifest and `memory.db` contract;
  additive, recoverable, and independently backupable.
- `PersonalStore` in `aegis-core-sdk` (or a small `aegis-personal` crate) with
  ownership verification and bounded serialization.

### 6.3 Emotion engine (pure, deterministic)
- `EmotionVector` (dimensions e.g. valence/arousal + discrete influences),
  intensity, decay half-life; `apply_influences(state, influences, personality,
  world, elapsed_ms)`; no random startup regeneration.
- Internal state vs `ExpressedEmotion` projection (face/gesture/voice params).
- History append-only; correction/reflection is explicit, not silent.

### 6.4 Continuity
- On session start: `elapsed = now - last_active`; load bounded last-session
  summary, open threads, current activities; refresh world snapshot; write
  `continuity` checkpoint.
- On end/message: update `last_active_at`, `last_checkpoint_at`, `last_session_id`.
- Inject a bounded `ContinuitySummary` + `WorldSummary` into the system prompt
  (e.g. ≤1,000 chars), never raw transcript dumps.

### 6.5 Context-aware decisions (deterministic suggestions)
- `greeting_style(time_of_day, emotion, personality)`.
- `activity_suggestions(world, time, emotion, preferences)`.
- `select_outfit(inventory, world, season, time, activity, emotion, personality)`
  with **hard constraints** (weather safety, identity, asset kind) taking
  precedence; returns rationale.
- Projections into expressions/gestures/voice/avatar; no identity rewrite.

## 7. Database and migration implications

- New `personal.db` per Core: created on demand for existing Cores from
  read-only `core.db` identity; no destructive change to existing DBs.
- Additive `world_context`/`world_events` in `platform.db`; schema revision
  marker; backward-compatible.
- **Backup/restore must be extended** to include `personal.db` and existing
  `memory.db`, `conversation.db`, `assets.db`, `social.db`, `embeddings.db`
  (plus integrity/ownership metadata) before P0 can claim durability. This is a
  prerequisite milestone, not optional.
- Interrupted-write recovery: temp file + atomic rename; verify ownership and
  digest on open; no partial state accepted.
- Migrations must be additive and backup-gated; no resets.

## 8. Test coverage plan (acceptance-mapped)

- Multi-Core simultaneous: same world snapshot, independent emotion/continuity.
- Shutdown → restart → resumption: elapsed time, restored emotion, open threads,
  world freshness.
- Weather unavailable/stale: honest states; local continuity unaffected.
- Provider failure: no data loss, no invented weather.
- Per-Core isolation: emotion/memory/continuity cannot cross DBs; foreign copies
  rejected.
- Emotion: multi-influence, decay, history append-only, no random startup.
- Wardrobe: weather safety + identity constraints beat preference; rationale
  recorded; no asset contamination.
- Personality: same world, different interpretations (deterministic).
- Prompt bounds: context injection bounded; no transcript dumps.
- Migration/recovery: interrupted write; backup/restore round-trip including all
  per-Core DBs.

## 9. Live verification status

- Live today: SAPI TTS + file-based STT; BMP avatar frames; inference-pressure
  GPU measurement; LAN artifact fetch; HMAC identity. **None of these are world,
  emotion, or continuity systems.**
- Weather/time/world/emotion/continuity: **not implemented, not tested, not live.**
- Core #7: infrastructure only; identity remains unwritten.

## 10. Blockers and owner approvals

- Weather provider/network/API key (owner approval).
- Timezone/DST configuration (owner decision; fixed offset first).
- Artemis/Sofia VM distributed world context (owner authorization; blocked).
- Any new dependency (tz database, HTTP client) — none required for M1–M5.
- Backup format change requires a migration/version bump and owner awareness.

## 11. Recommended implementation sequence

1. **M1 — Persistence safety (highest priority):** extend backup/restore to all
   per-Core DBs; verify round-trip; diagnostics visibility. (Fixes a real gap.)
2. **M2 — `aegis-world`:** clock/timezone(fixed offset)/season/time-of-day +
   snapshot store + IPC/SDK read + honest unavailable states + tests.
3. **M3 — Personal state:** `personal.db`, ownership, emotion engine, continuity,
   activity/routine records; multi-Core + restart tests.
4. **M4 — Conversation integration:** bounded world+continuity+emotion context in
   the turn prompt; elapsed-time behavior; tests.
5. **M5 — Projections:** context-aware outfit selection; expression/gesture/voice
   projection; UI/TUI `world.json`/`personal.json` via existing snapshot loader.
6. **M6 — Optional weather:** owner-approved provider; freshness/provenance;
   live verification.
7. **M7 — Distribution + QA:** Artemis/Sofia VM shared context (owner auth);
   independent review.

## 12. Conclusion

P0 is compatible with the current design and should be implemented as a new
coordinated milestone. The immediate prerequisite is **backup coverage of
existing per-Core state**, because personal continuity cannot be claimed durable
while `memory.db`/`conversation.db`/`assets.db`/`social.db`/`embeddings.db` are
excluded from backups.

No conflicting files were modified during this audit. No P0 implementation has
begun. Batch 7 integration remains intact.
