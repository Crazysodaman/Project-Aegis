# Blockers and Residual Risks

## Owner-authority blockers

- **A-04:** adult eligibility jurisdiction/verification method. Adult capability remains unavailable/fail-closed.
- **B-01:** final Hephaestus backup target and Platform authority relocation placement. No final placement assumed.
- **B-06:** backup retention and RPO/RTO. No target claimed.

## Technical residual risks

| Severity | Issue | Current state | Next action |
|---|---|---|---|
| Medium | TOCTOU between metadata checks and SQLite open | Symlink/reparse checks, canonicalization, and permission hardening exist; an adversarial same-user swap test was not run | Add handle-pinning or fail-closed revalidation test |
| Low | Windows ACL policy not explicitly asserted | Unix modes 0700/0600 applied; Windows relies on user profile ACLs | Document and test Windows ACL expectations |
| Low | WAL/sidecar stress | Integrity layer rejects unexpected `-wal`/`-shm`; no concurrent-writer stress | Add concurrent-writer fixture |
| Medium | `platform_core` and `core_registry` coexist | `core_registry` is authoritative; `platform_core` is legacy-compatible | Plan deprecation/migration note |
| Medium | Module execution sandbox absent | Process/env isolation, timeouts, forced kill, and crash detection are implemented; no OS sandbox or hard CPU/memory limit | Add OS-level sandbox only if required |
| Medium | Automatic gaming-load yield not live-verified | RTX 3080 Ti telemetry/admission and cooperative model-unload yield are live-verified; no gaming workload was run | Add gaming-load detection and exercise under a controlled load |
| Low | Module tool isolation is same-account process execution | Cleared env, private input/output files, timeouts, output caps, and Enabled-only gating exist; no OS sandbox | Documented trust boundary for owner-controlled modules |

| Low | Social UX and Core-to-Core awareness incomplete | Private groups and per-Core relationships are implemented and membership-enforced; no social GUI/TUI, automatic Core awareness, or trust negotiation | Add social UI and explicit awareness/trust APIs |
| Low | No avatar renderer or audio voice provider | Asset pipeline, appearance, wardrobe, and voice-profile configuration exist and are tested | Integrate a renderer/voice provider when one is chosen |
| High | Linux not built/tested | Windows-only verification | Build and run full suite on Linux |
| Medium | OS service installation absent | Process-level service manager only | Add SCM/systemd registration when deployment target is fixed |
| High | Social/voice/embodiment/fleet absent | Not started | Implement only under owner-approved scope |

## Environmental limitations

- Windows-only build/test performed; Linux not verified.
- No production host, OS service installation, real model, voice, or avatar verification.
- No commit or push; all work uncommitted and untracked.

## Compliance notes

- A-04, B-01, and B-06 are not settled by any implementation or test.
- Cores #4/#5 remain prohibited; Core #7 identity is not invented.
- No adult capability is implemented.
- Digest integrity is not presented as authenticity.
