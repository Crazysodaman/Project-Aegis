# Wave 3a — Autonomy Foundation

Wave 3a adds a per-Core autonomy substrate and a real execution engine. It
stores goals, plans, scheduled tasks, settings, and an append-only autonomy
event ledger, then executes due work against the Core's real possessions,
consent, and personal stores. It does not fabricate documents, inventory,
experiences, or consent.

## Contracts (`aegis-contracts::living`)

Additive contracts reuse the existing `Goal` and `LIVING_CONTRACT_VERSION`:

- `TaskStatus`: `Pending`, `Active`, `Paused`, `Completed`, `Failed`,
  `Cancelled` with `as_str` / `parse`.
- `PlanStatus`: `Active`, `Completed`, `Abandoned` with `as_str` / `parse`.
- `Plan`: schema, plan id, Core UUID, optional goal id, title, status,
  timestamps, revision; `validate`.
- `Task`: schema, task id, Core UUID, optional plan/goal ids, title,
  activity kind, status, priority, optional due time, progress percent,
  JSON payload, timestamps, revision; `validate` enforces identifiers,
  bounded text, `progress_percent <= 100`, and `revision >= 1`.
- `AutonomySettings`: schema, Core UUID, `enabled`, `max_active_tasks`
  (1..=16), timestamp, revision; `validate`.

## Per-Core database

`<core>/autonomy.db` uses SQLite application marker `AEGU` (`0x4145_4755`)
and component `core_autonomy`. The database is bound to both the Core UUID and
the Core database instance UUID through the immutable `autonomy_ownership`
table. The schema marker `aegis.core.autonomy.v1` lives in `autonomy_schema`.

Tables:

- `goals`
- `plans`
- `tasks`
- `autonomy_settings`
- `autonomy_events` (append-only via UPDATE/DELETE triggers)

Ownership rows and autonomy events are immutable. A copied database from a
different Core (or a database from a different Core database instance) is
rejected during `AutonomyStore::open`.

`AutonomyStore::open(root, core_uuid)` exposes:

- Goals: `create_goal`, `update_goal`, `get_goal`, `list_goals`.
- Plans: `create_plan`, `get_plan`, `list_plans`.
- Tasks: `create_task`, `update_task`, `get_task`, `list_tasks`,
  `tick(now)` (due `Pending` tasks with `due_at <= now`; missing `due_at`
  means not scheduled).
- Lifecycle: `start_task`, `pause_task`, `resume_task`,
  `complete_task(progress)`, `fail_task(reason)`, `cancel_task`.
- Settings: `settings`, `set_settings`.
- Events: `list_events`.

`CoreSdkContext::open` now opens autonomy automatically and
`CoreSdkContext::autonomy()` borrows the store.

## Execution engine (`AutonomyEngine`)

`AutonomyEngine::run_due(now)` respects `AutonomySettings.enabled`; when
disabled it returns an empty list without touching task state. It executes due
pending tasks plus previously active/resumed tasks, bounded by
`max_active_tasks`, and returns `TaskRun { task_id, status,
progress_percent, message }`.

Activities:

- `read_document`: reads a real document from `PossessionsStore`, advances by
  `percent_per_run` (default 34), records a bounded "Read document <title>"
  experience through `PersonalStore`, and completes at 100.
- `write_journal`: appends payload `append` to the real document body via
  `PossessionsStore::update_document`, advances progress, records an
  experience, and completes.
- `organize_inventory`: lists real inventory and records a bounded experience
  with the item count.
- `social_interaction`: reads payload participant, capability, and scope, and
  checks `ConsentStore` at action time. Missing, revoked, or expired consent
  fails the task with `consent_required`; active consent records an experience
  and completes.
- `reflect`: records a bounded reflection/experience and completes.
- Anything else fails with `unsupported_activity`.

Partial work is persisted before the task is paused. Reopening the store keeps
the progress; `resume_task` followed by another `run_due` continues from the
stored progress. Each Core's engine can only reach that Core's stores and
database.

## Tests

`crates/aegis-core-sdk/tests/autonomy.rs` covers:

1. Goal → plan → scheduled task → tick → start → partial execution → pause →
   reopen → resume → completion with progress preserved.
2. Reading a real document created through `PossessionsStore` and verifying the
   recorded experience.
3. Consent denial, grant, success, revocation, and failure again.
4. Disabled autonomy returning no runs and leaving tasks untouched.
5. Sofía (#1) and Mack (#2) isolation, persistence across reopen, and rejection
   of a foreign autonomy database copy.
6. Schema/ownership markers, append-only events, and unsupported activity
   failure.

Command run for this wave:

```text
cargo test -p aegis-core-sdk -p aegis-contracts
cargo clippy -p aegis-contracts -p aegis-core-sdk --all-targets -- -D warnings
```

## Limitations

- This wave provides a single-process, synchronous scheduler; there is no
  daemon, wall-clock timer, or background executor.
- Plans are stored and listed but not automatically advanced by the engine.
- `max_active_tasks` caps task starts per run; it is not a global concurrency
  governor across processes.
- Cross-Core work is out of scope: every store remains strictly per-Core.
- No new third-party crates were added.
