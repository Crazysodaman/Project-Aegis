# Batch 6 — Agent 10: Security Tests, Core Regression & Acceptance Evidence

**Date:** 2026-11-26
**Repository:** `D:\Aegis`
**Baseline HEAD:** `2bd2513` (unchanged; no commit performed)
**Scope:** additive tests under owned packages plus this documentation. No
production source was modified, no dependency was added, and no live `var/`
database was touched.

---

## 1. Status

**PASS (lab-development evidence).** 16 new additive tests pass; the owned package
gates pass (`fmt`, `clippy -D warnings`, `build`, `test`). No product defect was
found, so no crate source was changed. Production readiness remains **NO-GO**
and independent acceptance remains **NOT AVAILABLE** (see §7).

---

## 2. Exact files created

Test files (all new):

| File | Tests | Purpose |
|---|---|---|
| `crates/aegis-safe/tests/security_acceptance.rs` | 4 | Owner-proof failure opacity, token/path non-disclosure, foreign Safe marker rejection |
| `crates/aegis-core-storage/tests/security_markers.rs` | 2 | Core DB rejects foreign application markers without leaking paths |
| `crates/aegis-core-runtime/tests/stop_security.rs` | 2 | Active/persisted STOP blocks runtime start with no side effects |
| `crates/aegis-core-install/tests/core_regression.rs` | 3 | Two Cores: separate DBs and distinct instance UUIDs; #4/#5 prohibited; #7 needs owner name |
| `crates/aegis-roster/tests/acceptance_roster.rs` | 3 | Sofía/Mack/Nova/Vera identities & statuses, #4/#5 prohibited, #7 blank |
| `crates/aegis-core-profiles/tests/core_regression.rs` | 2 | Profile identities/statuses and cross-Core memory isolation |

Documentation (new):

- `docs/implementation/BATCH6_AGENT10_SECURITY_ACCEPTANCE.md` (this file)
- `docs/implementation/BATCH6_ACCEPTANCE_EVIDENCE.md`

No other file was created, edited, or deleted by this agent. Concurrent work by
other agents in `aegis-fleet`, `aegis-embodiment`, `aegis-service`, `aegis-ui`,
`aegis-cli`, etc. was not touched.

---

## 3. What the new security tests prove

### 3.1 Owner-proof failures do not leak secrets (`aegis-safe`)

`security_acceptance.rs`:

1. `owner_proof_failure_does_not_echo_candidate_or_real_token` — a wrong
   candidate (including a one-character mutation of the real token) fails
   `verify_owner`; `set_stop`/`clear_stop` return `SafeError::Unauthorized`
   whose `Display` contains neither the candidate nor the real token.
2. `owner_proof_and_database_paths_are_absent_from_errors` — a missing
   `owner.proof` yields `OwnerNotInitialized`; the message contains no
   `owner.proof`, no `safe.db`, and no storage-root path.
3. `corrupt_owner_proof_failure_is_opaque` — a corrupted proof file either fails
   verification (`Ok(false)`) or returns an error that contains none of the
   token, the corrupted bytes, the proof filename, or the root path. This test
   is platform-tolerant (Windows DPAPI error vs. Unix fallback mismatch).
4. `safe_database_with_foreign_marker_is_rejected` — a `safe.db` carrying the
   Platform application marker is rejected as
   `StorageError::WrongApplicationId { expected: SAFE, actual: PLATFORM }` with no
   path in the message.

### 3.2 Storage rejects wrong application markers (`aegis-core-storage`)

`security_markers.rs`:

1. `foreign_application_markers_are_rejected_without_path_leak` — after creating a
   valid Core DB, the marker is overwritten with the Platform marker, the Safe
   marker, and the memory marker (`AEGM`). Each open fails as
   `CoreStorageError::ForeignOwnership`; the error string contains neither
   `core.db` nor the storage root. Restoring the `AEGC` marker allows a clean
   reopen.
2. `core_marker_is_distinct_from_platform_and_safe` — the Core marker differs
   from Platform, Safe, and each other.

### 3.3 STOP blocks effects (`aegis-core-runtime`)

`stop_security.rs`:

1. `active_stop_blocks_runtime_start_and_leaves_no_effect` — with STOP active,
   `CoreRuntime::start` returns `RuntimeError::StopActive`, no heartbeat file is
   created, and the recorded state stays `Stopped`. After `clear_stop`, start
   succeeds.
2. `stop_survives_reopen_and_still_blocks_start` — STOP persists across a new
   `SafeStore` instance and still blocks start.

---

## 4. What the new regression tests prove

### 4.1 Two installed Cores are physically and logically separate

`crates/aegis-core-install/tests/core_regression.rs`:

- Installs #2 Mack Fuller and #6 Vera. Asserts distinct `core.db` paths, neither
  equal to `platform.db`/`safe.db`, both files present, both `installed`.
- Asserts distinct `database_instance_uuid` values and that neither instance UUID
  equals its Core UUID or the other Core's UUID.
- `inspect`/`verify` return each Core's own identity and a `verified` integrity
  report.

### 4.2 Roster/prohibited/blank policy

`crates/aegis-roster/tests/acceptance_roster.rs` and
`crates/aegis-core-install/tests/core_regression.rs`:

- #1 Sofía Ada Lyra (legacy preserved), #2 Mack Fuller, #3 Nova Arden, #6 Vera
  (designation `Vera / S-039`) are `implementable`, identity-defined, authorized,
  and installable.
- #4 and #5 are `prohibited`, identity not defined, not authorized, not
  installable; the installer rejects them with
  `CoreContractError::ProhibitedCoreNumber` and creates no database.
- #7 is `Unwritten`/blank: no default name, no invented identity; install with a
  blank name fails `CoreContractError::InvalidDisplayName`; install with an
  owner-supplied name succeeds.

### 4.3 Profile layer matches the roster

`crates/aegis-core-profiles/tests/core_regression.rs`:

- #1/#2/#3/#6 have the approved names/status/identity sources; #4/#5 are
  `DesignOnly`/`Prohibited` with no display name; #7 is `BlankFoundation`/
  `OwnerSupplied` with no invented identity fields.

### 4.4 Memory isolation

Same file: after installing two Cores, `CoreMemoryStore::open` for each yields
distinct `memory.db` paths. A memory created in Core A is not found by
`get`/`search`/`export` in Core B. Copying A's `memory.db` over B's path causes
`SdkError::OwnershipMismatch` on open. (Placed in `aegis-core-profiles` tests
because it is the owned, verified package that already depends on
`aegis-core-sdk`; `aegis-core-sdk` itself is outside this agent's edit scope.)

---

## 5. Verification results (this run)

```
cargo test -p aegis-safe -p aegis-core-identity -p aegis-core-storage \
  -p aegis-core-integrity -p aegis-core-registry -p aegis-core-install \
  -p aegis-core-runtime -p aegis-roster -p aegis-core-profiles
```

**143 passed, 0 failed, 0 ignored** across the nine packages (16 are new).

```
cargo clippy -p aegis-safe -p aegis-core-identity -p aegis-core-storage \
  -p aegis-core-integrity -p aegis-core-registry -p aegis-core-install \
  -p aegis-core-runtime -p aegis-roster -p aegis-core-profiles \
  --all-targets -- -D warnings
```

**exit 0**, no warnings.

```
cargo fmt -p aegis-safe -p aegis-core-identity -p aegis-core-storage \
  -p aegis-core-integrity -p aegis-core-registry -p aegis-core-install \
  -p aegis-core-runtime -p aegis-roster -p aegis-core-profiles -- --check
cargo build -p aegis-safe -p aegis-core-identity -p aegis-core-storage \
  -p aegis-core-integrity -p aegis-core-registry -p aegis-core-install \
  -p aegis-core-runtime -p aegis-roster -p aegis-core-profiles
```

Both **exit 0**.

Per-binary counts: identity 19+3, install 12+3+5, integrity 16, profiles 3+2+3+6,
registry 19, runtime 9+2, storage 13+2, roster 7+3+4, safe 2+6+4.

No whole-workspace gate was run by this agent.

---

## 6. Defects found

**None in production code.** All new tests passed against the baseline behavior
on first execution. One test-authoring error (an incorrect
`preserve_legacy_identity` expectation for Cores #2/#3/#6 in the new roster test)
was corrected; it was a test bug, not a product defect. No crate source change was
needed or made.

---

## 7. Honest security posture

| Area | Actual posture | Evidence / limitation |
|---|---|---|
| Fleet remote identity | Symmetric **HMAC-SHA256** over a shared secret; nonce/host/callsign binding; constant-time compare. | `aegis-fleet/src/remote.rs`. No transport encryption. |
| Transport | Plain `http://` only; `https://` is rejected by the built-in Ollama adapter. | `validate_endpoint`; secrets must be provisioned out of band. |
| Owner proof (Windows) | **Windows DPAPI** (`CryptProtectData`) bound to the current user; proof file is not the raw token. | `windows_owner_proof_is_dpapi_protected` test; this batch's opacity tests. |
| Owner proof (Linux) | Fallback is a `0600` plaintext file (`unix-file-dev`), **not** DPAPI. | **UNTESTED on Linux.** Only Windows was exercised in this environment. |
| STOP | Owner-gated; persists across reopen; blocks runtime start before spawning. | `stop_security.rs`; also used by Platform service, modules, tools. |
| Module execution | **Not sandboxed.** Manifests are declarative; permissions do not grant data access; there is no OS sandbox. | `aegis-modules` module docs. |
| Core integrity | Content integrity (SHA-256 manifest), **not authenticity**. A party who can rewrite DB+manifest can create a self-consistent pair. | Batch 5 audit; no signature/publisher chain. |
| Diagnostics | Read-only; never creates files; does not include secret material. | `aegis-diagnostics`; CLI run below. |
| Independent QA | **NOT AVAILABLE.** All authors are implementers; no separate independent reviewer exists. | This document is self-produced evidence. |

This batch does **not** claim independent QA. It does not claim a production-
ready security boundary. It adds regression evidence for the behaviors above.

---

## 8. Acceptance readiness

- **Lab development: GO.** Additive tests are green; owned gates pass.
- **Production: NO-GO.** Plain-HTTP HMAC, no module sandbox, content-only
  integrity, Linux proof untested, no independent QA.
- **Artemis / two-host Fleet: BLOCKED.** Requires owner authorization,
  credentials, an installed agent/Ollama, and out-of-band shared-secret
  provisioning. Not performed.
- **Hardware/owner items:** live GPU/gaming measurement, live TTS/STT/audio,
  native renderer, OS service registration, Linux runtime verification, and a
  confirmed destructive purge each require owner/hardware participation.

See `docs/implementation/BATCH6_ACCEPTANCE_EVIDENCE.md` for the reproducible
command package and expected outputs.
