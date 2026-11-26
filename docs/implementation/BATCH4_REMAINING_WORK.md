# Batch 4 — Remaining Work Inventory

**Date:** 2026-11-26
**Repository:** `D:\Aegis` (baseline `2bd2513`)
**Purpose:** Evidence-based inventory of remaining work. This is **not**
permission for scope expansion; Batch 4 did not implement these systems.

Legend: Yes = present/verified · Partial = scaffolding/config only · No = absent.
Statuses are independent; one does not imply another.

| # | Capability | Implemented | Tested | Integrated | Live-verified | Independently accepted |
|---|---|---|---|---|---|---|
| 1 | Voice generation / TTS | No | No | No | No | No |
| 2 | Speech recognition / STT | No | No | No | No | No |
| 3 | Avatar rendering / animation | Partial | Partial | No | No | No |
| 4 | Native GUI window | No | No | No | No | No |
| 5 | Rust TUI / dashboard completion | Partial | Partial | Partial | Partial | No |
| 6 | OS service registration | Partial | Partial | No | No | No |
| 7 | Linux runtime and deployment | No | No | No | No | No |
| 8 | Cross-host Artifact Library | Partial | Partial | No | No | No |
| 9 | Core asset and voice packaging | Partial | Partial | Partial | No | No |
| 10 | Social runtime + multi-Core group UX | Partial | Yes | Partial | No | No |
| 11 | Process modules / real execution | Partial | Yes | Yes | Partial | No |
| 12 | Independent QA and owner acceptance | No | No | No | No | No |

## 1. Voice generation / TTS

- **Existing:** `aegis-embodiment` stores a `VoiceProfile` (voice id, display
  name, optional sample asset) and validates sample asset kind.
- **Missing:** any synthesis engine, audio format handling, playback device,
  streaming, interruption, per-Core voice binding to output.
- **Dependencies:** audio output backend (WASAPI/CoreAudio/ALSA/PulseAudio),
  a TTS provider or model, asset storage integration.
- **Verification required:** live audio output on Windows; per-Core voice
  isolation; STOP/interruption; latency and resource bounds.
- **Blocks production readiness:** Yes (declared Aegis embodiment capability).

## 2. Speech recognition / STT

- **Existing:** none (no capture, no VAD, no transcription).
- **Missing:** microphone capture, resampling, VAD, STT provider, transcript
  handoff to conversation, privacy controls.
- **Dependencies:** audio capture backend, STT model/provider, privacy policy.
- **Verification required:** live capture + transcription; no audio persistence
  without owner consent; STOP.
- **Blocks production readiness:** Yes.

## 3. Avatar rendering and animation

- **Existing:** `aegis-embodiment` appearance config (avatar asset, expressions,
  gestures, wardrobe, theme, Aegis palette) with SHA-256 asset integrity;
  `aegis-ui` exposes appearance/asset data in the dashboard.
- **Missing:** any renderer, animation state machine, expression/gesture
  playback, lip-sync, GPU/window integration.
- **Dependencies:** renderer (native or web), asset pipeline, UI window.
- **Verification required:** live rendering; asset integrity on load; animation
  correctness; resource use.
- **Blocks production readiness:** Yes (declared embodiment capability).

## 4. Native GUI window

- **Existing:** loopback HTTP dashboard (`aegis-ui::gui`) and HTML generation;
  read-only by default, mutation flag.
- **Missing:** native window/webview host, OS integration, packaging.
- **Dependencies:** windowing/webview library; platform testing.
- **Verification required:** live window on Windows; security of local server.
- **Blocks production readiness:** No (dashboard is usable), but required for
  the stated native GUI deliverable.

## 5. Rust TUI / dashboard completion

- **Existing:** `run_tui` and `run_tui_once`; dashboard read-only/mutation flows;
  dashboard tests (`gui_dashboard.rs`).
- **Missing:** TUI interaction tests; group/social UX; voice/avatar controls;
  provider/model controls; richer status.
- **Dependencies:** UI model completion.
- **Verification required:** scripted TUI interaction tests; live operator use.
- **Blocks production readiness:** No.

## 6. OS service registration

- **Existing:** `aegis-service` process manager (start/stop/status, detached
  `aegis-service-host`, PID/socket/heartbeat) — verified live.
- **Missing:** Windows SCM registration, systemd units, boot/reboot recovery,
  privilege handling, uninstall cleanup.
- **Dependencies:** platform-specific service APIs; installer integration.
- **Verification required:** install/remove/reboot recovery on Windows and Linux.
- **Blocks production readiness:** Yes.

## 7. Linux runtime and deployment

- **Existing:** codebase targets Windows paths/DPAPI; no `cfg(target_os =
  "linux")` implementation found; Unix IPC/owner-proof fallbacks are untested.
- **Missing:** Linux build validation, owner-proof backend, IPC transport,
  service units, installer, path handling, asset/audio/GPU telemetry.
- **Dependencies:** Linux CI/host; DPAPI replacement.
- **Verification required:** build + test + runtime on Linux; GPU telemetry
  (`nvidia-smi` present but untested on Linux).
- **Blocks production readiness:** Yes.

## 8. Cross-host Artifact Library

- **Existing:** per-Core local `assets.db` + files with SHA-256 integrity, export.
  Blueprint names the Artemis Sofía VM as the intended primary Artifact Library.
- **Missing:** remote artifact storage/placement, replication, integrity over
  network, host authorization for artifacts, failover.
- **Dependencies:** Fleet agent, authenticated transport, storage replication.
- **Verification required:** real Artemis/Venus artifact transfer; interruption
  and recovery; integrity verification.
- **Blocks production readiness:** Yes (for the intended multi-host layout).

## 9. Core asset and voice packaging

- **Existing:** roster profiles encode approved identity/personality/appearance
  where approved; Vera points to an external spec; Sofía is preservation-only.
- **Missing:** packaged per-Core asset/voice bundles, import validation, versioning,
  and delivery to Cores.
- **Dependencies:** Artifact Library, asset pipeline.
- **Verification required:** package install + integrity on a fresh root; Sofía
  legacy preservation.
- **Blocks production readiness:** Yes.

## 10. Social runtime and multi-Core group UX

- **Existing:** Platform-owned private groups with membership enforcement and
  append-only events; per-Core relationship records; tests.
- **Missing:** Core-to-Core awareness/trust semantics, group UX in GUI/TUI,
  notification flows, cross-Core conversation presentation.
- **Dependencies:** UI completion, Core messaging design (owner decision).
- **Verification required:** multi-Core live group interaction; privacy checks.
- **Blocks production readiness:** No (semantics are owner-design),
  but required for the declared social deliverable.

## 11. Process modules and real execution

- **Existing:** strict manifest, package materialization, owner-gated lifecycle,
  supervised child processes, heartbeat/crash/forced-kill, module tools.
- **Missing:** functional module roster (inference, voice, assets, social,
  operational), OS-level sandbox/resource caps, real execution integrations.
- **Dependencies:** module packages; OS sandboxing.
- **Verification required:** real non-example modules; isolation under abuse.
- **Blocks production readiness:** No for the framework; Yes for the functional
  roster.

## 12. Independent QA and owner acceptance

- **Existing:** self-produced QA plus a separate OS process (same author).
- **Missing:** a genuinely independent reviewer/agent and owner acceptance.
- **Dependencies:** independent access to the repository and evidence.
- **Verification required:** independent reproduction of quality gates and live
  tests.
- **Blocks production readiness:** Yes.

## Batch 4 boundary

Batch 4 performed: repository recovery audit, real-gaming GPU-yield validation,
Artemis prerequisites inspection (deployment blocked without owner credentials),
cross-host resilience tests against a LAN-bound endpoint, and Platform/Core
regression. It did **not** implement items 1–12 above.
