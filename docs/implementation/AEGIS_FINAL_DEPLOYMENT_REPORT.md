# AEGIS FINAL — Deployment Report

**HEAD:** `2bd2513` · no commit · no production deployment

## Windows / Venus

- Local Windows build/test/run verified (RTX 3080 Ti, Ollama 0.40.17).
- Installer/upgrade/uninstall tested (preflight, backup-gated upgrade, downgrade
  refusal, preserve/delete); fresh clean-room reinstall not repeated in FINAL.
- Windows service is a process manager; OS SCM registration remains absent.
- Live runtime verified on a disposable root: install, world timezone/location,
  live weather, Core install, chat with world/continuity context, personal state,
  backup→restore.
- SAPI TTS/STT live; avatar BMP sequences live; GPU inference-pressure measured.

## Artemis (192.168.1.55)

- **NOT DEPLOYED / NOT VERIFIED.** Reachable (WinRM/RDP/SMB/443), no SSH/Ollama/
  agent; no credentials/authorization used. Fleet agent runbook and secret-free
  templates exist; enrollment/firewall/service changes are owner-gated.
- No two-host or cross-host claim is made.

## Debian 13 (Sofia VM)

- **NOT INSTALLED / NOT VERIFIED.** systemd unit generation, install plan,
  restart/health specs, and runbook exist; no VM was available. No
  `systemd-analyze`, service enable/start, reboot recovery, or Linux owner-proof
  test was performed.
- Linux owner proof remains untested; `curl` required for weather.

## Commands (implemented, Windows-verified where noted)

```
aegis install --root <path> --headless
aegis world set-timezone --root <path> --timezone <IANA>
aegis world set-location --root <path> --lat <f> --lon <f>
aegis world weather|status --root <path> [--refresh-weather]
aegis core install <uuid> --roster <n> --root <path>
aegis core install-package <pkg-dir> --root <path>
aegis voice synthesize --root <path> --core <id> --text <s> --out <wav> [--rate n] [--volume n]
aegis chat send <core> <session> --model <m> --prompt <p> --root <path>
aegis personal show --root <path> --core <id>
aegis backup --root <path> --to <dir>
aegis restore --from <dir> --root <new-root>
aegis diagnostics|tui|gui|service ... --root <path>
```

## Deployment gate

**DEPLOYMENT NO-GO** until Artemis authorization, Debian runtime verification,
native GUI decision, and owner acceptance are complete.
