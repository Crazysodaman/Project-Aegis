# Aegis FINAL — Agent 16: Security, Ownership & Data Integrity Report

**Date:** 2026-11-26
**Repository:** `D:\Aegis`
**Baseline HEAD:** `2bd2513` (no commit, no reset, no clean, no deploy)
**Scope:** adversarial security audit of owner proof/token handling, secret
leakage in errors, database marker/ownership isolation, STOP enforcement, backup
ownership/contamination, and enrollment trust.
**Constraints honored:** `D:\SofiaDev\SofiaAdaLyra` never touched; production
source not edited; no new dependencies; only new test files plus this report.
All tests use `tempfile` roots; no live `var/` data was read or written.

---

## 1. Files added (test-only)

| File | Purpose |
|---|---|
| `crates/aegis-safe/tests/stop_and_owner_adversarial.rs` | Hostile token shapes, owner-proof removal, audit forgery, foreign markers, STOP destruction limit |
| `crates/aegis-core-install/tests/enrollment_authorization.rs` | Owner-gated lifecycle mutations, prohibited-Core enrollment traces, missing-proof enrollment |
| `crates/aegis-roster/tests/enrollment_trust.rs` | Hostile-name non-reflection and install-authorization invariants |
| `crates/aegis-backup/tests/ownership_contamination.rs` | Forged-manifest cross-Core sidecar swap, Platform/Safe marker swap, owner-proof leakage |

No existing test was duplicated: each file attacks a case not exercised by
`security_acceptance.rs`, `safe_boundary.rs`, `core_installation.rs`,
`core_regression.rs`, `lifecycle_integrity.rs`, `roster_policy.rs`,
`acceptance_roster.rs`, `backup_restore.rs`, `batch8_inventory.rs`, or
`batch8_qa.rs`.

---

## 2. Verification (Windows 1.88.0 toolchain)

Commands:

```
cargo test -p aegis-safe -p aegis-roster -p aegis-core-install -p aegis-backup
cargo clippy -p aegis-safe -p aegis-roster -p aegis-core-install -p aegis-backup --all-targets -- -D warnings
```

Results (final clean run):

| Package | Tests | Result |
|---|---|---|
| `aegis-safe` | 2 unit + 6 `safe_boundary` + 6 `security_acceptance` + **7 new** = **21** | 21 passed, 0 failed |
| `aegis-roster` | 7 unit + 4 `acceptance_roster` + 4 `roster_policy` + **2 new** = **17** | 17 passed, 0 failed |
| `aegis-core-install` | 12 `core_installation` + 5 `core_regression` + 5 `lifecycle_integrity` + **3 new** = **25** | 25 passed, 0 failed |
| `aegis-backup` | 12 `backup_restore` + 6 `batch8_inventory` + 2 `batch8_qa` + **3 new** = **23** | 23 passed, 0 failed |
| **Total edited packages** | **86** | **86 passed, 0 failed, 0 ignored** |

**Clippy:** `cargo clippy ... --all-targets -- -D warnings` → exit 0, no
warnings.

Note: one transient compile failure occurred while another agent was mid-write
in `crates/aegis-backup/src/backup.rs` (a referenced function did not exist yet).
After that file settled, the full run above completed clean; no production file
was modified by this agent.

---

## 3. Security posture (as actually implemented)

| Domain | Enforced today | Evidence |
|---|---|---|
| Owner token verification | Constant-time compare; empty/prefix/extension/wrong tokens all reject; STOP mutations return `Unauthorized` | new `empty_prefix_and_extension_tokens_never_authorize`, `wrong_token_cannot_clear_an_active_stop` |
| Secret leakage | `Unauthorized`, missing-proof, and corrupt-proof errors contain no token, proof bytes, `owner.proof`, `safe.db`, or root path; owner token never appears in `safe.db` | existing acceptance + new owner/lifecycle/backup tests |
| Owner-proof removal | Fails closed with `OwnerNotInitialized`; an existing STOP cannot be set or cleared; deleted proof cannot be rotated because the owner row already exists | new `removing_owner_proof_fails_closed_and_keeps_stop_active`, `deleted_owner_proof_cannot_be_rotated_by_reinitialization` |
| Audit integrity | Append-only SHA-256 chain detects rewritten rows; rejected token attempts append nothing | new `audit_chain_detects_forged_rows` |
| DB marker isolation | Safe rejects Platform/Core/Memory `application_id` markers; Core rejects Platform/Safe/Memory markers and foreign Core UUIDs | new `safe_database_rejects_core_and_memory_markers`; existing `security_markers.rs` |
| STOP enforcement | Active STOP blocks runtime start with no heartbeat/side effect; survives reopen; monotonic revision; owner-only clear | existing `stop_security.rs`; new owner tests |
| Enrollment/lifecycle | Install, rename, suspend, resume, uninstall-preserve all require owner proof; rejected calls leave lifecycle, DB bytes, manifest, and history untouched; prohibited Core numbers #4/#5 leave no DB/marker/registry row | new `lifecycle_mutations_require_owner_and_leave_state_untouched`, `prohibited_cores_cannot_enter_enrollment_and_leave_no_trace`, `missing_owner_proof_blocks_enrollment_without_leaking_paths` |
| Roster identity trust | Approved names are immutable at install; hostile supplied names are never reflected; only 1/2/3/6/7 resolve for install | new `hostile_requested_names_are_not_reflected_in_errors`, `only_approved_core_numbers_can_be_resolved_for_install` |
| Backup ownership | Cross-Core sidecar swap is rejected **even after the attacker rewrites the manifest digests** | new `cross_core_sidecar_swap_with_forged_manifest_is_caught_by_ownership` |
| Backup proof secrecy | `owner.proof` bytes copied verbatim, never parsed/embedded; tamper details and restore errors do not echo proof bytes | existing `owner_proof_is_copied_byte_for_byte_and_not_embedded`; new `tampered_owner_proof_is_reported_without_echoing_its_bytes` |

---

## 4. Defects / limitations found

### D-16.1 (Medium) `verify_backup` does not validate Platform/Safe markers
`verify_backup` hashes `platform.db` and `safe.db` but never calls
`check_markers`; marker validation happens only in `restore`'s
`verify_restored_root` (`crates/aegis-backup/src/verify.rs:70`,
`crates/aegis-backup/src/restore.rs:356`, `crates/aegis-backup/src/state.rs:139`).
An attacker who swaps the two files inside a finished backup and rewrites the
manifest digests gets `BackupVerificationStatus::Verified`.

Evidence: `swapped_platform_and_safe_databases_escape_verify_but_restore_fails_closed`.
Impact: `verify_backup` alone is not a sufficient integrity gate for Platform/Safe
identity. A fresh-target `restore` catches it (staging removed, no target
created). For an opted-in `existing_restore`, files are copied into the target
before `verify_restored_root` fails, so the rollback copy (captured first) is the
recovery path rather than an untouched target.
Recommendation: make `verify_backup` call `check_markers` (and Core marker
validation) so verification and restore agree.

### D-16.2 (Medium) STOP and audit have no external anchor
STOP state and the audit chain live only inside `safe.db`
(`crates/aegis-safe/src/store.rs:123`). Deleting `safe.db` then triggering any
`initialize()` path (`SafeStore::initialize`, used by
`PlatformService::initialize/start`) recreates a default inactive STOP and an
empty audit history (`crates/aegis-safe/src/store.rs:48-66`). A read without
`initialize()` fails closed, and runtime start fails closed immediately after
deletion, but the two-step reset is not tamper-evident.

Evidence: `destroying_safe_database_erases_stop_and_audit_documented_limitation`.
Impact: STOP is an operator safety control against misbehaving Cores, not a
tamper-proof interlock against a principal with write access to the storage
root. Same-user malware can already read the DPAPI proof, so this does not widen
the same-user threat, but it matters for forensic/hostile-disk assumptions.
Recommendation: add an independent anchor (e.g., a monotonic STOP revision pinned
outside `safe.db`, or refuse to recreate missing Safe state without an explicit
owner-authorized recovery).

### D-16.3 (Documented) Integrity is not authenticity
Backups and Core manifests are SHA-256 content integrity only; there is no
signature/MAC. A party that can rewrite both a database and its manifest can
produce a self-consistent pair. The cross-Core ownership check is genuine
defense-in-depth (proved by D-16 evidence), but a same-Core content rewrite with
a re-sealed manifest is undetectable by this mechanism. `crates/aegis-core-integrity/src/lib.rs`
already states this; the report challenge below is about "no defects/no risk"
wording, not the underlying design.

### D-16.4 (Documented) Windows-only DPAPI owner proof
`owner.proof` uses DPAPI current-user protection on Windows
(`crates/aegis-safe/src/owner_proof.rs:112-226`). This is not production-grade
owner authentication: same-user malware can decrypt it, there is no user
presence or attempt rate limit, and a backup restored under a different Windows
user/machine cannot decrypt it. On non-Windows the `unix-file-dev` fallback
stores the token in a plaintext `0600` file
(`crates/aegis-safe/src/owner_proof.rs:122-131`).

### D-16.5 (Documented) Linux owner proof is untested here
The Unix `cfg` paths compile in principle but were not built or run on Linux in
this environment. `OWNER_PROOF_KIND` becomes `unix-file-dev`, which is a
development fallback, not a Linux production owner-proof backend.

### D-16.6 (Documented) Remote identity is HMAC over plain HTTP
Fleet host identity is HMAC-SHA256 nonce challenge/response, which authenticates
hosts but does not encrypt the channel. The current transport policy rejects
public plain-HTTP endpoints by default and allows loopback/private-LAN or
`https://` (`crates/aegis-fleet/src/transport.rs:3-15`), but private-LAN plain
HTTP remains unencrypted and the bundled Ollama adapter cannot terminate TLS. A
public endpoint must be fronted by a TLS terminator.

### D-16.7 (Documented) Modules are not sandboxed
Modules get cleared-environment spawning, timeouts, output caps, watchdogs, and
forced termination, but **no OS sandbox, filesystem jail, or network isolation**;
children run with the Platform user's full rights
(`crates/aegis-modules/src/sandbox.rs:1-10`,
`crates/aegis-modules/src/policy.rs:4-20`,
`crates/aegis-modules/src/runtime.rs:22-23,1245-1246`). Environment isolation is
not a security sandbox. No memory-limit or CPU-limit enforcement exists.

---

## 5. Challenges to Batch 8 claims

1. **"M7 internal adversarial QA: 16 new tests; no defects found" /
   "Defects found: None" is overstated.** The internal suite validated the happy
   ownership paths but did not test (a) Platform/Safe marker swaps inside a
   finished backup, (b) forged manifest digests around a cross-Core sidecar swap,
   (c) re-initialization after `safe.db` destruction, or (d) owner-gated lifecycle
   rejection paths. This audit found verifiable gaps (D-16.1, D-16.2) and added
   tests for the cases that did hold. "No defects found" should read "no defects
   found within the tested matrix".
2. **"Backup ... ownership/digest validation" implies complete verification.** It
   is complete for Core databases and sidecars but not for the Platform/Safe
   marker binding (D-16.1). Restore does catch it; standalone `verify_backup`
   does not.
3. **"Cross-Core contamination rejected" is accurate but narrow.** It holds for
   naive and forged-digest sidecar swaps thanks to ownership tables (new evidence),
   but it is not a claim of backup authenticity. Re-sealing a same-Core manifest
   defeats digest-only detection, as the docs elsewhere correctly admit.
4. **Live "Venus" evidence and "798 passed / clippy clean" are self-reported and
   were not independently re-run in this audit.** They are plausible but cannot
   be treated as independent acceptance. Batch 8 itself correctly labels external
   QA "NOT AVAILABLE"; that remains true.
5. **"Nothing in the target is deleted" / atomic restore.** True for fresh-target
   staging. For `allow_existing`, files are replaced in place before final
   verification, so a late failure leaves a partly-updated target; the pre-capture
   rollback is the safety net, not target immutability. This nuance is not
   overstated in the M1 doc but is easy to misread from "atomic operations".
6. **Transport and module claims are consistent with the code** (plain HTTP with
   TLS-fronting requirement; no OS sandbox). They are limitations, not defects,
   and should remain explicit in any production-readiness statement.

---

## 6. Residual limitations to carry forward

- HMAC host identity over plain HTTP; no transport encryption unless a TLS proxy
  fronts it.
- DPAPI owner proof is Windows/current-user only; cross-user/cross-machine restore
  will not decrypt.
- Linux owner-proof path (`unix-file-dev`) is a plaintext 0600 development
  fallback and is untested on Linux.
- Modules are environment-isolated but unsandboxed (no FS jail, network isolation,
  memory/CPU caps).
- Content integrity is not authenticity; no signatures/MACs on manifests or
  backups.
- STOP/audit are not anchored outside `safe.db`; destruction plus
  re-initialization resets them.
- `verify_backup` does not check Platform/Safe application markers; use restore
  (or add the marker check) before trusting a backup root.
- Independent external QA and owner acceptance remain unavailable.

---

## 7. Verdict

Development security posture is **materially improved and verified** for token
rejection, error non-disclosure, Core/sidecar ownership, owner-gated lifecycle,
and cross-Core backup contamination. It is **not** a production security
acceptance: the marker-verification gap, unanchored STOP/audit state, plaintext
Linux owner proof, plain-HTTP HMAC transport, and unsandboxed modules remain.
**Production: NO-GO** until D-16.1 and D-16.2 are addressed, Linux owner proof is
built/tested, and independent review is performed.
