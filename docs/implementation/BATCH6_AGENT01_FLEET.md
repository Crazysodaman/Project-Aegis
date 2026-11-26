# Batch 6 â€” Agent 01: Fleet Transport Security & Artemis Readiness

**Date:** 2026-11-25
**Repository:** `D:\Aegis`
**Baseline:** `2bd2513` (large pre-existing uncommitted Batch 5 work preserved)
**Scope:** additive `crates/aegis-fleet` changes only; lab development. No
commit, deployment, remote action, or credential use.

**Status:** Implemented, tested, clippy-clean. **All Artemis/remote steps are
BLOCKED pending explicit owner approval.** No two-host or Artemis verification
was performed.

---

## 1. Constraints honored

- Only `crates/aegis-fleet/src/**`, `crates/aegis-fleet/tests/**`, and this
  document were edited.
- No changes to `crates/aegis-cli`, workspace `Cargo.toml`, `Cargo.lock`,
  `.gitignore`, `docs/completion`, other crates, or `D:\SofiaDev\SofiaAdaLyra`.
- No new dependencies; `crates/aegis-fleet/Cargo.toml` was not modified by this
  agent (its pre-existing uncommitted changes were left untouched).
- No commit, deploy, reset, rebase, clean, or other destructive git operation.
- No installation, enrollment, firewall/service/credential change, or remote
  credential use on Artemis. Artemis remains blocked.

---

## 2. Architecture audit (Batch 5 Fleet, read before changing)

| Area | Location | Observed behavior |
|---|---|---|
| Host registry | `src/lib.rs` | SQLite `FleetStore` (`fleet_hosts`, `fleet_events`, `fleet_policy`); owner-gated `add_host` / `enroll` / `revoke` / `remove`; hosts start untrusted. |
| Endpoint validation | `src/lib.rs` (`validate_endpoint`) | Accepts only `http://`, rejects `https://` (adapter has no TLS), rejects userinfo (`@`) and control chars. No network-class policy existed. |
| Health / freshness | `src/lib.rs`, `src/freshness.rs` | Probe-based `HostHealth`; RFC 3339 age helpers; `placements_at`/`host_summary_at` treat health older than `DEFAULT_HOST_STALE_AFTER_SECONDS` (300 s) as stale. |
| Capabilities | `src/capabilities.rs` | `HostCapabilities` derives `inference` only from the declared inference role; `from_value` rejects borrowed inference advertisement. Free-form raw `Value` is still accepted by `add_host` for compatibility. |
| Remote identity | `src/remote.rs` | HMAC-SHA256 nonce challenge/response; constant-time verify; shared secret supplied out of band (`AEGIS_REMOTE_SECRET`). |
| GPU / supervisor / gaming | `src/gpu.rs`, `src/supervisor.rs`, `src/gaming.rs` | Fail-closed telemetry/admission and stateful yield. Unchanged. |
| Integration tests | `tests/fleet.rs`, `tests/performance.rs` | Registry, probe, placement, policy, capabilities, staleness. |

Gaps found for this batch:

1. No explicit endpoint **transport class** (loopback / private-LAN / public)
   and no policy gate preventing public plain-http inference endpoints.
2. No pure pre-flight **deployment readiness** report for a new agent.
3. No safe, secret-free **agent configuration template** generator.

All three are additive; existing APIs and behavior are unchanged.

---

## 3. What changed

### 3.1 `src/transport.rs` (new) â€” endpoint transport classification + policy

- `EndpointClass::{Loopback, PrivateLan, Public}`.
- `TransportScheme::{Http, Https}`.
- `classify_endpoint(endpoint) -> Result<ParsedEndpoint, TransportError>`:
  offline, syntactic parsing only (no DNS).
  - Loopback: `127.0.0.0/8`, `::1`, IPv4-mapped loopback, `localhost`,
    `*.localhost`.
  - Private-LAN: RFC 1918 (`10/8`, `172.16/12`, `192.168/16`), link-local
    (`169.254/16`, `fe80::/10`), IPv6 unique-local (`fc00::/7`), `.local`/`local`
    mDNS names, IPv4-mapped private addresses.
  - Public: everything else, including `0.0.0.0` and `::`.
  - Parses bracketed IPv6, optional port, strips path/query/fragment, rejects
    credentials/userinfo, controls, unsupported schemes, zero/invalid ports, and
    unbracketed IPv6.
- `TransportPolicy { allow_insecure_public: bool }` (default/strict = `false`);
  `TransportPolicy::allow_insecure_public()` and
  `with_allow_insecure_public(bool)` for the explicit lab escape hatch.
- `validate_endpoint_security(endpoint, policy) -> Result<EndpointSecurity, TransportError>`:
  - **Rejects public plain-http** (`TransportError::PublicPlainHttp`) unless
    `allow_insecure_public` is set.
  - **Allows loopback/private-LAN plain http by default** (homelab).
  - Allows `https` for any class, but reports `adapter_supports_transport =
    false` because the bundled Ollama adapter speaks only HTTP.
  - `EndpointSecurity` records schema, endpoint, scheme, class, `tls_fronted`,
    `requires_tls_fronting`, `adapter_supports_transport`.
- Documented constant `OLLAMA_ADAPTER_TRANSPORT_NOTE`: the built-in adapter only
  speaks plain http and cannot terminate TLS; a public endpoint must be fronted
  by a TLS terminator addressed over http on loopback/private-LAN.

### 3.2 `src/deployment.rs` (new) â€” readiness report + safe template

**Deployment readiness (pure):**

- Input `DeploymentInput { callsign, host: Option<&FleetHost>, transport_policy,
  secret_provisioned: bool }`. Only a **boolean** secret indicator is accepted;
  a secret value can never reach this module.
- Output `DeploymentReadiness` with `host_record_present`, `enrolled`,
  `trusted`, `endpoint`, `endpoint_class`, `transport_accepted`,
  `capabilities_valid`, `declares_inference`, `secret_provisioned`, `ready`,
  `blockers`, `warnings`, plus `blocker_codes()` / `is_ready()`.
- `evaluate_deployment_readiness(&input)` checks, in deterministic order:
  1. callsign present;
  2. host record exists (`host_record_missing`);
  3. host enrolled (`host_not_enrolled`);
  4. host trusted (`host_not_trusted`);
  5. endpoint accepted by `TransportPolicy` (`transport_rejected`, with an
     actionable message for public plain-http / malformed / unsupported scheme);
  6. shared secret provisioned (`secret_not_provisioned`).
- `declares_inference` comes from `HostCapabilities::from_value` when the stored
  capabilities are structured; raw/legacy values fall back to the host role and
  raise a non-blocking `capabilities_not_declared` warning. Role/capability
  mismatch emits a warning, not a blocker.
- `ready = blockers.is_empty()`; warnings never block.

**Safe configuration template (pure):**

- `AgentConfigTemplate { host_uuid, callsign, bind_address, secret_env_var,
  endpoint, capabilities: &HostCapabilities, tls_proxy_note }`.
- `render_agent_config(&template, TemplateFormat::{Json,Toml})` renders
  deterministic text containing:
  `schema`, `host_uuid`, `callsign`, `bind_address`, `shared_secret_env`,
  `contains_secret_values = false`, `roles`, `capabilities`, `inference`,
  `endpoint`, `tls_proxy_note`.
- There is deliberately **no secret-value field**; only the environment variable
  name (`DEFAULT_SECRET_ENV_VAR = AEGIS_REMOTE_SECRET`). Tests assert no secret
  value and no `shared_secret_value` key appear.
- Validates host UUID, callsign, bind address, env-var name, endpoint syntax, and
  TOML escaping; defaults `DEFAULT_AGENT_BIND = 127.0.0.1:8787` and
  `DEFAULT_TLS_PROXY_NOTE`.

### 3.3 `src/lib.rs` (modified) â€” module wiring only

- Added `pub mod deployment;` and `pub mod transport;` plus re-exports. No
  existing function or signature changed; `add_host` endpoint validation is
  untouched for backward compatibility. Callers opt into the new gate via
  `validate_endpoint_security` / `evaluate_deployment_readiness`.

### 3.4 `tests/fleet.rs` (modified) â€” one integration test

- `deployment_readiness_reflects_stored_capabilities_and_transport`: stores a
  General/Artemis host and an Inference/Venus host with validated
  `HostCapabilities`, enrolls both, and confirms readiness reflects
  capabilities (`declares_inference`) and endpoint class (PrivateLan vs
  Loopback). No network or remote action.

Capability reporting and stale handling were re-verified by the existing
integration tests (`host_capabilities_validate_and_round_trip`,
`stale_hosts_are_excluded_from_placement_and_summary`,
`unenrolled_host_is_never_eligible`); no gaps were found, so they were not
rewritten.

---

## 4. Exact files changed by this agent

| File | Change |
|---|---|
| `crates/aegis-fleet/src/transport.rs` | **new** (10 unit tests) |
| `crates/aegis-fleet/src/deployment.rs` | **new** (13 unit tests) |
| `crates/aegis-fleet/src/lib.rs` | module wiring + re-exports only |
| `crates/aegis-fleet/tests/fleet.rs` | 1 new integration test + imports |
| `docs/implementation/BATCH6_AGENT01_FLEET.md` | this document (new) |

Not touched: `crates/aegis-fleet/Cargo.toml`, `Cargo.lock`, workspace
`Cargo.toml`, `crates/aegis-cli`, `.gitignore`, `docs/completion`, any other
crate, and `D:\SofiaDev\SofiaAdaLyra`.

---

## 5. Tests and results

Commands run from `D:\Aegis`:

```powershell
cargo test -p aegis-fleet
cargo clippy -p aegis-fleet --all-targets -- -D warnings
```

Result (this run):

| Target | Result |
|---|---|
| `src/lib.rs` unit tests (all modules) | **64 passed**, 0 failed |
| `src/bin/aegis-fleet-identity.rs` | 0 tests |
| `tests/fleet.rs` integration | **13 passed**, 0 failed |
| `tests/performance.rs` integration | **3 passed**, 0 failed |
| Doc-tests | 0 |
| **Total** | **80 passed, 0 failed** |
| `cargo clippy -p aegis-fleet --all-targets -- -D warnings` | **clean (exit 0)** |

New tests this batch: **24** (10 transport unit + 13 deployment unit + 1
integration). The live GPU test remains env-gated (`AEGIS_GPU_LIVE=1`) and
passes as a skip without hardware; this is pre-existing behavior.

---

## 6. Transport / TLS policy summary

- Loopback and private-LAN plain http: allowed by default (homelab).
- Public plain http: rejected by default.
- Public endpoint: must be TLS-fronted by a proxy; the proxy can be addressed
  over http on loopback/private-LAN because the bundled Ollama adapter only
  speaks http and cannot terminate TLS.
- `https://` schemas are accepted by the policy but flagged
  `adapter_supports_transport = false`; do not point the current adapter at them
  directly.
- Classification is syntactic and offline; it is a guardrail, **not** a firewall,
  and performs no DNS resolution.

---

## 7. Owner approval checklist (exact; required before any remote step)

1. Named owner approval recorded with date/time, scope, and lab-only intent.
2. Approved maintenance window and a named rollback owner.
3. Explicit decision: Artemis is **Inference** (requires Ollama, models, and TCP
   11434) or **General/Storage** (must not advertise inference). Default is
   General/Storage while no inference runtime is installed.
4. Physical or authorized-console access, or explicit authorization to use
   owner-provided WinRM/RDP credentials. No credentials are to be guessed,
   extracted, or reused by the agent.
5. Approved host configuration: callsign, `host_uuid`, bind address, endpoint,
   and whether a TLS terminator is required.
6. Out-of-band shared-secret provisioning for `AEGIS_REMOTE_SECRET` (at least 16
   bytes) on both hosts; never in argv, logs, templates, or the repository.
7. Approval of the exact install artifact/source and the ports to be opened
   (none beyond what the chosen role requires).
8. Agreement that the run is lab-only, not production activation, and that
   evidence will be captured.

This checklist is **not** approval; each item must be explicitly satisfied by
the owner before execution.

---

## 8. Install / rollback runbook (BLOCKED â€” for a future authorized run only)

> **Not executed. Every step below is blocked pending explicit owner approval
> and the checklist in section 7.**

### Install (authorized run only)

1. Record the approval, scope, window, rollback owner, and lab-only intent.
2. Back up Venus Fleet state: Platform SQLite DB and `owner.proof` (do not
   commit).
3. Choose Artemis role (default General/Storage; Inference only if authorized
   with Ollama installed and TCP 11434 intentionally opened).
4. Generate the agent configuration with `render_agent_config` (JSON or TOML).
   Confirm it shows `contains_secret_values = false` and only the env-var name.
5. On Artemis, via the approved console path, install only the authorized
   runtime/agent. Do not open ports beyond the role's requirement.
6. Provision `AEGIS_REMOTE_SECRET` out of band on both hosts; verify length
   â‰¥ 16 bytes without echoing it.
7. On Venus, with the owner token, `FleetStore::add_host(...)` using validated
   `HostCapabilities::to_value()`, then run `evaluate_deployment_readiness` and
   require `ready == true`.
8. `FleetStore::enroll(host_id, owner_token)`; trust changes are immediate and
   audited in `fleet_events`.
9. `FleetStore::probe(host_id)` and `FleetStore::host_summary()`; require
   `stale = 0`, `unreachable = 0`, and intended eligibility.
10. Run `aegis-fleet-identity verify --connect <addr> --expect-host <uuid>` with
    the secret set on both sides; require `verified = true`.
11. Verify placement: `placements()` includes Artemis only if enrolled, trusted,
    reachable, and fresh. Inference routing only if inference was authorized.
12. Capture evidence: readiness JSON, template (no secret), probe result,
    identity output, placement output.

### Rollback (on any failure)

1. `FleetStore::revoke(host_id, owner_token)`, then
   `FleetStore::remove(host_id, owner_token)`.
2. Stop/remove the Artemis agent and any installed runtime.
3. Close any ports added for the run.
4. Restore the Venus DB backup and re-probe remaining hosts.
5. Record the rollback outcome and retain evidence (no secrets).

---

## 9. Blocked items and explicit statements

- **No two-host Venusâ†”Artemis verification was performed.** No Artemis install,
  enrollment, probe, identity handshake, firewall change, service change, or
  credential use occurred in this batch.
- **All remote/Artemis steps remain blocked pending explicit owner approval**
  and satisfaction of section 7.
- Artemis has no confirmed Ollama/11434 inference runtime in the observed
  environment; it must not be declared as inference until an authorized install
  and declaration.
- The built-in Ollama adapter only speaks plain http and cannot terminate TLS;
  public inference endpoints therefore require an external TLS terminator.
- Transport classification is syntactic; it does not resolve DNS and is not a
  network firewall.
- Deployment readiness is advisory pre-flight; it does not prove a remote host
  is reachable or trustworthy.
- `add_host` still accepts free-form raw `Value` capabilities for compatibility;
  structured `HostCapabilities` is the supported path and the readiness report
  treats unstructured capabilities as a warning.

---

## 10. Next steps for the owner

1. Review this document and decide whether to authorize an Artemis run.
2. If authorizing, complete the section 7 checklist and schedule the runbook.
3. Have the CLI owner optionally surface `DeploymentReadiness`,
   `validate_endpoint_security`, and `render_agent_config` through
   `aegis-cli` diagnostics (CLI is outside this agent's ownership).
4. After an authorized run, perform the real two-host verification and record
   evidence. Until then, treat this document as **not** having done so.

