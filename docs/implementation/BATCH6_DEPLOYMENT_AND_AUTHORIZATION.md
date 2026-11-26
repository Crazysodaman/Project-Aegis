# Batch 6 — Deployment Instructions & Outstanding Authorization

**Date:** 2026-11-26

Nothing in this document has been executed. All remote/system operations require
explicit owner approval.

## Artemis Fleet agent (192.168.1.55)

Prerequisites (observed): NetBIOS `ARTEMIS`, WORKGROUP; TCP 135/139/443/445/3389/
5985 open; no SSH; no Ollama (11434 closed); no Aegis agent observed.

Owner approval required before:
1. Installing any agent/software on Artemis.
2. Creating/modifying credentials (out-of-band `AEGIS_REMOTE_SECRET` ≥ 16 bytes).
3. Enrolling Artemis in the Fleet.
4. Changing firewall or host services.

Prepared repository helpers (`aegis-fleet`):
- `transport::TransportPolicy` — rejects public plain-http inference unless
  explicitly allowed; loopback/private-LAN permitted.
- `deployment::DeploymentReadiness` — blockers/warnings before enrollment.
- Secret-free agent configuration template (host_uuid, callsign, bind address,
  secret env-var name, role/capabilities, TLS-proxy note).

Runbook summary (after approval):
1. Provision the shared secret out of band (never in source/logs).
2. Build/copy the agent; set `AEGIS_REMOTE_SECRET` in the service environment.
3. Start agent; verify HMAC challenge-response from Venus.
4. Enroll via owner-authorized Fleet flow; record callsign/role/capabilities.
5. Add Artemis as General/Storage if it does not host inference; never advertise
   inference without a real endpoint.
6. Prove probe/heartbeat, failure typing, and recovery.
7. If inference is intended, expose an Ollama endpoint and front non-private
   traffic with TLS (adapter is http-only).

Rollback: revoke enrollment, stop/remove agent, restore prior firewall/service
configuration.

## Debian 13 / Sofia VM service

Owner approval required before installing systemd units or changing the VM.
Prepared: pure systemd unit + install plan, restart policy/backoff, health
classification. Verify on a disposable Debian 13 VM: `systemd-analyze verify`,
start, kill -9 recovery, reboot, upgrade, rollback.

## GPU gameplay validation

Owner participation required: start Phasmophobia manually, run
`aegis-fleet-measure capture`, export PresentMon `timestamp_ms,frame_time_ms`,
then `aegis-fleet-measure compare`. No gameplay data exists yet.

## GUI / Voice / Avatar dependencies

New dependencies require approval:
- GUI native window: `eframe`+`egui` or `tao`+`wry`.
- GUI data links: `aegis-fleet`, `aegis-core-profiles`, `aegis-embodiment`,
  `aegis-cognition` (internal workspace crates).
- Voice: Whisper large-v3-turbo / Qwen3-TTS model downloads + audio backend
  (Windows SAPI TTS already works without downloads).
- Avatar: image decoder and/or GPU/window stack.

## Commit

No commit/checkpoint has been made. Owner approval is required before creating
one. Recovery material is in `.tmp/b6-recovery/` (and `.tmp/b4-recovery/`).

## Outstanding authorization checklist

- [ ] Artemis agent install + secret provisioning + enrollment
- [ ] Artemis firewall/service changes (if any)
- [ ] Manual gameplay session (FPS/frame-time)
- [ ] Debian 13 VM service install + reboot test
- [ ] New GUI/voice/avatar dependencies
- [ ] Production service registration
- [ ] Commit/checkpoint creation
- [ ] Production deployment
