# AEGIS FINAL — Architecture Report

**HEAD:** `2bd2513` · no commit

## Platform, Core, world, personal boundaries

- **Platform-owned shared state:** identity, registry, host/Fleet, services,
  world context (`platform.db` `world_context`/`world_events`), weather provider,
  Safe owner proof/STOP.
- **Core-owned private state (per Core DB):** `core.db` (identity/personality),
  `memory.db` (memory/personality growth), `conversation.db` (sessions/messages),
  `assets.db`+assets (library/appearance/wardrobe refs), `social.db` (private
  relationships), `embeddings.db` (cache), `personal.db` (emotion/continuity/
  activities/routines/outfit/events).
- **World vs personal:** shared world facts are read-only context; a Core's
  interpretation becomes private experience/memory/emotion. No shared personal
  history; no cross-Core private reads.

## Runtime chain (implemented)

World (`aegis-platform::world`) → caller/CLI builds a `WorldSnapshot` → guarded by
`TransportPolicy`/freshness → injected into `ConversationEngine` with continuity,
emotion, and activities → bounded system prompt → model turn → post-turn
continuity update + idempotent event (`aegis-core-sdk::events`) → experience +
emotion influence persisted in `personal.db` → presentation projections
(outfit/voice/expression/animation) → UI/TUI/CLI display.

## Subsystem map (key interfaces)

| Area | Interface |
|---|---|
| World | `WorldService`, `WorldStore`, `WorldClock`, `refresh_if_stale`, `snapshot_for_cores`, `WeatherProvider` |
| Weather | `OpenMeteoProvider` (curl, TTL cache), `parse_open_meteo`, WMO mapping |
| Personal | `PersonalStore`, `EmotionState/History`, `ContinuitySummary`, `apply_influences_at`, `apply_relationship_event`, `expressed_emotion` |
| Events | `aegis_core_sdk::events` ledger; engine applies once per turn |
| Conversation | `TurnOptions::with_world/continuity/emotion/activities`; bounded prompt; `SessionSummary`/`consolidate_experiences` |
| Presentation | `select_outfit`, `to_personal_record`, `voice_projection`, `expression_for_emotion`, `AvatarState`, `render_state_sequence` |
| Voice | SAPI TTS with optional `TtsStyle` (rate/volume); SAPI file STT |
| Fleet | host eligibility/freshness, HMAC identity, transport policy, deployment plan, inference queue |
| Backup | format v2, `VACUUM INTO` live-safe snapshots, ownership/digest, v1 compat |
| Packaging | `.corepkg` validate/install via CoreInstaller |
| Modules | dependency/version checks, lifecycle preconditions, real process execution |
| Social | membership-enforced groups, append-only relationship events, bounded awareness |
| UI | HTTP dashboard (World/Personal snapshots), TUI world/personal sections |

## Invariants

- Platform must work with zero Cores; shared world does not imply shared personal.
- Per-Core DB ownership bound to core UUID (+ instance UUID for core/personal).
- No random startup emotion; no invented weather/memories/awareness.
- Bounded prompts/context; no transcript dumping.
- Frozen identities #1/#2/#3/#6 preserved; #4/#5 design-only; #7 blank.
- No automatic trust; enrollment/owner proof required.

## Dependency posture

Only `chrono-tz 0.10.0` (Batch 8, IANA/DST) was added. Weather uses system
`curl.exe`. No new third-party crates were added in the FINAL programme.

## Known architectural gaps

No IPC/background world distribution (caller/CLI injects); no native GUI/GPU
renderer; no microphone STT; no OS module sandbox; no multi-host deployment; some
defects (backup marker swap, safe.db recreation) documented.
