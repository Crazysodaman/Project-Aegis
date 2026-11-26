# Batch 7 — Agent 09: Module Isolation, Recovery & Social Enforcement

Baseline: `2bd2513`. All work is additive and restricted to
`crates/aegis-modules/**`, `crates/aegis-social/**`, and this document. No
workspace manifest, `Cargo.lock`, CLI, or other crate was modified. No new
dependencies were added and no `unsafe` code was introduced. Pre-existing
uncommitted Batch 5/6/7 work was preserved.

## Status

Complete and verified with package-scoped gates:

- `cargo test -p aegis-modules -p aegis-social` — **68 passed, 0 failed**
  - `aegis-modules` lib unit tests: 14 (5 policy, 5 manifest, 4 sandbox)
  - `aegis-modules` `tests/isolation.rs`: 3
  - `aegis-modules` `tests/modules.rs`: 16
  - `aegis-modules` `tests/runtime.rs`: 18
  - `aegis-social` `tests/social.rs`: 17
  - Doc-tests: 0
- `cargo clippy -p aegis-modules -p aegis-social --all-targets -- -D warnings` —
  clean
- `cargo fmt -p aegis-modules -p aegis-social -- --check` — clean
- No `aegis-example-module` child process was left running after the test run
  (checked with `tasklist`).

## Files changed

- `crates/aegis-modules/src/sandbox.rs` (new)
- `crates/aegis-modules/src/lib.rs`
- `crates/aegis-modules/src/runtime.rs`
- `crates/aegis-modules/src/bin/aegis-example-module.rs`
- `crates/aegis-modules/tests/runtime.rs`
- `crates/aegis-modules/tests/isolation.rs` (new)
- `crates/aegis-social/tests/social.rs`
- `docs/implementation/BATCH7_AGENT09_SOCIAL_MODULES.md` (this file)

`crates/aegis-social/src/**` was reviewed and left unchanged: Batch 6 membership
enforcement and the per-Core participation summary were already correct. The
Batch 7 social work is stronger adversarial test coverage.

## 1. Honest isolation capability probe and sandbox abstraction

`crates/aegis-modules/src/sandbox.rs` introduces:

- `IsolationSupport` with `probe()`. It reports, at compile time:
  - `os_sandbox = false`, `filesystem_jail = false`, `network_isolation = false`,
    `memory_limit = false`, `cpu_limit = false` on Windows and Linux.
  - `process_tree_termination = cfg!(windows)` (Windows uses `taskkill /T /F`;
    Unix builds only signal the direct PID).
  - `cleared_environment = true` and `package_working_directory = true` because
    the runtime demonstrably does both.
  - `unsupported_mechanisms()` lists the missing OS controls and
    `is_unsandboxed()` is `true` until a real sandbox exists.
- `ModuleSandbox` trait (`support`, `policy`, `apply`). Its `apply` hook is where
  a real OS mechanism would attach.
- `NoopSandbox`, the current baseline. It returns the same honest probe and
  returns `SandboxError::Unsupported` from `apply`, so it can never claim that
  enforcement happened.
- `SandboxPolicy` + `FilesystemScope`, pure data describing what a future
  sandbox *would* enforce: package read scope, runtime-scratch write scope,
  network/child-process permissions, optional OS memory/CPU caps, and descendant
  termination. It enforces nothing by itself.

The probe is deliberately a crate-owned constant, not a host probe, so it cannot
drift into a false claim about facilities the runtime does not use.

## 2. Resource limits, timeout enforcement, and process-tree termination

Reviewed `policy.rs`, `runtime.rs`, and `tool_runner.rs`; existing enforcement is
unchanged and still correct:

- Tool path: `max_input_bytes`, `max_output_bytes` (including bounded failure
  reports), `max_tool_count`, and a policy-clamped timeout that kills the child.
- Service path: `start_with_policy` validates the policy, denies restricted
  modules without `allow_restricted`, checks declared tool count, and the
  watchdog writes the stop sentinel then forces termination when
  `max_runtime_ms` is exceeded.
- Lifecycle `enabled`, owner proof, manifest digest, and package integrity checks
  are untouched.

Improvement: forced stop on Windows now uses `taskkill /PID <pid> /T /F`.
`/T` terminates descendants started by the module; `/F` forces them down. This is
process cleanup, **not** a security boundary — the child still runs with the
Platform user's full rights (network, filesystem, registry, etc.).

New real-process test: `forced_stop_terminates_the_module_process_tree` starts
the real example module with `--ignore-stop --spawn-child`, waits for it to
report a live child PID, forces a stop, and asserts both parent and child are
gone. A `Drop` guard runs `taskkill /T /F` so a test panic cannot leak children.

## 3. Module recovery after a crash

`ModuleRuntime::recover` / `recover_with_policy` add an explicit recovery path,
separate from `restart`:

- Requires owner proof and an `enabled` lifecycle state. A module that was not
  explicitly enabled yields `NotEnabled`.
- Rejects a running or starting module (`AlreadyRunning`) and any other
  non-recoverable state (`NotRecoverable`).
- Only `failed`, `stale`, or `stopped` runtime states are recoverable.
- Validates the execution policy and applies the restricted opt-in gate exactly
  like `start_with_policy`.
- Stops first (cleaning a lingering PID through the normal stop path) and then
  starts under the supplied policy. It never re-enables, reinstalls, or bypasses
  authorization.

Real test: `crashed_module_can_be_recovered_after_explicit_owner_action` uses a
new `--fail-once-ms` example-module switch (fails once, leaves a marker, then
runs normally). The test proves: recovery is denied before enable, a real crash
is observed (`failed`, dead PID, error recorded), a bad owner token is denied, a
successful recovery returns to `running`, and recovering a running module is
rejected.

## 4. Social: membership enforcement and private-group isolation

Batch 6 already provided the per-Core `participation_summary`; it is complete and
unchanged. Batch 7 adds adversarial coverage:

- `removed_member_is_denied_immediately_across_all_operations` — after removal,
  `is_member`, `list_groups`, `group`, `members`, `messages`, `message`,
  `context_for_core`, `activity_summary`, `events`, `post_message`, and
  `participation_summary` all deny/exclude the removed Core immediately, while
  remaining members and the owner keep access.
- `non_members_cannot_read_membership_or_events` — non-members cannot read the
  member roster, events, activity summary, or group metadata.
- `private_relationship_fields_never_leak_through_group_reads` — sets private
  relationship label/trust/notes/tags for both Cores, then serializes every
  group projection (group, members, messages, events, activity, participation,
  context) for each Core and asserts the other Core's group content and all
  private relationship markers are absent. It also asserts both Cores are denied
  each other's private group.

## Trusted vs restricted (current truth)

| Concern | `trusted_local` | `restricted` |
| --- | --- | --- |
| Must be explicitly enabled | yes | yes |
| Explicit `allow_restricted` opt-in | not required | **required** |
| Runtime budget | up to framework cap | tighter preset (60 s default) |
| Input/output byte budget | framework caps | tighter preset |
| Declared-tool budget | framework cap | tighter preset |
| OS sandbox | none | none |

`restricted` is a conservative policy preset with an explicit owner opt-in. It
is **not** a sandbox and does not reduce the module's OS privileges.

## Enforced vs not enforced

Enforced today:

- Cleared child environment, no owner token or Platform secrets in the child.
- Working directory set to the module package; executable must resolve inside
  `<root>/modules/<module_id>` and must not be redirected.
- Start timeout, heartbeat freshness, stop timeout, forced termination.
- Windows forced stop terminates the process tree (`taskkill /T /F`).
- Policy budgets: runtime (tool timeout clamp + service watchdog), input bytes,
  output bytes, declared-tool count, restricted opt-in.
- Lifecycle, owner proof, manifest digest, and package asset integrity.

Not enforced (no OS sandbox, no false claim):

- Filesystem jail, network isolation, memory/CPU/handle limits. Manifest
  `resources` limits remain validated fields only.
- Process-tree cleanup on Unix only signals the direct PID (reported honestly by
  the probe as unsupported).
- Publisher signatures/attestation; the manifest digest is content integrity
  only.
- `wasm` entrypoints remain rejected by the runtime.

## OS isolation options (owner-approved mechanisms only)

None of these are implemented or added as dependencies:

- **Windows:** Job Objects (kill-on-close, memory/CPU limits), AppContainer or
  restricted/low-integrity token, Windows Sandbox/service account boundary.
  Typically needs `windows-sys`/`windows` — a new dependency requiring approval.
- **Linux:** `unshare` namespaces (PID/mount/net/user), `pivot_root`/`chroot`,
  seccomp BPF, cgroups v2. Typically needs `nix`/`libc` bindings or
  owner-approved privileged sysfs writes.

Any of these requires owner approval, a dependency review, and a dedicated
security acceptance test; `IsolationSupport` and `NoopSandbox` must not claim
support until such enforcement is demonstrated.

## Limitations

- Policy is caller-supplied; trust is not persisted per module.
- Recovery restarts a module but does not repair a broken package or re-verify
  assets beyond the checks the normal start path already performs.
- Windows process-tree termination is best-effort cleanup; a descendant that
  breaks away from the tree is not guaranteed to be terminated.
- Participation summaries cover group-visible data only; per-Core relationship
  data is intentionally never included.
- Group counts reflect current membership; historical departures are not
  reconstructed.

## Next steps

1. Owner decision on a real Windows/Linux sandbox mechanism with dependency
   review and a sandbox acceptance test; update `IsolationSupport` only then.
2. Persist owner-assigned trust per module with a lifecycle audit event if needed
   (owner approval required).
3. Enforce manifest memory/CPU/disk limits only after an approved OS mechanism
   exists; do not emulate enforcement in-process.
4. Optional UI/TUI surfaces for `IsolationSupport`, `recover`, and the extended
   social isolation projections.
