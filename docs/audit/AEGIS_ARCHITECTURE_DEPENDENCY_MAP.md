# H. Architecture Dependency Map

**Purpose:** correct implementation order for the missing vision. No code was
changed.

## Shared foundations to design once (before domain work)

1. **Versioned contract/ID foundation** — definition vs instance IDs, revisions,
   provenance, tombstones, actor, idempotency, migration/backup registration.
   *Blocks everything below.*
2. **Shared item catalog + item-instance model** (immutable definition; per-Core
   instance: owner, custody, state, location, provenance).
   *Blocks wardrobe, objects, action asset binding, transfers.*
3. **World object/space model** (type/instance, placement, affordances, state,
   perception; Platform owns shared definitions, Core owns subjective state).
   *Blocks object animation, activity locations, action targets.*
4. **Action registry + execution state machine** (id, category, rigs/anatomy,
   preconditions, params, assets, safety, consent refs, interrupt/cancel).
   *Blocks gestures, object interaction, adult actions, activities, UI menus.*
5. **Consent/eligibility registry** (per-Core enablement, scoped affirmative
   grants, withdrawal/expiry, audit; separate from trust and technical permission).
   *Blocks adult capability, romantic actions, intimate wardrobe, invitations.*
6. **Autonomy policy/goal/plan contract + Core scheduler** (goals, plans, due/
   triggers, budgets, quiet periods, approval thresholds, cancellation, STOP).
   *Blocks activities/routines, social initiation, background work.*

## Dependency chain (each item after its prerequisites)

- Item catalog/instances → wardrobe inventory/composition → outfit selection/
  history → wardrobe transfers/trades/gifts/loans → cross-host wardrobe.
- World object model → object interactions/state → object animation/actions →
  project-linked objects (requires project model) → shared scene/privacy.
- Action registry → action execution/STOP → adult gating → intimate actions and
  embodiment; also gestures/social gestures and activity execution.
- Consent registry → adult capability → adult privacy/log/backup policy.
- Autonomy contract → scheduler → activities/routines execution → social
  initiation/budgets → notifications/quiet periods.
- IPC/event envelope → background world distribution → automatic UI updates →
  cross-host world/objects/actions.
- Native GUI shell → full UX workspaces (wardrobe/objects/consent/activities/
  relationships/admin/developer).
- Backup/restore extension for every new store → migration/version gate →
  acceptance evidence.

## Hard prerequisites already satisfied (reuse, don't rebuild)

- Per-Core identity/storage/ownership/isolation and backup v2.
- World clock/weather/freshness and personal emotion/continuity/event ledger.
- Conversation engine with bounded context and memory retrieval.
- Asset integrity (SHA-256), appearance config, SAPI voice, BMP/animation
  primitives, module manifest/registry/process/tool bridge, Fleet/GPU.

## Risks of wrong order

- Building wardrobe transfers before item instances/custody → unsafe ownership
  moves and cross-Core leakage.
- Building adult content before consent/action registry → cannot enforce consent,
  boundaries, STOP, privacy.
- Building UI before stable contracts → rework and fake/incomplete controls.
- Building autonomy before budgets/consent → harassment/loops and authority
  confusion.
- Adding persistence before backup/migration → unrecoverable personal state.

## Recommended ordering (summary)

Contracts → item/object/action/consent/autonomy contracts → backup of new stores
→ wardrobe/objects/actions implementation → consent/adult → autonomy/activities →
IPC/world distribution → native GUI/UX → modules catalog/security → cross-host →
independent QA/owner acceptance.
