# Batch 7 — Agent 10: Security, Core Regression & QA

**Date:** 2026-11-26
**Repository:** `D:\Aegis`
**HEAD:** `2bd2513` (unchanged; no commit performed)
**Scope:** Additive tests under the permitted packages plus two new documents.
No production source was edited, no dependency was added, no live `var/`
database was opened, and `D:\SofiaDev\SofiaAdaLyra` was never touched.

---

## 1. Status

**PASS (lab-development evidence only).** 14 new additive tests pass; the owned
package gates (`fmt` check, `clippy --all-targets -- -D warnings`, `test`) are
green. No production defect was found.

**Production: NO-GO. Independent acceptance: NOT AVAILABLE.** All evidence here
was produced by the implementation author (this agent). It is **not** independent
QA and must not be described as such.

---

## 2. Files created / edited

Only test files and the two Batch 7 documents were touched.

| File | Change | Tests (before → after) |
|---|---|---|
| `crates/aegis-safe/tests/security_acceptance.rs` | +2 tests | 4 → 6 |
| `crates/aegis-core-storage/tests/security_markers.rs` | +2 tests | 2 → 4 |
| `crates/aegis-core-runtime/tests/stop_security.rs` | +3 tests | 2 → 5 |
| `crates/aegis-core-install/tests/core_regression.rs` | +2 tests | 3 → 5 |
| `crates/aegis-roster/tests/acceptance_roster.rs` | +1 test | 3 → 4 |
| `crates/aegis-core-profiles/tests/core_regression.rs` | +4 tests | 2 → 6 |
| `docs/implementation/BATCH7_AGENT10_SECURITY_CORE.md` | new | — |
| `docs/implementation/BATCH7_ACCEPTANCE_EVIDENCE.md` | new | — |

No other file was created, edited, or deleted by this agent. Concurrent work by
other agents in `aegis-fleet`, `aegis-ui`, `aegis-service`, `aegis-embodiment`,
`aegis-modules`, and `aegis-social` was not modified.

---

## 3. New security tests

### 3.1 `aegis-safe` — owner proof token/path non-disclosure

1. `owner_token_is_not_persisted_or_audited_in_plaintext` — after initializing
   the owner and exercising STOP set/clear, the raw `safe.db` bytes contain
   neither the 64-hex owner token nor the absolute storage root; the audit chain
   is still valid. Complements the Batch 6 error-message opacity tests by
   checking storage rather than only `Display`.
2. `uninitialized_owner_error_does_not_leak_paths` — `verify_owner` and
   `owner_token` on a store whose `owner.proof` was never created return
   `SafeError::OwnerNotInitialized` without `owner.proof`, `safe.db`, or the
   storage root in the message.

Note: `safe.db`'s `safe_owner.proof_file` column intentionally records the
static string `owner.proof`. That is a fixed filename, not a filesystem path,
and the new test therefore checks the absolute root rather than the filename.

### 3.2 `aegis-core-storage` — corrupt/foreign markers and ownership binding

1. `corrupt_core_database_bytes_are_rejected_without_path_leak` — overwriting a
   valid `core.db` with non-SQLite bytes makes `CoreDatabase::open` fail, and the
   error contains neither `core.db` nor the storage root.
2. `core_database_is_bound_to_its_owning_core_uuid` — copying Core A's `core.db`
   into Core B's directory makes `CoreDatabase::open` (and `read_snapshot`)
   return `CoreStorageError::ForeignOwnership`, with no path leak. This is the
   explicit database-ownership binding edge case that Batch 6 did not isolate
   from the SQLite application-marker check.

### 3.3 `aegis-core-runtime` — STOP persistence and effect blocking

1. `stop_revision_is_monotonic_across_reopens` — STOP revision is 1 across two
   fresh `SafeStore` instances, then 2 after clear; the audit chain stays valid.
2. `stop_blocks_runtime_start_without_reactivating_suspended_core` — a suspended
   Core is not reactivated by a STOP-blocked `start`; no heartbeat is written;
   after clearing STOP the Core starts and becomes `active`.
3. `unauthorized_runtime_error_does_not_leak_token_or_paths` — `start` with a
   wrong token returns `RuntimeError::Unauthorized` whose `Display` contains
   neither the candidate token, the real token, `owner.proof`, nor the root, and
   no heartbeat is created.

---

## 4. New core regression tests

### 4.1 `aegis-core-install`

1. `three_installed_cores_keep_distinct_stable_instances_across_reopen` —
   installs #1 Sofía, #2 Mack, #6 Vera; asserts three distinct `core.db` paths,
   three pairwise-distinct `database_instance_uuid` values, none equal to any
   Core UUID, and that all instance UUIDs and integrity reports are stable after
   constructing a fresh `CoreInstaller` on the same root.
2. `suspend_and_reactivate_preserve_identity_and_record_history` — deactivate and
   reactivate a Core; asserts identity, `database_instance_uuid`, and the exact
   database bytes are unchanged, integrity still verifies, and the append-only
   lifecycle history records `installed→suspended` then `suspended→active`.

### 4.2 `aegis-roster`

1. `approved_cores_reject_impostor_names_and_keep_catalog_designations` — for
   #1/#2/#3/#6 the approved name resolves, any other name yields
   `RosterError::DisplayNameMismatch`, and designations are exact (including
   #6 = `Vera / S-039`). Complements the existing default/prohibited/blank
   coverage without duplicating it.

### 4.3 `aegis-core-profiles`

1. `foreign_memory_database_error_does_not_leak_paths` — copying A's `memory.db`
   over B's still yields `SdkError::OwnershipMismatch`, now also asserted to
   contain neither `memory.db` nor the root.
2. `cross_core_memory_import_is_rejected_without_leaking_data` — importing A's
   export into B fails with `SdkError::ForeignCore`; B's `get`/`export` stay
   empty and A's entry is intact.
3. `one_core_cannot_mutate_another_cores_memory_entries` — B's
   `update_content`/`set_tags`/`archive`/`soft_delete`/`purge` against A's id
   all return `SdkError::NotFound`; A's entry is unchanged (revision 1).
4. `suspend_and_reactivate_preserve_core_memory_and_identity` — memory written
   before suspension is readable during suspension and after reactivation;
   identity/instance UUID/integrity are preserved; B remains empty.

---

## 5. Read-only security interface review

- **Fleet remote identity** (`crates/aegis-fleet/src/remote.rs`): HMAC-SHA256
  over a domain-separated, nonce/host/callsign-bound message; constant-time
  compare; minimum 16-byte secret; nonce length bound. Symmetric secret over
  plain HTTP — not transport encryption and not a substitute for TLS.
- **Fleet transport** (`crates/aegis-fleet/src/transport.rs`): public plain-HTTP
  endpoints are rejected unless explicitly opted in; loopback/private-LAN
  allowed. Classification is syntactic and offline (no DNS resolution), so it is
  a guardrail, not a firewall.
- **Safe owner proof** (`crates/aegis-safe/src/owner_proof.rs`): Windows uses
  DPAPI (`CryptProtectData`) and constant-time comparison; the non-Windows
  fallback is a plaintext 0600 file and is untested here.
- **Storage markers** (`crates/aegis-core-storage/src/lib.rs`,
  `crates/aegis-storage/src/lib.rs`): per-database SQLite `application_id`
  markers (Platform/Safe/Core/Memory) plus per-row Core ownership; both are
  enforced on open, and errors do not echo paths.
- **Module permissions** (`crates/aegis-modules/src/policy.rs`): trust is
  assigned externally (a manifest cannot self-promote); restricted modules are
  denied unless `allow_restricted` is set; limits are clamped to framework hard
  caps. **There is no OS sandbox.** This agent owns no `aegis-modules` tests
  (outside its edit scope), so module policy was only reviewed, not extended.

---

## 6. Verification (this run)

Package-scoped only; no whole-workspace gate was run.

```
cargo test -p aegis-safe -p aegis-core-storage -p aegis-core-runtime \
  -p aegis-core-install -p aegis-roster -p aegis-core-profiles \
  -p aegis-core-identity -p aegis-core-integrity -p aegis-core-registry
```

**Actual: 157 passed, 0 failed, 0 ignored** (Batch 6 baseline: 143; **+14**).

Per package: safe 14, core-storage 17, core-runtime 14, core-install 22,
roster 15, core-profiles 18, core-identity 22, core-integrity 16,
core-registry 19.

```
cargo clippy -p aegis-safe -p aegis-core-storage -p aegis-core-runtime \
  -p aegis-core-install -p aegis-roster -p aegis-core-profiles \
  -p aegis-core-identity -p aegis-core-integrity -p aegis-core-registry \
  --all-targets -- -D warnings
```

**Actual: exit 0, no warnings.**

```
cargo fmt -p aegis-safe -p aegis-core-storage -p aegis-core-runtime \
  -p aegis-core-install -p aegis-roster -p aegis-core-profiles \
  -p aegis-core-identity -p aegis-core-integrity -p aegis-core-registry -- --check
```

**Actual: exit 0, no diff.**

Targeted new-test binaries (all pass): `security_acceptance` 6,
`security_markers` 4, `stop_security` 5, install `core_regression` 5,
`acceptance_roster` 4, profiles `core_regression` 6.

---

## 7. Defects found

**No production defect.** All 14 new tests passed on first execution after one
test-authoring correction: the first draft asserted that the string
`owner.proof` never appears in `safe.db`, but `safe_owner.proof_file` stores that
static filename by design. The assertion was corrected to check for the absolute
storage root instead. Test-only issue; no product code changed.

---

## 8. Review of Batch 6 claims (corrections / clarifications)

I did **not** silently approve Batch 6. Corrections and boundaries:

1. **"Cargo.lock unchanged / no new third-party dependencies."** Partly
   imprecise. `Cargo.lock` did change (+1 line) and
   `crates/aegis-fleet/Cargo.toml` added `sha2.workspace = true`. `sha2` is an
   existing workspace dependency, so **no new third-party crate** was added, but
   the lockfile and that crate manifest are not byte-identical to HEAD.
2. **"614 tests, 0 failed, 0 ignored" / "cargo clippy --workspace exit 0".**
   Self-reported and **not independently verified here**; this agent is
   restricted to package-scoped gates and did not run the whole-workspace suite.
3. **"Secret scan: no hits; `var/` not tracked."** Re-verified: the ripgrep secret
   pattern returns no matches (`rg` exit 1 = pass), `git ls-files var` is empty,
   and no tracked `*.db`/`owner.proof` exists.
4. **"16 new security/regression tests / 143 across nine packages."** Confirmed:
   the Batch 6 baseline sum is 143. This batch adds 14 more (157).
5. **"No independent QA" statements are accurate.** Batch 6 does not claim
   independent acceptance; this document likewise does not.
6. **Live TTS/avatar/BMP, Artemis, Debian 13, gameplay/FPS, native GUI,
   cross-host artifacts.** These remain owner/hardware-dependent and
   self-reported; this agent could not and did not verify them.

---

## 9. Security limitations (unchanged)

- HMAC is symmetric over plain `http://`; no transport encryption. TLS must be
  terminated by a proxy. The transport check is a syntactic guardrail.
- Owner proof is DPAPI-protected only on Windows; the non-Windows fallback is a
  plaintext 0600 file and is **untested on Linux** in this environment.
- Core integrity is content SHA-256, not authenticity/signing.
- Modules are not OS-sandboxed; trust is policy, not a security boundary.
- `safe.db` stores the static proof filename, and the `run/` heartbeat and stop
  sentinels are plaintext files (no secrets in them).
- **Independent QA / owner acceptance: NOT AVAILABLE.**

---

## 10. GO / NO-GO

- **Lab development: GO** for the owned packages (green, additive, recoverable).
- **Production: NO-GO** (plain-HTTP HMAC, Windows-only-tested proof, no module
  sandbox, content-only integrity, no independent acceptance).
- **Artemis / Linux / gameplay / native GUI: BLOCKED** on owner authorization,
  hardware, or approved dependencies.
