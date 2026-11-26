# Batch 6 — Agent 09: Module Trust/Limits & Social Participation

Baseline: `2bd2513`. All work is additive and restricted to
`crates/aegis-modules/**`, `crates/aegis-social/**`, and this document. No
workspace manifests, `Cargo.lock`, CLI, or other crates were modified. No new
dependencies were added and no `unsafe` code was introduced. Existing
uncommitted Batch 5 work was preserved.

## Status

Complete and verified with package-scoped gates:

- `cargo test -p aegis-modules -p aegis-social` — **56 passed, 0 failed**
  - `aegis-modules` lib unit tests: 10 (5 policy + 5 pre-existing manifest)
  - `aegis-modules` `tests/modules.rs`: 16
  - `aegis-modules` `tests/runtime.rs`: 16
  - `aegis-social` `tests/social.rs`: 14
- `cargo clippy -p aegis-modules -p aegis-social --all-targets -- -D warnings` —
  clean
- `cargo fmt -p aegis-modules -p aegis-social -- --check` — clean

## Files changed

- `crates/aegis-modules/src/policy.rs` (new)
- `crates/aegis-modules/src/lib.rs`
- `crates/aegis-modules/src/tool_runner.rs`
- `crates/aegis-modules/src/runtime.rs`
- `crates/aegis-modules/tests/runtime.rs`
- `crates/aegis-social/src/groups.rs`
- `crates/aegis-social/src/lib.rs`
- `crates/aegis-social/tests/social.rs`
- `docs/implementation/BATCH6_AGENT09_SOCIAL_MODULES.md` (this file)

## 1. Module trust and execution policy

`crates/aegis-modules/src/policy.rs` introduces:

- `ModuleTrust { TrustedLocal, Restricted }` with `as_str`/`parse`. Trust is
  assigned by the Platform/owner and is never read from or promoted by a module
  manifest.
- `ExecutionPolicy { trust, max_runtime_ms, max_output_bytes, max_input_bytes,
  max_tool_count, allow_restricted }`.
- `validate()` rejects zero limits and any limit above the framework hard caps
  (`MAX_CPU_MILLIS`, `MAX_TOOL_OUTPUT_BYTES`, `MAX_TOOL_INPUT_BYTES`, `MAX_TOOLS`).
  `max_tool_count = 0` is valid and denies every declared tool.
- `effective_input_bytes()`, `effective_output_bytes()`, and
  `effective_runtime(requested)` clamp to the hard caps.
- Presets: `default()` = `trusted_local` (matches the pre-Batch 6 behavior);
  `for_trust(Restricted)` is tighter (60 s, 256 KiB output, 16 KiB input, 8
  tools) and always starts with `allow_restricted = false`.

Enforcement points:

- `run_module_tool_with_policy(...)` validates the policy, denies a restricted
  module unless `allow_restricted` is set, checks declared tool count, bounds
  input bytes, clamps the timeout, and bounds output bytes. If the module exits
  nonzero with a JSON failure report, that read is also bounded by the policy
  output limit.
- `run_module_tool(...)` delegates with `ExecutionPolicy::default()`, preserving
  the existing public signature used by `aegis-tools`.
- `ModuleRuntime::start_with_policy(...)` validates the policy, denies restricted
  modules without opt-in, checks declared tool count, and (when
  `max_runtime_ms` is tighter than the framework cap) spawns a supervisor
  watchdog that writes the stop sentinel, escalates to forced termination, and
  records a `failed` runtime row with an explicit policy reason.
- `ModuleRuntime::start(...)` delegates with the default policy.

No lifecycle transition, owner authorization, package-integrity, or manifest
check was relaxed.

## 2. Execution audit

`ModuleExecutionAudit` keeps all Batch 5 fields (`execution_summary` remains
available) and adds:

- `policy` — the policy applied to the projection.
- `restricted_count`, `denied_count`, `denied_modules`.
- `entries: Vec<ModuleAuditEntry>` with per-module `trust`, `policy`,
  `exit_outcome` (`ModuleExitOutcome`), `denied_reason`, and `last_error`.

`ModuleRuntime::execution_summary()` uses the default policy;
`execution_summary_with_policy(&policy)` validates the supplied policy first.
Denial reasons are derived from lifecycle state and policy (for example
`module is disabled; enable it before execution` or
`restricted module requires an explicit allow_restricted policy`).
`ModuleExitOutcome::from_runtime` distinguishes `not_started` from `stopped` via
the recorded `started_at` value. The audit stays read-only.

## 3. OS sandboxing is NOT implemented

True operating-system isolation is **not** implemented and is not claimed.

- No Windows Job Object, AppContainer, restricted token, or integrity-level
  isolation is used.
- No Linux namespace (`unshare`), seccomp filter, cgroup, or capability drop is
  used.
- There is no filesystem jail, network namespace/firewall, or memory/CPU
  enforcement by the OS.
- The process runtime provides cleared environment, no owner token/Platform
  secrets, working-directory containment, start/stop timeouts, heartbeat
  freshness, forced termination, and the new policy budgets. These are lifecycle
  and environment hygiene, **not** a security boundary against hostile code.

### Windows isolation options (would require new, approved dependencies)

- Job Objects (`AssignProcessToJobObject`, kill-on-close, memory/CPU limits) —
  commonly via `windows-sys`/`windows` crate; approval required for a new
  workspace dependency.
- AppContainer / restricted tokens / low integrity — same dependency family and
  a security review of capability SIDs.
- Windows Sandbox or a service account boundary — deployment/OS configuration,
  owner approval required.

### Linux isolation options (would require new, approved dependencies)

- `unshare` namespaces (PID/mount/net/user), `pivot_root`/`chroot` — usually via
  `nix` or raw `libc`; approval required.
- seccomp BPF filter — `libseccomp` bindings or raw syscalls; approval required.
- cgroups v2 for CPU/memory/IO — direct sysfs writes with owner-approved
  privileges.

None of the above may be added without explicit owner approval, a dependency
review, and a dedicated security acceptance test. Until then modules remain
trusted local code and `restricted` is an explicit, policy-level opt-in, not a
sandbox.

## 4. Social: per-Core participation summary

`GroupStore::participation_summary(core_uuid)` returns
`ParticipationSummary`:

- `group_count` (current memberships only), `message_count` (messages authored
  by that Core), `last_activity_at`, and `memberships`.
- Each `GroupParticipation` carries `group_id`, `group_name`, `joined_at`,
  `message_count` for that group, and `last_activity_at` (last authored message,
  or the join time if the Core has not posted).

Isolation properties:

- Membership is enforced in SQL by joining `social_group_members` for the
  requesting Core only. Removed members disappear immediately; non-members see
  an empty summary.
- The query never opens, joins, reads, or summarizes any Core's private
  `social.db`. Relationship `label`, `trust`, `notes`, and `tags` cannot leak.
- No trust, awareness, sentiment, or personality is inferred. This is the Batch
  5 `GroupActivitySummary` plus per-Core membership/participation counts.
- `activity_summary` from Batch 5 is unchanged and still member-scoped.

## 5. Real execution tests (not simulated)

Tests build and run the real `aegis-example-module` binary through
`CARGO_BIN_EXE_aegis-example-module`:

- `restricted_policy_denies_runtime_and_tools_until_explicitly_allowed` — real
  process start and tool execution, denied then allowed with explicit opt-in.
- `execution_policy_limits_are_enforced_and_clamped` — real input rejection,
  output rejection, runtime clamp via the slow `sleep` tool, tool-count
  rejection, invalid-policy rejection, and a successful default execution.
- `execution_audit_includes_policy_denial_and_exit_outcome` — audit entries
  across not-started, running, stopped, and disabled lifecycle states.
- `runtime_policy_stops_a_process_that_outlives_its_budget` — real process is
  stopped and recorded `failed` when it exceeds `max_runtime_ms`.
- Existing `disabled_or_uninstalled_module_cannot_run_a_tool` continues to prove
  that installed-but-disabled, disabled, and uninstalled modules cannot execute
  tools; crash and forced-stop tests still pass.

## Enforced limits (summary)

| Limit | Tool path | Process path |
| --- | --- | --- |
| `max_runtime_ms` | clamps requested timeout | watchdog stop + forced kill |
| `max_input_bytes` | rejects oversized serialized input | n/a |
| `max_output_bytes` | rejects oversized output (also on failure report) | n/a |
| `max_tool_count` | rejects manifest with more declared tools | rejects at start |
| restricted opt-in | `allow_restricted` required | `allow_restricted` required |
| lifecycle `enabled` | required | required |
| owner proof | unchanged (caller layer) | required |

## What remains unsandboxed

- Arbitrary filesystem/network/registry/process access by a spawned module.
- Memory, CPU, disk, and open-file limits declared in the manifest are validated
  fields only; they are not enforced by the OS.
- `wasm` entrypoints remain rejected by the runtime.
- No publisher signature/attestation verification; the manifest digest is
  content integrity only.

## Limitations

- Policy is provided by the caller; it is not persisted per module. Trust is an
  explicit owner decision, never inferred from the manifest.
- The runtime watchdog enforces a tighter runtime budget with stop-then-kill
  semantics; it does not constrain memory, CPU, handles, or syscalls.
- Participation summaries cover group-visible data only; per-Core relationship
  data is intentionally not included.
- Group counts are based on current membership rows; historical departures are
  not reconstructed.

## Next steps

1. Owner-approved decision on Windows Job Objects / AppContainer and Linux
   namespaces/seccomp/cgroups, with dependency review and a dedicated sandbox
   acceptance test.
2. If trust must be persisted, add an owner-assigned trust column/record through
   the module registry with a lifecycle audit event (requires owner approval).
3. Optional UI/TUI consumers for `participation_summary` and the extended
   execution audit entries.
4. Enforce manifest memory/CPU/disk limits only after an approved OS mechanism
   exists; do not emulate enforcement in-process.
