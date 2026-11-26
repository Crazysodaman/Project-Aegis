# Batch 7 — Deployment Instructions & Outstanding Authorization

**Date:** 2026-11-26

Nothing here has been executed on external hosts. Owner approval is required.

## Artemis Fleet agent (192.168.1.55)

Observed: reachable (WinRM 5985/RDP/SMB/443), no SSH, no Ollama, no agent, no
credentials used. Prepared: `FleetAgentPlan`, preflight/doctor, transport policy,
secret-free templates, live HMAC re-verification (Venus LAN).

Required approval: agent install, secret provisioning, enrollment, firewall/
service changes. Runbook: deploy agent → provision `AEGIS_REMOTE_SECRET` out of
band → verify HMAC → enroll → probe/heartbeat → failure/recovery test → TLS front
if non-private. Rollback: revoke, stop/remove agent, restore prior config.

## GPU gameplay

Approval/participation required for a real Phasmophobia session. Use
`aegis-fleet-measure capture` + PresentMon `timestamp_ms,frame_time_ms` export +
`compare`. No gameplay data exists yet.

## Debian 13 / Sofia VM

Approval required to build, install the systemd unit, and reboot-test. Dry-run
executor exists; nothing installed. Verify `systemd-analyze verify`, start/stop/
restart, kill -9 recovery, reboot, upgrade, rollback.

## GUI / Voice / Avatar / P0

- Native GUI: needs approved `eframe`+`egui` or `tao`+`wry` (and internal crate
  links to Fleet/profiles/embodiment/cognition).
- Microphone STT: needs mic access + Whisper/faster-whisper; large model download
  approval. Current TTS and file-based STT use built-in Windows SAPI.
- Avatar: needs image decoder and/or GPU/window stack.
- P0 world/continuity: needs weather provider/API approval, timezone owner
  decision, and backup-format change awareness. See
  `BATCH7_P0_WORLD_CONTINUITY_AUDIT.md`.

## Commit / production

No commit, checkpoint, or deployment performed. Owner approval required.

## Authorization checklist

- [ ] Artemis install + secret + enrollment
- [ ] Artemis firewall/service changes
- [ ] Manual gameplay/FPS session
- [ ] Debian 13 service install + reboot test
- [ ] New GUI/voice/avatar dependencies
- [ ] Weather provider/timezone configuration (P0)
- [ ] Backup format change for per-Core DBs (P0 M1)
- [ ] Commit/checkpoint
- [ ] Production deployment
