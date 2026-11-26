# Batch 5 — Agent 03 — Platform Services

**Date:** 2026-11-25
**Repository:** `D:\Aegis`
**Baseline:** `2bd2513`
**Owner scope:** `crates/aegis-service/src/**`, `crates/aegis-service/tests/**`,
`docs/implementation/BATCH5_AGENT03_SERVICES.md`
**Environment:** Windows 10/11 host, PowerShell 5.1, Rust workspace build.
**Deployment:** none. No service was installed, started, stopped, or removed.

## 1. Original state

`aegis-service` implemented a process-based lifecycle only:

- `ServiceManager` started a detached `aegis-service-host` child, wrote
  `<root>/run/service.json`, probed local IPC, and performed bounded
  graceful/forced shutdown.
- Owner proof was required for `start`/`stop`.
- There was **no** OS service definition model, no systemd unit generator, no
  Windows SCM command specification, and no capability probe.
- The CLI exposed only `aegis service start|stop|status`, mapped directly to
  `ServiceManager`.
- No boot-time auto-start existed (by design).

## 2. Changes (additive only)

New module `src/registration.rs`, re-exported from the crate root:

### Declarative model

- `ServiceRegistration` — platform-neutral definition: target platform, name,
  description, absolute executable, arguments, optional working directory,
  environment pairs, systemd `Type` (`Simple`/`Notify`), restart policy
  (`No`/`OnFailure`/`Always`), and restart delay.
- `ServiceUnit` — rendered systemd artifact with `suggested_path` and
  `render()` / `Display`.
- `ServiceCommandSpec` — one `sc.exe` command as unexecuted argv data with
  `arguments()` and display-only `render()`.
- `WindowsServiceSpecs` — `create`, `start`, `stop`, `query`, `delete`
  specifications with a combined `render()`.
- `RenderedRegistration` — dispatch result (`Systemd` or `Windows`) with
  `platform()` and `render()`.

### Validation

`ServiceRegistration::validate()` rejects:

- empty, over-long (>64 bytes), or unsafe names; allowed pattern is
  `[A-Za-z0-9][A-Za-z0-9._-]*`;
- empty or control-character descriptions; >256 bytes;
- empty executables; paths that are not absolute **for the declared target**
  (Linux `/...`, Windows drive `C:\...`/`C:/...`, or UNC `\\server\share`);
- `..` parent components and control characters in paths;
- relative working directories;
- control characters in arguments and environment values;
- environment names outside `[A-Za-z_][A-Za-z0-9_]*`;
- unsupported target platforms and platform/render mismatches.

Validation is repeated by `validate()` and by every `render_*`/`render`
method, so public fields cannot bypass checks.

### systemd rendering

- `Type=simple` by default; `Type=notify` + `NotifyAccess=main` when selected.
  The current host does not send `sd_notify`, so `simple` is the default.
- `ExecStart=<absolute exe> <args>` with systemd-safe quoting (`%` → `%%`,
  `$` → `$$`, backslash/quote escaping).
- `WorkingDirectory=` when supplied.
- `Environment="KEY=VALUE"` with escaping.
- `Restart=on-failure` + `RestartSec=5` by default (configurable).
- Hardening: `NoNewPrivileges=true`, `PrivateTmp=true`, `ProtectSystem=full`,
  `ProtectKernelTunables=true`, `ProtectKernelModules=true`,
  `ProtectControlGroups=true`, `RestrictSUIDSGID=true`,
  `LockPersonality=true`, `RestrictRealtime=true`.
  `ProtectHome` and `ReadWritePaths` are intentionally omitted because the
  storage root may live under `/home`; this needs a packaging decision.
- `[Install] WantedBy=multi-user.target`.
- Suggested path `/etc/systemd/system/<name>.service`.

### Windows SCM rendering

`binPath` is built as `"<exe>" <args>` with Windows CRT-compatible quoting,
then represented as data:

```text
sc.exe create <name> binPath= <binPath> start= auto DisplayName= <description>
sc.exe start <name>
sc.exe stop <name>
sc.exe query <name>
sc.exe delete <name>
```

`create` argv tokens use the required doubled `key=` / value form, e.g.
`["create", name, "binPath=", ..., "start=", "auto", ...]`. No command is
executed.

### Capability probe

`platform() -> ServicePlatform` is compile-time/file-system-free and reports:

- `os`, `arch`, `kind` (`Linux`/`Windows`/`Unsupported`);
- `registration_supported` (known systemd/SCM mechanism);
- `generation_supported` (always true in this build);
- `runtime_registration_performed` (**always false**);
- `mechanism` (`systemd` / `windows-scm` / none);
- `notes` stating that installation is not performed.

## 3. Exact files changed

| File | Change |
|---|---|
| `crates/aegis-service/src/lib.rs` | Added `pub mod registration;` and re-exports; module-level doc note. No existing behavior changed. |
| `crates/aegis-service/src/registration.rs` | New: model, validation, systemd/Windows rendering, capability probe, quoting helpers. |
| `crates/aegis-service/tests/registration.rs` | New: 12 tests. |
| `docs/implementation/BATCH5_AGENT03_SERVICES.md` | This report. |

No other crate, workspace manifest, lockfile, `.gitignore`, or legacy
`D:\SofiaDev\SofiaAdaLyra` path was touched. No dependencies were added.

## 4. Verification

Commands run in `D:\Aegis`:

| Command | Result |
|---|---|
| `cargo test -p aegis-service` | **18 passed, 0 failed** (12 new registration + 6 existing service tests; 0 unit/doc tests) |
| `cargo clippy -p aegis-service --all-targets -- -D warnings` | **exit 0** |
| `cargo fmt -p aegis-service -- --check` | **exit 0** |

New test coverage:

- exact systemd unit directives, suggested path, trailing newline;
- systemd quoting of spaces, `%`, and environment values;
- `Type=notify` + `NotifyAccess=main`;
- restart policy rendering including omission of `RestartSec` for `no`;
- exact Windows `create` argv tokens, `start`/`stop`/`query`/`delete`, and
  rendered command with correct quote escaping;
- deterministic (repeat-call) systemd and Windows output;
- platform dispatch and `Display`;
- invalid/empty/long names;
- relative executables per target, Linux-vs-Windows path confusion, UNC paths;
- parent components, relative working directory, control characters,
  invalid environment names/values, invalid descriptions;
- unsupported platform and render/platform mismatch;
- capability probe invariants (`runtime_registration_performed == false`).

## 5. Limitations / remaining gaps

- **No OS registration runtime.** This build never calls `systemctl`,
  `sc.exe`, writes unit files, edits the registry, or changes boot state.
- **No reboot recovery.** No `systemctl enable`, no Windows auto-start
  installation, and no post-reboot health verification.
- **No Linux build/runtime verification.** All work was built and tested on
  Windows. The Linux systemd output was exercised only as text; it was **not**
  validated with `systemd-analyze verify`, installed, started, or rebooted.
- **Linux owner-proof unverified.** `aegis-safe` has a Unix 0600 file fallback
  (`unix-file-dev`), but it was not exercised on Linux in this batch.
- **Windows service hardening incomplete.** `create` does not set a service
  account, recovery actions (`sc.exe failure`), or event-log configuration;
  the default LocalSystem account applies if installed later.
- **No packaging/CLI wiring.** The CLI is outside this agent's ownership and
  was not changed; no command exposes the generated definitions yet.
- **Hardening is fixed.** No per-deployment `ReadWritePaths`, user/group,
  `ProtectHome`, or log redirection options.

## 6. Next actionable steps

1. Add an owner-authorized packaging command (separate batch) that can
   **optionally** write the unit / run `sc.exe` with an explicit
   `--install` flag; keep generation as the default and dry-run safe.
2. Decide storage-root-aware hardening: `ReadWritePaths=<root>` and
   `ProtectHome` policy; add user/group fields for systemd.
3. Add Windows recovery/account configuration (`sc.exe failure`,
   `obj=`), and a `native` service host mode rather than relying on the
   console host.
4. Debian 13 verification (required before any Linux claim):
   - `cargo build --release -p aegis-service --bins` on Debian 13;
   - write the generated unit to `/etc/systemd/system/aegis-platform.service`;
   - `systemd-analyze verify` the unit;
   - `systemctl daemon-reload && systemctl enable --now aegis-platform`;
   - confirm `aegis service status` reachability via IPC;
   - `kill -9` the host and confirm `Restart=on-failure` recovery;
   - reboot and confirm automatic recovery;
   - `systemctl disable --now` and remove the unit;
   - exercise the Unix owner-proof backend during the same run.
5. Add a Linux CI job that renders a unit and runs `systemd-analyze verify`
   (container or VM), plus a Windows test that inspects `sc.exe` argv without
   executing it.
