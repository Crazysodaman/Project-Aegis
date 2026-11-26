# E. Platform Completeness Register

**Scope:** Platform, Fleet, inference/GPU, world, backup, security, install,
GUI/TUI/CLI, diagnostics. Evidence is from source/tests/logs; "live" means
documented Venus/Windows evidence, not installation.

| Area | Implemented | Tested | Installed | Live-verified | Classification |
|---|---|---|---|---|---|
| Platform identity/status/zero-Core | Yes | Unit/integration + real service spawn | No | Windows dev roots | IMPLEMENTED NOT FULLY VERIFIED |
| Lifecycle shutdown/OS service | Partial (`AtomicBool`) | Unit | No | No | PARTIAL |
| IPC | Yes (status/STOP/shutdown/ping) | Integration | No | Local | PARTIAL (no world/event methods) |
| Core registry/lifecycle/install | Yes | Integration + real process | No | Dev roots | IMPLEMENTED NOT FULLY VERIFIED |
| Core package `.corepkg` | Yes (local) | Integration | No | Local | PARTIAL |
| Fleet registry/enrollment/HMAC | Yes | Unit/integration, two-process LAN | No | LAN only | IMPLEMENTED NOT FULLY VERIFIED |
| Artemis two-host | No | No | No | No | BLOCKED (owner auth/credentials) |
| GPU telemetry/admission/yield | Yes | Unit + live inference pressure | No | Venus RTX 3080 Ti | IMPLEMENTED NOT FULLY VERIFIED |
| Gaming-safe FPS | No | No | No | No | MISSING (owner participation) |
| Inference queue | Yes (pure) | Unit | No | No | FOUNDATION ONLY (not wired) |
| World clock/DST/weather | Yes | Unit/integration/live weather | No | Venus | IMPLEMENTED NOT FULLY VERIFIED |
| World distribution | No (caller-injected) | Unit | No | No | PARTIAL (no IPC/background) |
| Calendar | No | No | No | No | MISSING |
| Personal emotion/continuity | Yes | Unit/integration/live | No | Venus | IMPLEMENTED AND VERIFIED (foundation) |
| Event→emotion | Yes | Unit/integration | No | Turn-level | IMPLEMENTED (non-atomic effects) |
| Memory/conversation | Yes | Unit/integration/live | No | Venus | IMPLEMENTED NOT FULLY VERIFIED |
| General Chat | No | No | No | No | MISSING |
| Social/groups/awareness | Partial (trust scalar) | Integration | No | No | PARTIAL |
| Adult/consent | No | No | No | No | MISSING/DEFERRED |
| Modules framework | Yes | Unit/integration/real process | No | No | FOUNDATION (0/55) |
| Module sandbox/signatures | No | No | No | No | MISSING |
| Backup v2 | Yes | Unit/integration/live `VACUUM INTO` | No | Temp roots | IMPLEMENTED (D20-02/03 open) |
| Installer Windows | Yes | Integration/preflight | No | CLI smoke | PARTIAL |
| Debian runtime/service | No (specs only) | No | No | No | DESIGNED/NOT IMPLEMENTED |
| Security owner proof/STOP | Yes (Windows) | Adversarial | No | No | PARTIAL (Linux untested) |
| Transport security | HMAC only | Unit | No | LAN | PARTIAL (plain HTTP) |
| GUI | HTTP dashboard | 37 tests | No | Local | PARTIAL (not native) |
| TUI | Refresh-only | 13 tests | No | Local | PARTIAL |
| CLI | Broad control plane | 23 E2E | No | Local | IMPLEMENTED |
| Diagnostics | Read-only aggregation | Integration | No | Local | PARTIAL (no social/embodiment/fleet/service health) |

## Cross-cutting findings

- No subsystem has OS-level **installed** evidence; no SCM/systemd registration.
- Env-gated live tests early-return without env vars, so the 942-test gate is not
  live coverage.
- UI `run/status/*.json` panels have no in-repo emitter; Fleet/GPU/world/personal/
  service appear UNAVAILABLE in normal operation.
- `platform_core` (legacy) and `core_registry` (authoritative) coexist without a
  settled deprecation rule.
- Service supervision is duplicated in three crates.
- Backup excludes `modules/`; uninstall deletion also omits module packages.
- `verify-backup` does not validate Platform/Safe application markers (D20-02).
- `safe.db` deletion recreates inactive STOP/empty audit (D20-03).
- GUI can drive owner-authorized lifecycle/STOP actions using a page session
  token when mutations are enabled; it is not a true owner-proof boundary.

## Platform verdict

**PARTIAL.** Strong local Windows foundation and honest documentation of limits;
not deployed, not cross-host, not OS-service installed, and without independent
acceptance.
