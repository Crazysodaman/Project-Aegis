# AEGIS FINAL — Security Audit

**HEAD:** `2bd2513`

## Posture

- **Owner authority:** mutating Core/platform/lifecycle operations require owner
  proof; STOP blocks effects. Windows DPAPI owner proof. Linux owner-proof
  fallback is **untested**.
- **Enrollment:** explicit owner action; hosts untrusted on add; HMAC-SHA256
  challenge-response binds nonce/host/callsign; constant-time compare; wrong
  secret rejected. Symmetric secret over **plain HTTP**; public plain-HTTP
  inference rejected by policy; TLS must terminate externally.
- **Core isolation:** per-Core DBs with ownership markers; memory/personal/
  conversation/asset/social/embedding stores scoped to the owning Core; cross-Core
  copy rejection tested.
- **Backup integrity:** per-DB ownership + SHA-256; v2 manifest; restore verifies.
- **Secrets:** no secrets in tracked/untracked source; `owner.proof`/`var/` ignored.
- **Modules:** policy/permissions/lifecycle enforced; **no OS sandbox** (trusted
  local code; environment isolation is not a security boundary).
- **Packaging:** `.corepkg` validates digest/roster/path containment; integrity is
  content SHA-256, not publisher signing.

## Open defects (Agent 16/20)

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| D20-02 | Medium | `verify-backup` can report Verified after Platform/Safe marker swap with rewritten digests; restore fails closed | Owner fix/accept |
| D20-03 | Medium | Deleting `safe.db` recreates inactive STOP on next Platform start; not tamper-evident | Owner fix/accept |
| D20-01 | High (process) | Uncommitted work at `2bd2513` | Owner commit |

## Limitations

- No transport encryption (HMAC auth only); no mTLS.
- DPAPI ties owner proof to Windows/current user.
- No OS module sandbox; no signed packages; no tamper-evident STOP across deletion.
- No independent security review / penetration test.

## Verdict

No critical remote-code or auth-bypass was found in review. Medium integrity and
tamper-evidence gaps remain. **Security acceptance: NO-GO for production** until
D20-02/D20-03 are fixed or formally accepted and an independent review occurs.
