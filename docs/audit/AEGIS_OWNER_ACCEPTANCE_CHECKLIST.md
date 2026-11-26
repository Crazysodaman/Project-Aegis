# L. Owner Acceptance Checklist

**Purpose:** practical owner decisions/actions required before implementation
batches and before any deployment. Nothing here has been executed.

## 1. Audit & governance decisions

- [ ] Review `AEGIS_MASTER_COMPLETION_ASSESSMENT.md` and the A–L registers.
- [ ] Decide the governing specification: mirror/point to the external v0.3.0
      `AEGIS_MASTER_SPECIFICATION` (legacy `docs/specs`) or keep v0.2.0 as the
      in-repo authority. Record the decision.
- [ ] Confirm whether legacy voluntary-governance/collective-decision designs are
      in scope, since they are absent from v0.2.0.
- [ ] Confirm whether the observed dirty state of `D:\SofiaDev\SofiaAdaLyra`
      (modified tracked files + untracked specs at `d1666be`) is expected; Aegis
      never wrote it, but "untouched" cannot be proven from repo state.
- [ ] Authorize a commit/checkpoint of the current `D:\Aegis` working tree
      (HEAD is `2bd2513`; 52 modified + 212 untracked).

## 2. Identity & Core decisions

- [ ] Approve binding each roster number to a canonical UUID (Sofía
      `a9a0a604…`) and the Constitution digest `F30AFEDF…`, with mismatch
      rejection; or formally withdraw the preservation claim.
- [ ] Decide whether to import/authorize Sofía's canonical personality,
      appearance, fox ears/tail assets, and voice file from the legacy reference,
      with digest verification (no regeneration/substitution).
- [ ] Resolve #6 Vera authorization (docs say design-only; later work installs it).
- [ ] Confirm #4/#5 remain design-only and #7 remains blank/owner-named.

## 3. Living-environment scope decisions

- [ ] Wardrobe baseline: 275/23/47 (historical) vs 295/51 (later legacy); decide
      whether/how to import records into a new catalog/instance schema.
- [ ] Approve shared item catalog + per-Core instance model (prerequisite for
      transfers/trading).
- [ ] Approve interactive-object/world-space model (books/notes/objects).
- [ ] Approve action registry + consent/eligibility architecture before any adult
      capability work.
- [ ] Decide A-04 adult eligibility/consent model; confirm adult capability stays
      disabled/fail-closed until implemented. (Consent must be affirmative,
      interaction-specific, revocable; no Core may be forced; #7 gets nothing.)
- [ ] Approve autonomy scope (goals/interests/routines/scheduler), bounded with
      budgets/quiet periods/anti-harassment.
- [ ] Decide social-agency scope (invitations/accept/decline/leave/mute/block).

## 4. Security & data decisions

- [ ] Decide fix vs formal acceptance for D20-02 (verify-backup marker swap) and
      D20-03 (safe.db recreation).
- [ ] Decide whether to strengthen sidecar ownership checks with
      `database_instance_uuid` and reject missing ownership rows (D20-18).
- [ ] Decide GUI owner-auth model (D20-17) before enabling mutations broadly.
- [ ] Decide module sandbox/signature requirements before untrusted modules.
- [ ] Decide transport security: TLS proxy requirement / mTLS for non-private
      networks; HMAC alone is plain HTTP.
- [ ] Decide backup placement/retention (B-01/B-06) and whether `owner.proof`
      remains in backups.

## 5. Deployment prerequisites

- [ ] Artemis: authorize agent install, secret provisioning, enrollment,
      firewall/service changes; provide credentials/access.
- [ ] Debian 13 Sofía VM: provide VM; authorize systemd install; verify reboot
      recovery and Linux owner proof.
- [ ] Windows: authorize clean-room install/upgrade/uninstall verification and
      OS service decision (SCM vs process manager).
- [ ] Native GUI: approve toolkit dependency and internal crate links.
- [ ] Voice: approve microphone access and (optional) Whisper/Qwen3-TTS models;
      playback/device stack.
- [ ] Avatar: approve image decoder and GPU/window stack for real rendering.
- [ ] Module ecosystem: approve catalog recovery and implementation priorities.

## 6. Acceptance & release

- [ ] Approve the roadmap/batch order in `AEGIS_IMPLEMENTATION_ROADMAP.md`.
- [ ] Commission a genuinely independent external reviewer (not an author).
- [ ] After implementation: repeat full gates/live tests, verify no regressions,
      and record evidence per requirement.
- [ ] Record owner acceptance of deployed software and remaining known limits.
- [ ] Authorize production deployment only after Gates C–F pass.

## 7. Immediate smallest blocker-removal list

1. Commit/checkpoint authorization (D20-01).
2. D20-02/D20-03 disposition.
3. Artemis access + Debian VM.
4. Spec governance + identity/Constitution decision.
5. Adult/consent and legacy-data decisions (they gate Batches D/F).

**No implementation batches will start until the owner authorizes the roadmap and
the above decisions.**
