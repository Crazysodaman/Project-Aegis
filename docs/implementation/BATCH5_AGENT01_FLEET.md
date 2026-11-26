# Batch 5 — Agent 01: Fleet & Artemis Integration

**Date:** 2026-11-25
**Repository:** `D:\Aegis`
**Baseline:** `2bd2513`
**Scope:** additive `aegis-fleet` changes only; lab development. No deployment,
no remote host changes, no commits.

**Status:** Implemented and tested. Artemis two-host verification is **blocked**
pending owner authorization (no Artemis install, enrollment, or credentials).

---

## 1. Architecture audit (read before changing)

`aegis-fleet` (crate version in the workspace) already provides:

| Area | Location | Notes |
|---|---|---|
| Host registry + policy | `src/lib.rs` | SQLite-backed `FleetStore` (`fleet_hosts`, `fleet_events`, `fleet_policy`), owner-gated `add/enroll/revoke/remove/probe`, `placements`, `route_inference`, GPU admission. |
| Health model | `src/lib.rs` (`HostHealth`) | `reachable`, `model_count`, `latency_ms`, `error`, `checked_at` (RFC 3339 from `aegis_storage::now_rfc3339`). No freshness helper existed. |
| Free-form capabilities | `src/lib.rs` (`FleetHost.capabilities: Value`) | `add_host` validates only that the value is `null` or an object ≤ 8192 bytes. No structured declaration. |
| Placement | `src/lib.rs` (`PlacementCandidate`) | Filtered on `enrolled && trusted && reachable`; no staleness. |
| GPU telemetry/admission | `src/gpu.rs` | `nvidia-smi` probe; fail-closed admission. |
| Yield supervisor | `src/supervisor.rs` | Stateful hysteresis, deferred work, recovery. |
| Gaming detection | `src/gaming.rs` | Process-list only; fail-closed. |
| Remote identity | `src/remote.rs` | HMAC-SHA256 nonce challenge/response; constant-time verify. |
| Identity binary | `src/bin/aegis-fleet-identity.rs` | `serve`/`verify` handshake harness. |
| Integration tests | `tests/fleet.rs` | Owner gating, probe, placement, routing, policy, GPU. |

Key constraints observed:

- `aegis-storage::now_rfc3339()` uses `chrono`, but `chrono` is **not** a
  dependency of `aegis-fleet`. Adding it would violate the "no new dependencies"
  rule, and `crates/aegis-fleet/Cargo.toml` is outside this agent's file
  ownership. Freshness parsing is therefore implemented locally with `std`.
- The SQLite schema already stores `health_checked_at`; no migration is needed.
- Existing public API must not break (other batches/tests rely on it).

---

## 2. What changed

All changes are additive or preserve existing signatures.

### 2.1 `src/freshness.rs` (new)

- `DEFAULT_HOST_STALE_AFTER_SECONDS = 300`.
- `HostHealth::probe_age_seconds(now)` — whole-second age from `checked_at`,
  clamped at zero for future/clock-skewed probes.
- `HostHealth::is_fresh(now, max_age)` / `HostHealth::is_stale(...)` — temporal
  freshness only; placement combines this with `reachable`.
- `HostStatus` — serializable per-host status (`host_id`, `callsign`, `role`,
  `enrolled`, `trusted`, `has_health`, `reachable`, `stale`,
  `last_probe_age_seconds`, `checked_at`, `eligible_for_placement`).
- `HostStatusSummary` — serializable aggregate: `total`, `enrolled`, `trusted`,
  `healthy`, `stale`, `unreachable`, `never_probed`, `eligible`, `as_of`,
  `max_age_seconds`, plus per-host entries.
- `host_status(...)` / `summarize_hosts(...)` — pure functions (no DB/network).
- Local RFC 3339 parser (`Z`, `z`, `±HH:MM`, optional fractional seconds) with a
  proleptic-Gregorian day conversion, plus leap-year/range validation.
  Fractional seconds are accepted but truncated to whole seconds.

### 2.2 `src/capabilities.rs` (new)

- `HostCapabilities { schema, roles, capabilities, inference }`.
- `HostCapabilities::new(roles, capabilities)` validates:
  - non-empty, duplicate-free, bounded roles (≤ 8);
  - capability labels trimmed/lowercased, ASCII `[-_.a-z0-9]`, non-empty,
    ≤ 48 chars, duplicate-free, ≤ 32 entries;
  - `inference` is **derived** from `roles` containing `HostRole::Inference`;
  - a declared `inference` capability without the inference role is rejected.
- `to_value()` serializes into the existing `FleetHost.capabilities: Value`;
  `from_value()` re-validates and rejects a value that advertises
  `inference: true` without the inference role.
- `supports_inference()`, `has_capability(...)`.

### 2.3 `src/lib.rs` (modified)

- Added `pub mod capabilities;` / `pub mod freshness;` and re-exports.
- `placements()` keeps its signature but now uses the current time and
  `DEFAULT_HOST_STALE_AFTER_SECONDS`.
- Added `placements_at(now, max_age_seconds)` — same filtering/sorting as
  `placements()`, with explicit time/freshness; a reachable but stale host is
  excluded.
- Added `host_summary()` and `host_summary_at(now, max_age_seconds)` — pure
  DB-read diagnostics using `summarize_hosts`.
- `route_inference` continues to use `placements()`, so it inherits staleness
  exclusion.

### 2.4 `tests/fleet.rs` (modified)

Added three integration tests:

1. `host_capabilities_validate_and_round_trip` — valid inference/general
   declarations, JSON round-trip, invalid declarations, borrowed inference
   rejection, and storage of a validated declaration via `add_host`.
2. `stale_hosts_are_excluded_from_placement_and_summary` — live mock probe,
   fresh summary/placement, then a future reference time makes the host stale:
   `stale = 1`, `healthy = 0`, `eligible = 0`, placements empty.
3. `unenrolled_host_is_never_eligible` — never-probed/untrusted host is counted
   as `never_probed` and is not eligible.

No existing test was removed or weakened.

---

## 3. Exact files changed

| File | Change |
|---|---|
| `crates/aegis-fleet/src/capabilities.rs` | **new** |
| `crates/aegis-fleet/src/freshness.rs` | **new** |
| `crates/aegis-fleet/src/lib.rs` | module wiring, re-exports, `placements_at`, `host_summary[_at]` |
| `crates/aegis-fleet/tests/fleet.rs` | 3 new integration tests |
| `docs/implementation/BATCH5_AGENT01_FLEET.md` | this document (new) |

Not touched: `crates/aegis-fleet/Cargo.toml` (no dependency changes),
`Cargo.lock`, workspace `Cargo.toml`, `crates/aegis-cli`, `.gitignore`,
`docs/completion`, any other crate, and `D:\SofiaDev\SofiaAdaLyra`.

---

## 4. Tests and results

Commands (run from `D:\Aegis`):

```powershell
cargo test -p aegis-fleet
cargo clippy -p aegis-fleet --all-targets -- -D warnings
```

Result (this run):

| Target | Result |
|---|---|
| `src/lib.rs` unit tests | **31 passed**, 0 failed |
| `src/bin/aegis-fleet-identity.rs` | 0 tests |
| `tests/fleet.rs` integration | **12 passed**, 0 failed |
| Doc-tests | 0 |
| **Total** | **43 passed, 0 failed** |
| `cargo clippy -p aegis-fleet --all-targets -- -D warnings` | **clean (exit 0)** |

The live GPU test remains env-gated (`AEGIS_GPU_LIVE=1`) and reports a skip
(pass) without live hardware; this is pre-existing behavior.

---

## 5. Artemis prerequisites (observed, non-invasive)

From the non-invasive discovery recorded in Batch 4 (not repeated here as
remote actions):

- NetBIOS/host name: `ARTEMIS`, address `192.168.1.55`, `WORKGROUP`.
- Open TCP: 135, 139, 443, 445 (SMB), 3389 (RDP), 5985 (WinRM).
- Closed/absent: 22 (SSH), 80, 8080, 9090, 11434 (no Ollama), no Fleet agent.
- No credentials were used, extracted, guessed, or brute-forced.
- No remote execution, install, reboot, firewall change, or service change was
  performed by this agent.

Implication: Artemis currently **cannot** serve inference (no Ollama/11434). It
must be declared as `HostRole::General`/`Storage` with `HostCapabilities` that do
**not** advertise inference until an inference runtime is intentionally installed
and authorized. This implementation never advertises inference for a host that
does not declare the inference role.

---

## 6. Owner authorization gate

Installing anything on Artemis, enabling enrollment, opening new ports, or
provisioning a shared secret requires explicit **owner authorization**. This
agent did not perform any of these. The gate for a future authorized run:

- Named owner approval (recorded with date/time and scope).
- Approved maintenance window and rollback owner.
- Physical/authorized-console access to Artemis (or WinRM/RDP use explicitly
  authorized with owner-provided credentials).
- Confirmation of whether Artemis is intended as inference (Ollama needed) or
  general/storage (no inference advertisement).
- Out-of-band shared-secret delivery for `AEGIS_REMOTE_SECRET` (≥ 16 bytes;
  never passed on the command line or logged).
- Agreement that the run is lab-only and not production activation.

---

## 7. Runbook (for owner-authorized execution later)

> Not executed. Each step must be explicitly authorized first.

1. **Record authorization.** Capture approver, scope, window, and rollback owner
   in the change log. Confirm lab-only intent.
2. **Back up Venus Fleet state.** Copy the Platform SQLite DB and `owner.proof`
   to a safe location (do not commit).
3. **Decide Artemis role.** Inference (requires Ollama + models + TCP 11434) vs.
   General/Storage (no inference). Default: General/Storage.
4. **Provision Artemis (only if authorized).** On Artemis via the approved
   console/WinRM/RDP path, install the chosen runtime/agent. Do not open ports
   beyond those needed. Do not enable 11434 unless inference is chosen.
5. **Provision secret.** Place the shared secret in `AEGIS_REMOTE_SECRET` on both
   hosts out of band; verify it is at least 16 bytes and never echoed.
6. **Register the host on Venus.** Using the owner token, call
   `FleetStore::add_host(...)` with a validated `HostCapabilities::to_value()`
   (General/Storage unless inference is authorized). The host starts untrusted.
7. **Enroll explicitly.** Call `FleetStore::enroll(host_id, owner_token)`.
   Trust changes are immediate and audited in `fleet_events`.
8. **Probe and inspect freshness.** Call `FleetStore::probe(host_id)`, then
   `FleetStore::host_summary()`; confirm `stale = 0`, `unreachable = 0`, and
   `eligible` matches intent. Re-probe before relying on placement.
9. **Verify identity handshake.** On Venus run
   `aegis-fleet-identity verify --connect <artemis-agent-addr> --expect-host <uuid>`
   with `AEGIS_REMOTE_SECRET` set on both sides; success is `verified=true`.
10. **Verify placement.** `FleetStore::placements()` should include Artemis only
    if enrolled, trusted, reachable, and fresh. If inference was authorized,
    `route_inference("<model>")` must find the model on Artemis.
11. **Capture evidence.** Store `host_summary` JSON, probe result, identity
    verification output, and placement output in the batch evidence log.
12. **Rollback on any failure.** `revoke(host_id, owner_token)`, then
    `remove(host_id, owner_token)`; stop/remove the Artemis agent; close any added
    ports; restore the Venus DB backup.

---

## 8. Explicit limitations / statements

- **No two-host Artemis verification was performed.** Artemis was not installed,
  enrolled, probed, or otherwise changed in this batch. The Batch 4 Artemis
  registry entry was `reachable=false` and excluded from placement; no real
  Artemis authenticated inference exists.
- No remote host, firewall, credential, or service was modified.
- `add_host` still accepts free-form `Value` capabilities for backward
  compatibility; validated declarations are opt-in via `HostCapabilities`. A
  caller can still store an inconsistent raw object directly; the helper is the
  supported path and the tests cover it.
- Freshness is whole-second, default 300 s; fractional seconds are truncated.
  Callers that need a different window use `_at` variants.
- The local RFC 3339 parser accepts `Z`/`z`/numeric offsets and rejects malformed
  dates; it does not implement leap seconds beyond allowing `:60`.
- No CLI diagnostics were added because `crates/aegis-cli` is outside this
  agent's ownership. Wiring `host_summary`/`placements_at` into CLI output should
  be done by the owner of that crate.
- No schema migration was required; `health_checked_at` already exists.

---

## 9. Next steps

1. Owner authorization for Artemis install/enrollment (gate above).
2. Wire `host_summary()` / `placements_at()` into `aegis-cli` diagnostics by the
   CLI owner, if desired.
3. After an authorized run, execute a real **two-host** Venus↔Artemis
   authenticated inference test and record evidence; treat this document as not
   having done so.
4. Optionally persist a per-fleet stale window in `fleet_policy` (would require a
   schema addition in a future batch).
