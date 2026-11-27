# Wave 2b — Consent Tools

Status: **IMPLEMENTED / TESTED**
Scope: `crates/aegis-tools/src/lib.rs` (specs + handlers),
`crates/aegis-tools/tests/consent_tools.rs` (new), and this document. No new
dependencies, workspace, CLI, or other crate changes were made.

## 1. Built-in tools

All consent tools are caller-bound: each handler opens
`ConsentStore::open(self.paths.root(), &self.caller_core)` and therefore only
ever reads or writes the calling Core's own `<core>/consent.db`. Every call goes
through the existing executor checks (grant, STOP, owner approval where
required) and is written to the Safe audit chain with no argument values.

| Tool | Sensitivity | Approval | Input |
| --- | --- | --- | --- |
| `consent.grant` | `PersonalWrite` | none | `{ scope, capability, participants?, context?, expires_at? }` |
| `consent.grant_intimate` | `Sensitive` | owner token | `{ scope, capability, participants?, context?, expires_at? }` |
| `consent.revoke` | `PersonalWrite` | none | `{ grant_id }` |
| `consent.list` | `ReadOnly` | none | `{ limit? }` |
| `consent.check` | `ReadOnly` | none | `{ scope, capability, participant? }` |

Default grants are seeded like the other built-ins (`initialize` inserts every
spec except `memory.create`). `consent.grant_intimate` remains owner-gated even
when granted because it is `Sensitive` and `approval_required`.

## 2. Semantics

### Scope partition

- `consent.grant` accepts only `conversation`, `object`, and `shared_project`.
  Any other scope (including `affection`, `romantic`, `adult`, or an unknown
  string) is rejected with `InvalidInput` and nothing is persisted.
- `consent.grant_intimate` accepts only `affection`, `romantic`, and `adult`.
  The other scopes are rejected with `InvalidInput`.
- `consent.check` accepts any known scope so an intimate grant recorded by
  `consent.grant_intimate` can be verified.

### Grant, check, revoke, list

- `consent.grant`/`consent.grant_intimate` create a fresh `grant_id` and a
  `ConsentGrant` bound to the caller Core UUID with `granted_at = now`, then
  call `ConsentStore::grant`. Duplicate/foreign/invalid grants are rejected by
  the store.
- `consent.revoke` calls `ConsentStore::revoke`; revocation is immediate and
  idempotent, and a revocation advances the stored revision.
- `consent.list` returns this Core's grants (including revoked) up to `limit`
  (default 50, capped at 256).
- `consent.check` returns `{ active: bool }`. With a `participant` it uses exact
  `scope + capability + participant` matching; without one it reports whether
  any active grant matches `scope + capability`. Revocation and expiry take
  effect immediately; consent is never inferred from trust or ownership.

### Intimate consent is recording only

`consent.grant_intimate` records consent. It does not execute any adult action;
adult action execution remains owner-gated and is not implemented here.

## 3. Isolation

Grant IDs are generated locally and grants live only in the caller Core's
`consent.db`. Core B's executor cannot list, check, or revoke Core A's grants:
`list` is empty, `check` is `false`, and `revoke` of A's `grant_id` fails
without altering A. The store itself also rejects foreign ownership rows.

## 4. Tests and results

Command: `cargo test -p aegis-tools` — **25 passed, 0 failed**:

- `tests/consent_tools.rs` — 6 tests: grant/check (participant exactness and
  participant-free check), immediate revoke (plus idempotent re-revoke), list +
  limit, non-intimate scope rejection, intimate approval gate then execution
  with the Platform owner token, and cross-Core isolation.
- Existing suites preserved: `creation_tools` 2, `module_tools` 6, `tools` 11.

Command: `cargo clippy -p aegis-tools --all-targets -- -D warnings` — **clean**.

## 5. Limitations

- Grant IDs use a timestamp/sequence/process identifier rather than a UUID
  because `aegis-tools` has no new dependency; they satisfy the contract ID
  rules and are unique in practice.
- `consent.list` returns stored grants, including revoked ones; callers use
  `consent.check`/`active` semantics for liveness.
- Expiry events are recorded lazily by the store when `check`/`active` observes
  an expired grant.
- Consent records are descriptive; enforcement of intimate actions remains
  outside this tool surface and is not implemented.
