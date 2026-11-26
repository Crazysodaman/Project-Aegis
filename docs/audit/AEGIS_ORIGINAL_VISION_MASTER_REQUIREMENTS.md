# A. Original Vision — Master Requirements Register

**Evidence base:** `Project_Aegis_Master_Architecture_Blueprint_v0.1.md` and
`v0.2.0.md`, `OpenCode_Workorder_Aegis_v0.2.0_ReadOnly_Audit.md`, plus the
read-only legacy reference `D:\SofiaDev\SofiaAdaLyra` (v0.3.0 master spec,
discovery decisions, wardrobe/object/social/governance specs). Where a later
decision superseded an earlier one, both are recorded.

**Classification vocabulary:** IMPLEMENTED AND VERIFIED / IMPLEMENTED NOT FULLY
VERIFIED / PARTIALLY IMPLEMENTED / FOUNDATION ONLY / DESIGNED BUT NOT
IMPLEMENTED / MISSING / INTENTIONALLY DEFERRED / SUPERSEDED.

## Platform & identity (designed, authoritative)

| # | Requirement | Classification | Evidence |
|---|---|---|---|
| P1 | Platform is not a Core; must work with zero Cores | IMPLEMENTED AND VERIFIED | `aegis-platform`; zero-Core CLI/lifecycle tests |
| P2 | Separate Project/Platform/Host/Core/Module/Owner identity; UIs are clients | PARTIALLY IMPLEMENTED | `aegis-host`, `aegis-fiic`? (IPC); UI reads DBs directly (finding) |
| P3 | One active authority, fencing, no auto-promotion/split-brain | FOUNDATION ONLY | `authority:"local_single"` hard-coded; no lease/fencing |
| P4 | Platform/Safe/Core DB separation; General Chat Platform-owned; private chat Core-owned | PARTIALLY IMPLEMENTED | DBs separate; General Chat MISSING |
| P5 | Explicit owner authorization and enrollment; no auto-trust | IMPLEMENTED NOT FULLY VERIFIED | Safe DPAPI, Fleet enroll; Linux proof untested |

## Universal Core

| # | Requirement | Classification | Evidence |
|---|---|---|---|
| C1 | One universal Core contract/SDK; no permanent Sofía special path | PARTIALLY IMPLEMENTED | `aegis-core-sdk`; 23 capabilities; lifecycle incomplete |
| C2 | Approved roster #1/#2/#3/#6, #4/#5 prohibited, #7 blank | IMPLEMENTED NOT FULLY VERIFIED | roster/profiles tests; UUID/Constitution not enforced |
| C3 | Per-Core independent DBs, memory, emotion, relationships | IMPLEMENTED AND VERIFIED (foundation) | per-Core DBs + ownership tests |
| C4 | 19-domain capability parity (identity/constitution, memory, continuity, emotion, relationships, conversation, autonomy, voice, avatar, gestures, wardrobe, actions, modules, activities, diagnostics, backup) | PARTIALLY IMPLEMENTED | manifest over-advertises; Constitution/actions/autonomy missing |
| C5 | Core #1 must not be a generic template; preserve Sofía | MISSING/PARTIAL | name+voice string only; Constitution/assets/fox anatomy absent |

## Living-environment requirements (v0.2 + legacy)

| Domain | Requirement | Classification | Evidence |
|---|---|---|---|
| Sofía | UUID `a9a0a604…`, Constitution `F30AFEDF…`, fresh DB, no historical import | PARTIALLY IMPLEMENTED | fresh init yes; UUID/Constitution unenforced |
| Sofía | Canonical personality, appearance, fox ears/tail, voice reference | MISSING (assets), CONFIG-ONLY (voice) | no assets/rigs; reference string only |
| Mack/Nova | Approved identity, personality, appearance | PARTIALLY IMPLEMENTED | profiles present but incomplete (Mack height/weight, Nova details) |
| Vera | Spartan identity, armor, voice, external spec | DESIGNED/EXTERNAL ONLY | profile points to external spec; no data/assets |
| Cass #4/#5 | Design-only frozen twins | COMPLIANT (prohibited) | roster/profiles/tests |
| #7 | Blank identity; no invented name/gender/species/avatar/voice/personality | IMPLEMENTED (blank) | roster/profiles/tests |
| Wardrobe | Shared catalog; per-Core inventory; ownership vs wearing; sizing; gifts/loans/trades; favorites; 23 saved outfits; 275 garments; 47 private-only; armor/undersuit | PARTIALLY IMPLEMENTED (assets/selection) / MISSING (catalog/instances/transfers/privacy) | `aegis-embodiment`; legacy has full system |
| Appearance | Hair/eyes/face/ears/tails/species/makeup/tattoos/presets/base rig; persistent; autonomous changes | MISSING (domains) | `appearance.rs` narrow; no assets/rig |
| Objects | Books/notebooks/papers/notes/pens/binders/project folders/decor/belongings; persistent state, holders, affordances, conversation/project linkage | MISSING | none in Aegis; legacy `interaction` module had it |
| Gestures/animation | Gesture/expression libraries, species/Core gestures, body language, object interaction, sequencing, sync, interruption | PARTIALLY IMPLEMENTED | enums/keyframes/BMP; no rig/action registry |
| Relationships | Owner-Core and Core-Core; structured asymmetric states; history; trust never grants permission | PARTIALLY IMPLEMENTED | `trust:u8` scalar + labels; no structured dimensions |
| Social agency | Invitations, accept/decline, leave, mute/block, quiet periods, budgets, anti-harassment | MISSING | groups are owner-administered only |
| Group chat | Private multi-Core groups, membership, transcript privacy, memory policy | PARTIALLY IMPLEMENTED | `aegis-social` groups; no UI/memory policy/federation |
| Adult/intimacy | Optional per-Core; romantic/sexual preferences; affirmative revocable consent; boundaries; privacy; action registry; aftercare (not in blueprint) | MISSING | no code; A-04 provisional |
| Personality/autonomy | Independent opinions, disagreement, goals, interests, routines, gradual change, bounded autonomy/social initiation | PARTIALLY IMPLEMENTED (profile/history) / MISSING (autonomy/opinions) | prompt-only; no scheduler |
| Activities | Hobbies, routines, projects, scheduling, due/trigger, progress | PARTIALLY IMPLEMENTED (records only) | `personal.db` activities/routines; inert |
| Memory | Fact/preference/episodic/procedural; consolidation; forgetting/correction; provenance; no cross-Core sharing | PARTIALLY IMPLEMENTED | memory/consolidation; no procedural/retention policy |
| Conversation | General Chat (Platform), private chat (Core), groups; bounded context; tool approval | PARTIALLY IMPLEMENTED | per-Core chat; General Chat MISSING |
| Voice | Per-Core voice identity, TTS/STT, expressive delivery, streaming, interruption, devices | PARTIALLY IMPLEMENTED | SAPI TTS/file STT; no mic/playback/streaming |
| World | IANA clock/DST, time-of-day/season, calendar, weather/forecast provenance/freshness, automatic distribution | PARTIALLY IMPLEMENTED | `aegis-platform::world`; no IPC/background/calendar |
| Modules | 55-entry catalog (5 companion/47 optional/MOD-INTERACTION/2 adapters); manifests/permissions/lifecycle/sandbox | FOUNDATION ONLY | framework; 0/55 catalog modules |
| Artifact/Core packages | Artifact Library; `.corepkg`; integrity; install/update/rollback; cross-host | PARTIALLY IMPLEMENTED | `.corepkg` local; no cross-host library |
| Fleet/GPU | Artemis/Venus; explicit enrollment; gaming-aware yield; inference routing | IMPLEMENTED NOT FULLY VERIFIED | local Venus + HMAC; Artemis/Debian blocked |
| UI | TUI primary shell; native GUI; workspaces; chat/groups/wardrobe/objects/admin/dev | PARTIALLY IMPLEMENTED | HTTP dashboard + refresh-only TUI; no native |
| Backup | All DBs/assets, live-safe, restore, recovery | PARTIALLY IMPLEMENTED | v2 + VACUUM INTO; D20-02/03 open |
| Install | Windows + Debian 13 service install/upgrade/rollback | PARTIALLY IMPLEMENTED | Windows file-tree; no OS service/Debian |
| Security | Owner proof, STOP, audit, isolation, transport, module sandbox | PARTIALLY IMPLEMENTED | Windows DPAPI, HMAC plain HTTP, no sandbox |

## Historical decisions / supersessions (summary)

- v0.1 superseded by v0.2.0; v0.2.0 further referenced to external v0.3.0
  `AEGIS_MASTER_SPECIFICATION` in the legacy repo (AEGIS-MAB-003).
- Fresh DB / no historical import is intentional (legacy DB preserved read-only).
- Python legacy UI and desktop path abandoned for Rust TUI-first.
- Encryption-at-rest deferred; per-Core SQLite + OS permissions chosen.
- Adult capability (A-04) deferred/provisional; fail-closed by absence.
- Hephaestus backup placement (B-01) and retention/RPO/RTO (B-06) undecided.
- #6 Vera authorization conflict: DECISIONS says design-only while later work
  installs it; no explicit owner artifact found.

## Unresolved historical questions

1. Is the legacy repo's v0.3.0 spec the actual governing source, and should it be
   mirrored into `D:\Aegis`? It is currently external-only.
2. Was explicit owner authorization recorded for implementing/installing #6 Vera?
3. Are the 275/23/47 wardrobe figures the canonical baseline (later legacy commit
   updated to 295/51)? No target schema exists in Aegis either way.
4. What is the approved owner-visible form of adult consent/eligibility (A-04)?
5. Which Platform authority/fencing model and backup placement (B-01/B-06) are
   approved?
