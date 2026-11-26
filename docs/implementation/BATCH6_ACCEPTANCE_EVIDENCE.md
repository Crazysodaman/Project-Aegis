# Batch 6 — Reproducible Acceptance Evidence Package

**Date:** 2026-11-26
**Repository:** `D:\Aegis`
**Baseline HEAD:** `2bd2513`
**Owner:** Project Aegis (owner authorization required for anything beyond lab
verification)

This document is a reproducible package for an owner/operator to re-run. It uses
only disposable temporary roots. **Never** point these commands at the live
`D:\Aegis\var` data for acceptance testing, and never commit the temp root,
`owner.proof`, or any `*.db`.

Legend: **[A]** fully automated; **[O/H]** requires owner authorization and/or
hardware; **[BLOCKED]** not possible without the owner.

---

## 0. Environment

- Windows PowerShell 5.1 (this run), Rust toolchain compatible with the workspace.
- Run from `D:\Aegis`.
- Temporary root:

```powershell
$root = Join-Path $env:TEMP ("aegis-accept-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $root | Out-Null
$root
```

Expected: a new empty directory under `%TEMP%`. Keep `$root` for the session and
delete it in §7.

Build the CLI once:

```powershell
cargo build -p aegis-cli
$exe = "D:\Aegis\target\debug\aegis.exe"
```

Expected: exit 0; `$exe` exists.

---

## 1. Automated quality gates (owned packages) — [A]

These are the gates this agent is allowed to run. They are package-scoped, not
whole-workspace.

### 1.1 Format

```powershell
cargo fmt -p aegis-safe -p aegis-core-identity -p aegis-core-storage `
  -p aegis-core-integrity -p aegis-core-registry -p aegis-core-install `
  -p aegis-core-runtime -p aegis-roster -p aegis-core-profiles -- --check
```

Expected: no output, exit 0.

### 1.2 Clippy

```powershell
cargo clippy -p aegis-safe -p aegis-core-identity -p aegis-core-storage `
  -p aegis-core-integrity -p aegis-core-registry -p aegis-core-install `
  -p aegis-core-runtime -p aegis-roster -p aegis-core-profiles `
  --all-targets -- -D warnings
```

Expected: `Finished ...` and exit 0, no warnings.

### 1.3 Build

```powershell
cargo build -p aegis-safe -p aegis-core-identity -p aegis-core-storage `
  -p aegis-core-integrity -p aegis-core-registry -p aegis-core-install `
  -p aegis-core-runtime -p aegis-roster -p aegis-core-profiles
```

Expected: exit 0.

### 1.4 Tests

```powershell
cargo test -p aegis-safe -p aegis-core-identity -p aegis-core-storage `
  -p aegis-core-integrity -p aegis-core-registry -p aegis-core-install `
  -p aegis-core-runtime -p aegis-roster -p aegis-core-profiles
```

Expected (this run): **143 passed, 0 failed, 0 ignored** across the nine packages.
New Batch 6 tests: 16 (safe 4, core-storage 2, core-runtime 2, core-install 3,
roster 3, core-profiles 2).

New-test targeted re-run:

```powershell
cargo test -p aegis-safe --test security_acceptance
cargo test -p aegis-core-storage --test security_markers
cargo test -p aegis-core-runtime --test stop_security
cargo test -p aegis-core-install --test core_regression
cargo test -p aegis-roster --test acceptance_roster
cargo test -p aegis-core-profiles --test core_regression
```

Expected: each binary reports its new tests passing (4/2/2/3/3/2).

---

## 2. Core install / verify / inspect — [A] via CLI on a temp root

### 2.1 Initialize the Platform (writes an owner proof in the temp root)

```powershell
& $exe init --root $root
```

Expected JSON fields:

```json
{
  "schema": "aegis.cli.init.v1",
  "platform_id": "<uuid>",
  "callsign": "AEGIS-PLATFORM-XXXXXXXX",
  "owner_token_written": true
}
```

The owner token is **not** printed unless `--print-owner-token` is passed. Do not
pass that flag during acceptance evidence capture.

### 2.2 Install an approved Core (#2 Mack Fuller)

```powershell
& $exe core install a9a0a604-a4e7-4a88-a985-3f071e2c8e7d --root $root --roster 2
```

Expected JSON fields:

```json
{
  "schema": "aegis.cli.core_installation.v1",
  "identity": { "core_uuid": "a9a0a604-...", "core_number": 2,
                "display_name": "Mack Fuller", "identity_revision": 1 },
  "metadata": { "database_instance_uuid": "<uuid>",
                "initialization_mode": "fresh_no_migration", "schema_revision": 1 },
  "registry_entry": { "lifecycle_state": "installed",
                      "storage_relative_path": "cores/a9a0a604-.../core.db" },
  "integrity": { "status": "verified",
                 "actual_sha256": "..." }
}
```

### 2.3 Verify integrity

```powershell
& $exe core verify a9a0a604-a4e7-4a88-a985-3f071e2c8e7d --root $root
```

Expected: `"status": "verified"` and `actual_sha256 == expected_sha256`.

### 2.4 Inspect identity + storage + registry

```powershell
& $exe core inspect a9a0a604-a4e7-4a88-a985-3f071e2c8e7d --root $root
```

Expected: same `identity`, `metadata` (including `database_instance_uuid`), and a
registered `registry_entry`; `integrity.status == "verified"`.

### 2.5 Two-Core separation (automated equivalent)

The reproducible assertions live in
`crates/aegis-core-install/tests/core_regression.rs`
(`two_installed_cores_have_separate_databases_and_instance_uuids`). It installs
#2 and #6 and asserts distinct `core.db` paths and distinct
`database_instance_uuid` values. Run with the §1.4 command.

---

## 3. Diagnostics — [A]

```powershell
& $exe diagnostics --root $root
```

Expected fields after §2:

```json
{
  "schema": "aegis.diagnostics.report.v1",
  "status": "healthy",
  "reasons": [],
  "platform": { "available": true, "core_count": 1, "stop_active": false,
                "zero_core": false, "error": null },
  "safe": { "available": true, "owner_proof_present": true,
            "stop_active": false, "error": null },
  "cores": { "available": true, "count": 1, "error": null,
             "entries": [ { "integrity": { "status": "verified" },
                            "lifecycle_state": "installed" } ] },
  "modules": { "status": "available", "count": 0 }
}
```

Diagnostics is read-only. To prove it: capture the temp root tree before/after
(`Get-ChildItem -Recurse` or file hashes) and confirm no files were created or
modified by `aegis diagnostics`.

---

## 4. Fleet probe failure typing — [A] (no remote host)

Register a host whose endpoint is closed, then probe it. The probe **records**
failure as typed health data and returns success; it does not panic and does not
misreport the host as healthy.

```powershell
& $exe fleet host add probe-fail --endpoint http://127.0.0.1:9 --role inference --root $root
$host = (& $exe fleet host list --root $root | ConvertFrom-Json)[0]
& $exe fleet host probe $host.host_id --root $root
& $exe fleet placement --root $root
```

Expected from `probe` (exit 0):

```json
{
  "callsign": "probe-fail",
  "health": {
    "reachable": false,
    "model_count": 0,
    "latency_ms": <integer>,
    "error": "network error: failed to connect to Ollama: ...",
    "checked_at": "<rfc3339>"
  },
  "enrolled": false,
  "trusted": false
}
```

The exact `error` text is OS/adapter specific. The invariant to check is:
`health.reachable == false`, `health.error != null`, `health.model_count == 0`,
and the command exits 0. Because the host is unenrolled/untrusted (and
unreachable), `fleet placement` returns `[]`.

Wrong-secret HMAC rejection is implemented in `aegis-fleet/src/remote.rs` and was
verified by Batch 4; a two-host live handshake is **[O/H]** and not run here.

---

## 5. Secret scan — [A]

From `D:\Aegis`:

```powershell
rg -n --hidden --glob '!target/**' --glob '!.git/**' --glob '!var/**' `
  --glob '!*.db' --glob '!*.proof' --glob '!docs/**' `
  -e '-----BEGIN (RSA |EC |OPENSSH |PGP )?PRIVATE KEY-----' `
  -e '(?i)(api[_-]?key|client[_-]?secret|password)\s*[:=]\s*["''][^"'']{16,}["'']' .
```

Expected: no matches. (`rg` exits 1 when it finds nothing; that is the pass
condition for this scan.)

Also confirm no live state is tracked:

```powershell
git ls-files var
git ls-files | Select-String -Pattern 'owner\.proof|\.db$'
```

Expected: both produce **no output**.

---

## 6. Automated vs owner/hardware matrix

| Evidence item | Status | Notes |
|---|---|---|
| fmt / clippy / build / package tests | **Automated** | §1; 143 passed |
| Security tests (token/path leakage, markers, STOP) | **Automated** | §1.4 targeted runs |
| Core install / verify / inspect | **Automated** | CLI on temp root (§2) + tests |
| Two-Core DB/instance separation | **Automated** | `core_regression.rs` |
| Roster #4/#5 prohibited, #7 blank | **Automated** | `acceptance_roster.rs`, `core_regression.rs` |
| Cross-Core memory isolation | **Automated** | `aegis-core-profiles/tests/core_regression.rs` |
| Diagnostics read-only report | **Automated** | CLI on temp root (§3) |
| Fleet probe failure typing | **Automated** | CLI on temp root (§4) |
| Secret scan / no tracked DB/proof | **Automated** | §5 |
| Windows DPAPI owner proof | **Automated on Windows** | `windows_owner_proof_is_dpapi_protected` (Windows-only) |
| Linux owner-proof path | **UNTESTED** | No Linux run; fallback is a 0600 plaintext file |
| Two-host Artemis enrollment + HMAC handshake | **Owner/hardware** | Needs owner authorization, install, credentials, out-of-band secret |
| Live GPU/gaming measurement | **Owner/hardware** | Needs GPU + game session |
| TTS/STT/audio | **Owner/hardware** | Needs engine, models, audio device |
| Native renderer / GUI window | **Owner/hardware** | Current GUI is an HTTP dashboard |
| OS service registration / Linux runtime | **Owner/hardware** | Not executed |
| Destructive purge with receipt | **Not implemented** | Only `uninstall-preserve` exists |

---

## 7. Cleanup

```powershell
Remove-Item -LiteralPath $root -Recurse -Force
```

Expected: the temp root is gone. The live `D:\Aegis\var` tree was never touched.
If any `--print-owner-token` output was captured, delete it; it is development
proof material, not production proof.

---

## 8. Acceptance statement

- These results are **self-produced implementation evidence**, not independent
  QA. Independent acceptance is **NOT AVAILABLE**.
- **Lab development: GO** for the owned crates.
- **Production: NO-GO.** Reasons include symmetric HMAC over plain `http`,
  Windows-only-tested owner proof (Linux untested), no module sandbox, content
  integrity without authenticity, and no independent QA.
- **Artemis/two-host: BLOCKED** pending owner authorization and hardware.
