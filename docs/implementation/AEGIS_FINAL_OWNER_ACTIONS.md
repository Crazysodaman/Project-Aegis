# AEGIS FINAL — Owner Actions

**HEAD:** `2bd2513` · no commit · no production deployment

## Required to remove blockers

1. **Artemis (192.168.1.55)** — authorize and provide access to:
   - install the Fleet agent and provision `AEGIS_REMOTE_SECRET` out of band;
   - enroll Artemis through owner proof;
   - change firewall/service configuration if needed;
   - run the two-host authenticated probe/inference/failure/recovery test.
   Do not use plain HTTP on untrusted networks; front non-private inference with TLS.
2. **Debian 13 (Sofía VM)** — provide an authorized VM and approve service
   installation; verify build, `systemd-analyze verify`, start/stop/restart,
   kill -9 recovery, reboot recovery, upgrade/rollback, and Linux owner proof.
3. **Native GUI** — approve a toolkit dependency (`eframe`+`egui` or `tao`+`wry`)
   and any additional internal crate links; then implement the real window.
4. **Voice hardware/models** — approve microphone access and (optionally)
   Whisper large-v3-turbo / Qwen3-TTS downloads; file-based SAPI TTS/STT already work.
5. **Avatar renderer** — approve an image/GPU/window stack if native pixels are
   required (current BMP is schematic CPU output).
6. **Security disposition** — decide to fix or formally accept D20-02
   (`verify-backup` marker swap) and D20-03 (`safe.db` recreation); D20-01 requires
   a commit checkpoint.
7. **Commit/checkpoint** — authorize committing the working tree (no commit made).
8. **Independent external QA** — commission a reviewer who is not an author.
9. **Owner acceptance** — record acceptance after the above.
10. **Production deployment** — authorize only after Gates C–F pass.

## Not required for continued development

Repository-side work can continue (tests/docs/refactors) without these approvals;
do not treat silence as authorization for host/system changes.
