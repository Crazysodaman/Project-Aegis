# Batch 7 — Agent 01: Fleet Agent Plan, Doctor Readiness, and Live HMAC Re-verification

**Date:** 2026-11-26
**Repository:** `D:\Aegis` (Windows PowerShell)
**Baseline:** `2bd2513` with large pre-existing uncommitted Batch 3–6 work preserved
**Scope:** additive `crates/aegis-fleet` changes only. Lab development.
**Status:** Implemented, tested, clippy-clean.

> **Artemis is NOT operational and was never contacted.** No install, enrollment,
> credential creation, firewall/service change, or remote credential use was
> performed. Every Artemis/remote step remains **BLOCKED** pending explicit owner
> approval. The live HMAC check below ran entirely between local processes on
> this host's own LAN interface.

---

## 1. Constraints honored

- Only `crates/aegis-fleet/src/**`, `crates/aegis-fleet/tests/**`, and this
  document were created/edited.
- Pre-existing uncommitted Batch 3–6 work was preserved. No commit, deploy,
  reset, rebase, clean, or other destructive git operation.
- No changes to `crates/aegis-cli`, workspace `Cargo.toml`, `Cargo.lock`,
  `.gitignore`, `docs/completion`, other crates, or `D:\SofiaDev\SofiaAdaLyra`.
- No new dependencies. `crates/aegis-fleet/Cargo.toml` was not modified by this
  agent (it still shows its pre-existing Batch 6 modification).
- No installation, enrollment, credential, firewall, or service change on
  Artemis; no remote credentials used.

Files touched by this agent:

| File | Change |
|---|---|
| `crates/aegis-fleet/src/agent_plan.rs` | **New.** `FleetAgentPlan` (package-as-data) + `preflight_agent_plan`. |
| `crates/aegis-fleet/src/doctor.rs` | **New.** `run_doctor` combined readiness report. |
| `crates/aegis-fleet/src/lib.rs` | Added `agent_plan`/`doctor` modules and re-exports. |
| `crates/aegis-fleet/src/deployment.rs` | Visibility only: `finding`/`transport_finding` made `pub(crate)` for reuse. No behavior change. |
| `crates/aegis-fleet/tests/live_identity.rs` | **New.** Opt-in live two-process HMAC re-verification over the LAN interface. |
| `docs/implementation/BATCH7_AGENT01_FLEET.md` | This document. |

---

## 2. Audit of Batch 6 Fleet code (read before changing)

| Area | Location | Observed behavior (reused, not modified) |
|---|---|---|
| Host registry | `src/lib.rs` | SQLite `FleetStore` (`fleet_hosts`, `fleet_events`, `fleet_policy`); owner-gated `add_host`/`enroll`/`revoke`/`remove`; hosts start untrusted. |
| Transport policy | `src/transport.rs` | `EndpointClass::{Loopback, PrivateLan, Public}`; `TransportPolicy`; `validate_endpoint_security` rejects public plain HTTP unless explicitly opted in. |
| Deployment | `src/deployment.rs` | `evaluate_deployment_readiness`, secret-free `render_agent_config`; accepts only a `secret_provisioned` boolean. |
| Capabilities | `src/capabilities.rs` | `HostCapabilities` derives `inference` only from the inference role; `from_value` rejects borrowed inference advertisement. |
| Freshness | `src/freshness.rs` | RFC 3339 age/staleness helpers on `HostHealth`. |
| Remote identity | `src/remote.rs` + `src/bin/aegis-fleet-identity.rs` | HMAC-SHA256 nonce challenge/response, constant-time verify, secret from `AEGIS_REMOTE_SECRET` (never argv); `serve`/`verify` modes. |
| GPU / supervisor / gaming / measurement / performance | respective modules | Unchanged. |

Gaps addressed (all additive):

1. No single deployment **package-as-data** describing exact files, env var
   names, argv, working dir, verification, and rollback for an agent.
2. No preflight tying binary presence + transport class + role/capability
   consistency + secret indicator together.
3. No combined `doctor` view of transport policy + deployment readiness + host
   record.
4. Remote-identity verification existed but had not been re-exercised live over
   the actual LAN interface in this batch.

---

## 3. What changed

### 3.1 `src/agent_plan.rs` — `FleetAgentPlan` (deployment package as data)

`FleetAgentPlan` is inert, serializable data describing an Artemis agent deploy:

- `files: Vec<AgentFileSpec>` — exact files to copy (`description`, `source`,
  `destination`, `required`). Always includes the identity binary; optionally a
  secret-free config file.
- `env: Vec<AgentEnvSpec>` — environment variable **names only** (`name`,
  `required`, `secret`, `description`). No values.
- `argv: Vec<String>` — the `serve` argument vector.
- `working_dir: String`.
- `verification: Vec<AgentStep>` — identity handshake (`verified=true`) and
  wrong-secret rejection.
- `rollback: Vec<AgentStep>` — stop process, revoke host, remove host record,
  delete config/binary.
- `contains_secret_values: bool` — always `false`.

Builder: `build_agent_plan(host: &FleetHost, options: &AgentPlanOptions) ->
Result<FleetAgentPlan, FleetError>` validates host id, callsign, env var name,
paths, bind/connect atoms, and a non-zero accept budget. Deterministic output.

`preflight_agent_plan(&AgentPreflightInput) -> AgentPreflight` checks:

- **Agent binary presence** — `Path::is_file()` on the caller-supplied binary
  path (`agent_binary_missing` blocker if absent).
- **Endpoint class via `TransportPolicy`** — `validate_endpoint_security`;
  records `endpoint`, `endpoint_class`, `transport_accepted`; emits
  `transport_rejected` on failure and `endpoint_missing` when empty.
- **Role/capability consistency** — inference is never advertised without a
  policy-accepted endpoint (`inference_without_endpoint` blocker). Role/cap
  mismatches are non-blocking warnings, matching the existing deployment module.
- **Secret-provisioned indicator** — a boolean only; the report names the env
  var (default `AEGIS_REMOTE_SECRET`) and never any value. A plan that claims
  `contains_secret_values` is a blocker.

### 3.2 `src/doctor.rs` — combined readiness report

`run_doctor(&DoctorInput) -> DoctorReport` composes:

- transport assessment (`transport_accepted`, `endpoint_class`,
  `transport_error`),
- deployment readiness (nested `deployment: DeploymentReadiness`),
- host record state (`host_record_present`, `enrolled`, `trusted`,
  `health_reachable`, `health_stale`), and
- optional agent-plan preflight (nested `preflight: AgentPreflight`).

Blockers/warnings are deduplicated by stable code, blockers first. Health
conditions (`host_never_probed`, `host_unreachable`, `host_stale`) are warnings;
missing record/enrollment/trust/transport/secret are blockers. `ready` is true
only when there are no blockers. The function is pure except the preflight's
binary-presence `is_file()` check; it never touches the network and never reads
a secret.

---

## 4. Tests and counts

Command:

```powershell
cargo test -p aegis-fleet
```

Result (default, offline/gated run):

| Test target | Passed |
|---|---|
| `src/lib.rs` unit tests | 90 |
| `tests/fleet.rs` | 13 |
| `tests/live_identity.rs` (skipped unless opted in) | 1 |
| `tests/measurement.rs` | 3 |
| `tests/performance.rs` | 3 |
| Doc-tests | 0 |
| **Total** | **110 passed, 0 failed** |

New coverage added by this batch: 8 `agent_plan` unit tests, 7 `doctor` unit
tests, and 1 live integration test (16 new tests). Unit tests cover: deterministic
secret-free plan, optional config file, invalid inputs, ready preflight, missing
binary/secret, inference-without-endpoint, rejected public endpoint (blocked vs
explicit opt-in), role/capability warnings, healthy/missing-host doctor reports,
health warnings, transport blocker, secret non-disclosure, and preflight/doctor
merge-and-deduplicate behavior.

Lint gate:

```powershell
cargo clippy -p aegis-fleet --all-targets -- -D warnings
```

Result: `Finished` with no warnings or errors.

---

## 5. Live HMAC identity evidence (LAN interface, no Artemis)

Vehicle: `tests/live_identity.rs` spawns the real `aegis-fleet-identity` binary in
`serve` mode on this host's LAN address and runs `verify` as separate processes.
The secret is passed only through `AEGIS_REMOTE_SECRET`, never argv.

Opt-in command:

```powershell
$env:AEGIS_FLEET_LIVE_HMAC = "1"
$env:AEGIS_FLEET_LAN_BIND = "192.168.1.71:0"   # :0 picks a free port
cargo test -p aegis-fleet --test live_identity -- --nocapture
```

Observed result (2026-11-26):

```text
live identity serve listening on 192.168.1.71:52947
live identity valid secret: {"callsign":"Venus","host_uuid":"11111111-1111-4111-8111-111111111111","nonce":"6b3caaab-7cac-4ea8-bf22-8af767c974ff","schema":"aegis.fleet.remote_verify.v1","verified":true}
live identity wrong secret rejected: aegis-fleet-identity: remote identity proof is invalid
test live_hmac_identity_over_lan_interface ... ok
test result: ok. 1 passed; 0 failed
```

Independent manual CLI re-run (same host, LAN interface, separate processes):

```powershell
# serve (secret in env only)
$env:AEGIS_REMOTE_SECRET = '<redacted 32-byte secret>'
aegis-fleet-identity serve --bind 192.168.1.71:52960 `
  --host-uuid 11111111-1111-4111-8111-111111111111 --callsign Venus --accepts 2
# -> {"listening":"192.168.1.71:52960","accepts":2}

# valid secret
$env:AEGIS_REMOTE_SECRET = '<same secret>'
aegis-fleet-identity verify --connect 192.168.1.71:52960 `
  --expect-host 11111111-1111-4111-8111-111111111111
# -> {"callsign":"Venus","host_uuid":"...","schema":"aegis.fleet.remote_verify.v1","verified":true}
#    exit code 0

# wrong secret
$env:AEGIS_REMOTE_SECRET = '<different 32-byte secret>'
aegis-fleet-identity verify --connect 192.168.1.71:52960 `
  --expect-host 11111111-1111-4111-8111-111111111111
# -> aegis-fleet-identity: remote identity proof is invalid
#    exit code 1
```

The `serve` process exited after its 2-accept budget; no processes were left
running (verified with `Get-Process aegis-fleet-identity`, empty).

Interpretation: the HMAC challenge/response and constant-time verification work
correctly across real sockets on the LAN interface. This proves the identity
mechanism only; it does **not** prove Artemis is reachable or operational.

---

## 6. Owner approval checklist (required before any Artemis action)

Artemis remains **BLOCKED** until the owner explicitly approves, in writing,
each of the following:

- [ ] Approve installing/copying the agent binary to Artemis.
- [ ] Approve creating/provisioning the shared secret out of band on both hosts
  (never transmitted by this tooling, never in argv or logs).
- [ ] Approve registering the Artemis host record and explicitly enrolling it
  with the owner token.
- [ ] Approve any firewall/listener change needed for the chosen bind address
  and port.
- [ ] Approve running the identity `serve` process on Artemis.
- [ ] Confirm the appropriate transport: loopback/private-LAN plain HTTP, or a
  TLS terminator for any public endpoint (the bundled adapter cannot speak TLS).
- [ ] Confirm rollback ownership and the exact revoke/remove sequence.
- [ ] Confirm a maintenance window and who will observe the two-host test.

No step above has been performed.

---

## 7. Two-host test plan (post-approval only)

Preconditions: owner approval for all checklist items; secret provisioned on both
hosts; host record created (untrusted) and then explicitly enrolled.

1. **Static preflight (safe, offline):** build the plan and run
   `preflight_agent_plan` locally with the real binary path and a
   `secret_provisioned = true` indicator. Require `ready == true`; resolve all
   blockers and warnings first.
2. **Transport review:** run `run_doctor` with the stored host record and strict
   `TransportPolicy`. Require no blockers; export the report for the audit trail.
3. **Deploy on Artemis (owner present):** copy the files listed in
   `FleetAgentPlan.files`, set the named env var out of band, start the `serve`
   process with the exact `argv` and `working_dir`.
4. **Identity check from this host:**
   `aegis-fleet-identity verify --connect <artemis-lan-addr>:<port> --expect-host <uuid>`.
   Require `verified=true`.
5. **Negative check:** repeat with a wrong secret; require non-zero exit and
   `proof is invalid`.
6. **Registry check:** `FleetStore::probe` the Artemis endpoint; confirm
   `host_summary` shows it enrolled, trusted, healthy, and fresh.
7. **Placement (optional, separate approval):** only if Artemis advertises
   inference *and* has a policy-accepted endpoint.
8. **Record:** capture exact commands, timestamps, host ids, and results in a
   batch evidence document.

Any failure stops the plan; proceed to rollback.

---

## 8. Rollback

Use the `FleetAgentPlan.rollback` steps (owner-gated):

1. Stop the `aegis-fleet-identity serve` process on Artemis.
2. `FleetStore::revoke(host_id, owner_token)` — trust removed immediately.
3. `FleetStore::remove(host_id, owner_token)` — host record deleted.
4. Delete the copied config file (if any) and binary from Artemis.
5. Rotate/discard the shared secret if it was ever exposed.
6. Re-run `run_doctor` from this host and confirm the host is gone and no stale
   placement candidates remain.

Rollback was not exercised against Artemis because Artemis was never touched.

---

## 9. Blocked items / non-claims

- Artemis is **not** operational; no remote action was taken.
- No credentials were created, stored, or used.
- No two-host test was run; the live HMAC check above is local-interfaces only.
- Network placement/inference against Artemis is not validated.
- Public inference will require a TLS terminator; the bundled adapter is HTTP-only.

## 10. Next steps

1. Owner completes the approval checklist and schedules the two-host window.
2. After approval, execute section 7 and record results in a new evidence file.
3. If Artemis will serve inference, first establish a policy-accepted endpoint
   (private LAN or TLS-fronted) so preflight/doctor stay green.
