# Batch 7 — Reproducible Acceptance Evidence (Agent 10)

**Date:** 2026-11-26
**Repository:** `D:\Aegis`
**Baseline HEAD:** `2bd2513` (unchanged)
**Environment:** Windows PowerShell 5.1, Rust toolchain compatible with the
workspace.

This package is **self-produced implementation evidence**, not independent QA.
It uses only package-scoped gates (no whole-workspace gate) and disposable
temporary roots. Do not point any command at the live `D:\Aegis\var` data.

Legend: **[A]** automated and run by this agent; **[N]** not run here;
**[O/H]** requires owner authorization and/or hardware.

---

## 0. Setup

```powershell
Set-Location D:\Aegis
```

Optional disposable root for any manual CLI work (not used by the automated
package tests, which create their own tempdirs):

```powershell
$root = Join-Path $env:TEMP ("aegis-b7-accept-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $root | Out-Null
$root
```

Expected: a new empty directory under `%TEMP%`.

---

## 1. Package quality gates — [A]

All commands are explicitly package-scoped.

### 1.1 Format check

```powershell
cargo fmt -p aegis-safe -p aegis-core-storage -p aegis-core-runtime `
  -p aegis-core-install -p aegis-roster -p aegis-core-profiles `
  -p aegis-core-identity -p aegis-core-integrity -p aegis-core-registry -- --check
```

Expected: no output, exit 0.
**Actual: exit 0, no diff.**

### 1.2 Clippy

```powershell
cargo clippy -p aegis-safe -p aegis-core-storage -p aegis-core-runtime `
  -p aegis-core-install -p aegis-roster -p aegis-core-profiles `
  -p aegis-core-identity -p aegis-core-integrity -p aegis-core-registry `
  --all-targets -- -D warnings
```

Expected: `Finished ...`, exit 0, no warnings.
**Actual: exit 0, no warnings.**

### 1.3 Tests (nine owned packages)

```powershell
cargo test -p aegis-safe -p aegis-core-storage -p aegis-core-runtime `
  -p aegis-core-install -p aegis-roster -p aegis-core-profiles `
  -p aegis-core-identity -p aegis-core-integrity -p aegis-core-registry
```

Expected: all pass.
**Actual: 157 passed, 0 failed, 0 ignored** (Batch 6 baseline 143; **+14**).

Per package (actual):

| Package | Passed |
|---|---|
| aegis-safe | 14 |
| aegis-core-storage | 17 |
| aegis-core-runtime | 14 |
| aegis-core-install | 22 |
| aegis-roster | 15 |
| aegis-core-profiles | 18 |
| aegis-core-identity | 22 |
| aegis-core-integrity | 16 |
| aegis-core-registry | 19 |
| **Total** | **157** |

---

## 2. New Batch 7 tests — targeted re-run — [A]

```powershell
cargo test -p aegis-safe --test security_acceptance
cargo test -p aegis-core-storage --test security_markers
cargo test -p aegis-core-runtime --test stop_security
cargo test -p aegis-core-install --test core_regression
cargo test -p aegis-roster --test acceptance_roster
cargo test -p aegis-core-profiles --test core_regression
```

Expected / **actual** pass counts (Batch 6 → Batch 7):

| Test binary | Before | After | New tests |
|---|---|---|---|
| `aegis-safe` `security_acceptance` | 4 | **6** | token-not-in-DB/audit; uninitialized error path opacity |
| `aegis-core-storage` `security_markers` | 2 | **4** | corrupt DB rejected; DB bound to owning Core UUID |
| `aegis-core-runtime` `stop_security` | 2 | **5** | monotonic revision across reopen; STOP blocks suspended-Core start without reactivating; unauthorized error opacity |
| `aegis-core-install` `core_regression` | 3 | **5** | three cores distinct/stable instances across reopen; suspend/reactivate preserves identity + history |
| `aegis-roster` `acceptance_roster` | 3 | **4** | approved names reject impostors; designations exact |
| `aegis-core-profiles` `core_regression` | 2 | **6** | foreign memory path opacity; cross-Core import rejected; cross-Core mutation blocked; suspend/reactivate preserves memory |

All six commands exit 0 with the counts above.

---

## 3. Repository integrity — [A]

```powershell
git -C D:\Aegis rev-parse HEAD
git -C D:\Aegis status --short
```

**Actual:** HEAD is `2bd2513`; the working tree still contains the pre-existing
Batch 5/6 modifications and untracked files plus these Batch 7 test/doc changes.
No commit, reset, rebase, or clean was performed by this agent.

```powershell
git -C D:\Aegis ls-files var
git -C D:\Aegis ls-files | Select-String -Pattern 'owner\.proof|\.db$'
```

Expected / **actual:** both produce **no output** (no tracked `var/`, no tracked
`owner.proof` or `*.db`).

Confirm `D:\SofiaDev\SofiaAdaLyra` was not modified (this agent never referenced
it for writes):

```powershell
Test-Path D:\SofiaDev\SofiaAdaLyra
```

**Actual:** `True` (path exists; untouched by this agent).

---

## 4. Secret scan — [A]

```powershell
rg -n --hidden --glob '!target/**' --glob '!.git/**' --glob '!var/**' `
  --glob '!*.db' --glob '!*.proof' --glob '!docs/**' `
  -e '-----BEGIN (RSA |EC |OPENSSH |PGP )?PRIVATE KEY-----' `
  -e '(?i)(api[_-]?key|client[_-]?secret|password)\s*[:=]\s*["''][^"'']{16,}["'']' D:\Aegis
```

Expected: no matches; `rg` exits 1 when nothing is found (that is the pass
condition).
**Actual: no matches, exit 1.**

---

## 5. Corrections to Batch 6 evidence

These correct or bound Batch 6 claims (see
`BATCH7_AGENT10_SECURITY_CORE.md` §8 for detail):

1. `Cargo.lock` is **not** unchanged: it gained one `sha2` line, and
   `crates/aegis-fleet/Cargo.toml` added `sha2.workspace = true`. `sha2` is an
   existing workspace dependency, so no **new third-party crate** was introduced,
   but the lockfile and that manifest do differ from HEAD.
2. The workspace-wide "614 tests / clippy --workspace exit 0" figures are
   **self-reported**; this agent ran only package-scoped gates and did not verify
   them.
3. Batch 6's "no independent QA" statements are accurate and are repeated here;
   none of these documents should be cited as independent acceptance.
4. Live TTS/avatar artifacts, Artemis/two-host Fleet, Debian 13 runtime,
   gameplay/FPS capture, native GUI, and cross-host artifact retrieval remain
   **unverified owner/hardware claims**.

---

## 6. Unverified / not performed (explicit list) — [N]/[O/H]

This evidence does **not** establish any of the following:

- **No independent QA or owner acceptance.** All work is by the implementer.
- **No whole-workspace test run** (the "614 tests" claim was not re-run here).
- **No Artemis / two-host Fleet verification** (no owner credentials/hardware).
- **No Linux run.** Windows DPAPI is exercised; the Linux plaintext-0600 owner
  proof fallback is untested.
- **No real gameplay / FPS / frame-time capture.** Gaming-safe operation is
  unproven.
- **No live TTS/STT/audio playback** and **no native GUI window**; previous
  TTS/BMP results are self-reported and not reproduced here.
- **No Debian 13 service install/reboot verification.**
- **No cross-host Artifact Library retrieval**; only in-process tests exist.
- **No module OS sandbox.** Module policy was reviewed read-only; this agent owns
  no `aegis-modules` tests.
- **No destructive purge** (only `uninstall-preserve` exists).
- **No CLI operational smoke test** was run by this agent (package tests create
  their own tempdirs).

---

## 7. Cleanup

```powershell
if ($root) { Remove-Item -LiteralPath $root -Recurse -Force }
```

Expected: the disposable temp root (if created in §0) is gone. The live
`D:\Aegis\var` tree was never opened.

---

## 8. Acceptance statement

- Results above are **self-produced implementation evidence**, not independent
  QA. **Independent acceptance is NOT AVAILABLE.**
- **Lab development: GO** for the owned packages (green, additive, recoverable).
- **Production: NO-GO** — plain-HTTP symmetric HMAC, Windows-only-tested owner
  proof, no module sandbox, content-only integrity, and no independent review.
- **Artemis / Linux / gameplay / native GUI: BLOCKED** on owner authorization,
  hardware, or approved dependencies.
