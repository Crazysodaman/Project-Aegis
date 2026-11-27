# Wave 3b — Autonomy Tools

Wave 3b exposes the Wave 3a autonomy substrate through the built-in
`aegis-tools` bridge. Every tool is caller-bound to the `ToolExecutor` Core,
audited through the Safe chain, and STOP-gated before execution. No tool
accepts a `core_uuid` or storage path argument, so a Core can only reach its own
`autonomy.db`, possessions, consent, and personal stores.

## Tools

| Tool | Sensitivity | Input |
| --- | --- | --- |
| `goal.create` | PersonalWrite | `title`, `description?`, `priority?`, `due_at?` |
| `goal.list` | ReadOnly | `limit?` |
| `goal.update` | PersonalWrite | `goal_id`, `status?`, `progress_percent?`, `priority?` |
| `task.create` | PersonalWrite | `title`, `activity_kind`, `payload?`, `due_at?`, `priority?`, `plan_id?`, `goal_id?` |
| `task.list` | ReadOnly | `limit?` |
| `task.start` | PersonalWrite | `task_id` |
| `task.pause` | PersonalWrite | `task_id` |
| `task.resume` | PersonalWrite | `task_id` |
| `task.complete` | PersonalWrite | `task_id`, `progress?` |
| `task.cancel` | PersonalWrite | `task_id` |
| `autonomy.tick` | PersonalWrite | `{}` |
| `autonomy.settings` | ReadOnly | `{}` |
| `autonomy.set_enabled` | PersonalWrite | `enabled`, `max_active_tasks?` |

All tools are registered in `TOOL_SPECS` and dispatched from `ToolExecutor::run`.
`autonomy.*` and `goal.*`/`task.*` use the `autonomy.read` / `autonomy.write`
permissions and are PersonalWrite or ReadOnly, so they inherit the existing
grant, STOP, audit, and owner-approval pipeline without becoming Sensitive.

## Behaviour

- `goal.create` builds a real `Goal` (schema, caller Core UUID, generated id,
  `Active` status, zero progress, `tool` provenance, revision 1) and persists it
  through `AutonomyStore::create_goal`.
- `goal.update` fetches the current goal first and only mutates the supplied
  fields (`status`, `progress_percent`, `priority`); the store advances the
  revision and timestamps.
- `task.create` builds a real `Task` in `Pending` state with an optional JSON
  object payload, due time, priority, and optional plan/goal links.
- Lifecycle tools delegate to `AutonomyStore::{start,pause,resume,complete,cancel}_task`.
  `task.complete` defaults progress to 100 and rejects values above 100.
- `autonomy.tick` opens a `CoreSdkContext` for the caller Core (autonomy,
  possessions, consent, personal stores) and runs `AutonomyEngine::run_due(now)`.
  It returns the engine's `TaskRun` results. When autonomy settings are
  disabled the engine returns no runs and leaves task state untouched.
- `autonomy.settings` returns the caller Core's settings.
- `autonomy.set_enabled` toggles `enabled` and optionally updates
  `max_active_tasks` (bounded to 1..=16).

Identifiers and bounded text use the same validation helpers as the existing
creation/consent tools. Audit records contain the tool name, caller Core,
outcome, and argument *keys* only, never argument values.

## Tests

`crates/aegis-tools/tests/autonomy_tools.rs` exercises the real `ToolExecutor`
and real installed Cores:

1. Create a goal and a reading task backed by a real document; tick executes it
   (50% → paused), survives a new executor, resumes to completion at 100%, and
   records the reading experience in the personal store.
2. A future-due task is skipped by `autonomy.tick`, while a past-due task runs.
3. Manual `start` / `pause` / `resume` / `complete` (with progress) / `cancel`
   lifecycle transitions.
4. Disabling autonomy through `autonomy.set_enabled` blocks execution, leaves
   tasks pending, and re-enabling allows the due task to complete.
5. A consent-denied social task fails with `consent_required`, then succeeds
   after a real `ConsentStore` grant.
6. Mack cannot list or mutate Sofía's tasks/goals; Sofía's records are
   untouched.

Commands run for this wave:

```text
cargo test -p aegis-tools
cargo clippy -p aegis-tools --all-targets -- -D warnings
```

## Limitations

- Scheduling is still pull-based: `autonomy.tick` runs due work only when the
  model/owner invokes it; there is no daemon or wall-clock timer.
- `task.create` schedules `Pending` tasks and does not create plans automatically.
- `goal.update` exposes status/progress/priority only; title/description edits
  and plan management remain SDK-level operations.
- `autonomy.tick` opens the caller Core context synchronously; long activities
  still run inline within the tool call.
- Cross-Core autonomy remains out of scope; each tool is bound to one Core.
- No new third-party crates were added.
