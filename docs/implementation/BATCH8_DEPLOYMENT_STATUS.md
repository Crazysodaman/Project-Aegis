# Batch 8 — Deployment Status & Authorization

**Date:** 2026-11-26

## Status

Repository-side implementation complete for M1–M7. **No deployment or commit was
performed.** Production remains NO-GO.

## New dependency (owner awareness)

- **`chrono-tz = 0.10.0`** added to the workspace and `aegis-platform`, required
  for IANA timezone identifiers and DST. This is the only new third-party crate.
- Weather uses the system **`curl.exe`** (Windows) for HTTPS; no Rust TLS/HTTP
  crate. On Linux, `curl` must be installed for live weather.

## Owner authorization gates (not executed)

1. **Artemis**: install agent, provision `AEGIS_REMOTE_SECRET`, enroll, firewall/
   service changes, two-host distribution. (192.168.1.55 reachable; no creds used.)
2. **Sofia VM (Debian 13)**: build/install systemd unit, reboot recovery test,
   Linux owner-proof backend.
3. **Time zone/location configuration** on non-disposable roots: owner sets
   `world.timezone`, `world.latitude`, `world.longitude`.
4. **Weather provider**: live Open-Meteo calls are read-only and keyless, but
   enabling scheduled refresh/network egress is an owner operational choice.
5. **Microphone/STT, native GUI, GPU avatar renderer**: dependencies and hardware
   access not approved/implemented in this batch.
6. **Commit/checkpoint** and production deployment.

## Configuration

- `aegis world set-timezone --root <path> --timezone America/Chicago`
- `aegis world set-location --root <path> --lat 41.88 --lon -87.63`
- `aegis world weather --root <path>` (live)
- `aegis world status --root <path>` (stored snapshot)
- `aegis personal show --root <path> --core <uuid>`
- Chat automatically injects world + continuity + emotion (unless `--no-context`).

## Distribution / multi-host

Not implemented. World snapshots are Platform-owned and stored in `platform.db`;
the CLI injects them into Core turns. IPC/background distribution to remote Cores
and Artemis/Sofia VM distribution require owner authorization and a follow-up
milestone.

## Weather operations

- Provider: Open-Meteo (no API key), TTL cache 900 s, typed failures.
- Failure behavior: no invented weather; snapshot remains valid with weather
  absent/unavailable; Core operation continues.

## Commit / production

- No commit created. No production activation. Owner approval required.
