# Batch 7 — Agent 03 — Debian Runtime & Services

**Date:** 2026-11-26
**Repository:** `D:\Aegis`
**HEAD:** `2bd2513` (all pre-existing uncommitted work preserved)
**Owner scope:** `crates/aegis-service/src/**`, `crates/aegis-service/tests/**`,
`docs/implementation/BATCH7_AGENT03_SERVICES.md`
**Environment used:** Windows host, PowerShell 5.1, Rust 1.88+.

**Linux runtime: NONE.** No Debian (or any Linux) host was booted, built, or
exercised. No service was installed, registered, enabled, started, stopped,
restarted, or removed. No `systemctl`, `systemd-analyze`, `sc.exe`, service
registration, registry edit, or OS configuration was run. Everything below is
data, pure validation, or an operator runbook that was **not executed**.

---

## 1. Status

| Requested item | Status |
|---|---|
| Dry-run `ServiceInstallExecutor` / `InstallExecutionPlan` | Implemented as inert data + pure validation + tests. Never executes. |
| `RuntimeSupport` read-only probe | Implemented via filesystem metadata only. Windows reports Linux runtime **UNSUPPORTED / untested**. |
| Logging / failure-recovery specification | Implemented as bounded retention config + documented journal integration data + validation + tests. Writes no logs. |
| Debian 13 runbook | Documented in section 6. **Not executed.** |
| Backward compatibility | Preserved. Existing `ServiceManager`, `ServiceRegistration`, rendering, `InstallPlan`, health, and supervision APIs are unchanged; all additions are new modules, types, and re-exports. |

No item is claimed as a working OS runtime. The new code cannot start a
process or mutate the host by construction (see section 2).

---

## 2. Dry-run installation execution (`src/execution.rs`)

New module. It consumes a validated [`ServiceRegistration`] and produces an
ordered, inert plan. No function in this module calls `Command`, `fs::write`,
`systemctl`, or `sc.exe`.

### Types

- `InstallExecutionPlan { platform, service_name, unit_path, steps }`
  - `InstallExecutionPlan::for_registration(&ServiceRegistration)`
  - `validate()` — structural validation
  - `validate_against(&ServiceRegistration)` — platform / service name / unit
    path match
  - `rollback_steps()` — steps with a rollback, in reverse execution order
  - `render()` / `Display` — evidence text, never executed
- `ExecutionStep { order, action, description, command, preconditions,
  verification, rollback, idempotent }`
- `ExecutionPrecondition { code, description }`
- `ExecutionVerification { command, expectation }`
- `ExecutionRollback { command, description, idempotent }`
- `ServiceInstallExecutor` — dry-run wrapper:
  - `new(plan)`, `for_registration(&registration)`, `plan()`, `is_dry_run()`
  - `dry_run() -> DryRunReport` (the only "run"; `executed` is always `false`)
  - `into_plan()`
- `DryRunReport { executed, platform, service_name, operations }` with
  `is_inert()` and `render()`.
- `PlannedOperation { order, action, command, verification, rollback }`.
- `ExecutionError` — empty plan, non-contiguous order, missing command/empty
  program, missing precondition, missing verification, missing rollback on a
  mutating step, and registration mismatches.

### Exact commands captured as argv data

systemd (4 steps):

1. `write-unit-file` — file write to
   `/etc/systemd/system/<name>.service` (no command). Rollback:
   `rm -f <unit-path>`.
2. `reload-daemon` — `systemctl daemon-reload`.
3. `enable` — `systemctl enable --now <name>`. Rollback:
   `systemctl disable --now <name>`.
4. `query` — `systemctl status <name> --no-pager`; verified with
   `systemctl show <name> --property=ActiveState --no-pager`.

Windows SCM (4 steps):

1. `query` — `sc.exe query <name>` (read-only precondition check).
2. `create-service` — the exact `sc.exe create ...` argv from
   `ServiceRegistration::render_windows()`. Rollback: `sc.exe delete <name>`.
3. `start` — `sc.exe start <name>`. Rollback: `sc.exe stop <name>`.
4. `query` — `sc.exe query <name>` verification.

Each step also carries ordered preconditions (`owner-authorization`,
`platform-*`, `binary-present`, `unit-*`, `service-absent`, etc.), a
verification expectation (plus read-only verification command where relevant),
and an idempotency flag.

### Validation rules (`InstallExecutionPlan::validate`)

- plan has at least one step;
- step order starts at 1 and is contiguous;
- every action except `write-unit-file` has a command; programs are non-empty;
- every step has at least one non-empty precondition;
- every step has a non-empty verification expectation;
- every mutating action (`write-unit-file`, `enable`, `start`,
  `create-service`) has a rollback.

`ServiceInstallExecutor::new` rejects invalid plans, so a malformed plan cannot
be previewed.

---

## 3. Read-only runtime probe (`src/runtime.rs`)

New module. Detection uses `Path::is_dir` / `Path::is_file` only — no process
spawn, no `systemctl`, no socket, no host change.

### Types

- `OsFamily` — `Linux`, `Windows`, `Other` (`current()` via `cfg`).
- `RuntimeVerdict` — `Supported`, `Unsupported`, `Untested`.
- `RuntimeInputs { os_family, systemd_unit_dir_present,
  systemd_runtime_dir_present, systemctl_present }` with injected-input
  builders and `current()`.
- `RuntimeSupport { os_family, systemd_capable, systemd_present,
  linux_runtime, read_only_probe, process_spawned, notes }`.
- `runtime_support()` (current host) and `runtime_support_with(&inputs)` (pure).

Probe paths: `/etc/systemd/system`, `/run/systemd/system`, and
`/usr/bin/systemctl` or `/bin/systemctl`.

### Verdicts

| Inputs | `systemd_present` | `linux_runtime` |
|---|---|---|
| Windows | false | `Unsupported` (Linux runtime unsupported and untested) |
| Other OS family | false | `Unsupported` |
| Linux, all markers present | true | `Untested` (presence is not verification) |
| Linux, any marker missing | false | `Unsupported` |

`read_only_probe` is always `true`; `process_spawned` is always `false`.
`linux_runtime_supported()` is true only for `Supported`, which this probe never
reports. In this Windows environment the probe reports Linux runtime
**UNSUPPORTED / untested**.

---

## 4. Logging and failure-recovery specification (`src/logging.rs`)

New module. A specification expressed as data plus pure validation. Nothing
writes logs, calls `journalctl`, changes journald, or performs recovery.

### Bounded retention

`LogRetentionConfig { max_files, max_bytes_per_file, max_total_bytes,
max_age_secs, rotate_on_restart, compress_rotated }`

Defaults: 5 files, 8 MiB/file, 40 MiB total, 7 days, rotate on restart,
compress rotated.

Hard bounds (`validate()` rejects outside them):

| Field | Bound |
|---|---|
| `max_files` | `1..=MAX_LOG_FILES` (100) |
| `max_bytes_per_file` | `1..=MAX_LOG_BYTES_PER_FILE` (1 GiB) |
| `max_total_bytes` | `1..=MAX_LOG_TOTAL_BYTES` (4 GiB), and `>= max_bytes_per_file` |
| `max_age_secs` | `1..=MAX_LOG_AGE_SECS` (90 days) |

`effective_total_bytes()` = `min(max_total_bytes, max_files *
max_bytes_per_file)`.

### Journal integration approach (documented)

`JournalIntegration { unit_name, identifier, transport, retention_owner,
failure_recovery }`:

- transport: plain lines on stdout/stderr captured by systemd-journald
  (`JournalTransport::StdoutStderr`), with a native-journal option recorded for
  future work;
- retention owner: `Both` — journald owns journal rotation, Aegis owns bounded
  local files;
- failure-recovery notes: inspect with `journalctl -u <unit>`, check the
  current boot, confirm `run/service.json` ownership, never raise Aegis
  retention above the hard bounds, and stop a crash-loop before restarting.

`LoggingSpecification { retention, journal }` combines both and validates.
`render()` is display text only.

---

## 5. Exact files changed

| File | Change |
|---|---|
| `crates/aegis-service/src/execution.rs` | New: `InstallExecutionPlan`, `ServiceInstallExecutor`, steps, preconditions, verification, rollback, `DryRunReport`, validation. |
| `crates/aegis-service/src/runtime.rs` | New: `OsFamily`, `RuntimeInputs`, `RuntimeVerdict`, `RuntimeSupport`, read-only path probe. |
| `crates/aegis-service/src/logging.rs` | New: `LogRetentionConfig`, `JournalIntegration`, `LoggingSpecification`, bounds, validation. |
| `crates/aegis-service/src/lib.rs` | Added `pub mod execution/logging/runtime;` and additive re-exports. No existing item changed. |
| `crates/aegis-service/tests/execution.rs` | New: 7 tests (ordering, commands, rollback order, validation, mismatch, inert executor, determinism/render). |
| `crates/aegis-service/tests/runtime_support.rs` | New: 5 tests (Windows unsupported, detected-but-untested, missing markers, other OS, current probe read-only/no-spawn). |
| `crates/aegis-service/tests/logging.rs` | New: 6 tests (defaults, explicit config, every bound, journal render, invalid specs, combined spec). |
| `docs/implementation/BATCH7_AGENT03_SERVICES.md` | This report and runbook. |

No other crate, workspace `Cargo.toml`, `Cargo.lock`, `.gitignore`, CLI,
completion doc, or `D:\SofiaDev\SofiaAdaLyra` path was touched. No dependency
was added. No commit, deploy, reset, rebase, clean, or destructive git command
was run.

---

## 6. Verification

Run in `D:\Aegis`:

| Command | Result |
|---|---|
| `cargo test -p aegis-service` | **61 passed, 0 failed** (7 execution + 11 health + 6 logging + 15 registration + 5 runtime_support + 6 service + 11 supervision; 0 unit/doc) |
| `cargo clippy -p aegis-service --all-targets -- -D warnings` | **exit 0** |
| `cargo fmt -p aegis-service -- --check` | **exit 0** |

Coverage added this batch:

- systemd and Windows execution-plan ordering, exact argv, precondition
  presence, verification expectations, rollback commands, reverse rollback
  order, and determinism;
- rejection of empty, non-contiguous, missing-command, empty-program,
  missing-precondition, missing-verification, and missing-rollback plans;
- registration/platform/service-name/unit-path mismatch detection;
- executor is inert (`executed == false`), validates its plan, and produces a
  rendered preview;
- runtime probe with Windows, Linux-with-systemd (untested), Linux-missing-
  markers, and other OS inputs, plus current-host read-only/no-spawn checks;
- retention defaults, explicit configs, `effective_total_bytes`, every upper
  and lower bound, and the total-vs-file relationship;
- journal integration render/validation and combined logging specification.

---

## 7. Debian 13 deployment / upgrade / rollback runbook

> **Not executed.** These are intended operator commands for a disposable
> Debian 13 host, validated by a human before production. The Aegis code only
> renders data and validates plans. Installation, verification, start/stop,
> kill recovery, reboot, upgrade, and rollback are explicit operator actions.
> On the Windows build host used for this batch, `RuntimeSupport` reports the
> Linux runtime as unsupported/untested.

### 7.1 Paths and identities

| Purpose | Path |
|---|---|
| Service host binary | `/opt/aegis/bin/aegis-service-host` |
| CLI binary | `/opt/aegis/bin/aegis` |
| Storage/platform root | `/var/lib/aegis` |
| Run/state directory | `/var/lib/aegis/run` |
| Recorded service state | `/var/lib/aegis/run/service.json` |
| systemd unit | `/etc/systemd/system/aegis-platform.service` |
| Backups | `/var/backups/aegis/` |
| Source checkout | `/srv/aegis-src` |
| Service account | `aegis` (system, nologin) |
| IPC endpoint name | `aegis-platform` (local IPC, not a TCP port) |

### 7.2 Build

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup toolchain install 1.88.0
cd /srv/aegis-src
cargo build --release -p aegis-service -p aegis-cli
```

Evidence: full build log, `target/release/aegis-service-host --version`,
`sha256sum` of both binaries.

### 7.3 Install binaries and account

```bash
sudo install -d -m 0755 /opt/aegis/bin
sudo install -m 0755 target/release/aegis-service-host /opt/aegis/bin/aegis-service-host
sudo install -m 0755 target/release/aegis             /opt/aegis/bin/aegis

sudo useradd --system --home /var/lib/aegis --shell /usr/sbin/nologin aegis || true
sudo install -d -o aegis -g aegis -m 0750 /var/lib/aegis
sudo install -d -o aegis -g aegis -m 0750 /var/backups/aegis
```

### 7.4 Initialise the platform (writes the owner token)

```bash
sudo -u aegis /opt/aegis/bin/aegis init --root /var/lib/aegis
```

Do not pass `--print-owner-token` in production (development aid only). Record
only that initialization succeeded; do not capture the token.

### 7.5 Install the unit

The unit below is the data produced by
`ServiceRegistration::for_platform(Linux, "aegis-platform",
"/opt/aegis/bin/aegis-service-host")` with `--root /var/lib/aegis --socket
aegis-platform`, `WorkingDirectory=/var/lib/aegis`, and `AEGIS_LOG=info`.
Add `User=aegis` / `Group=aegis` only when the platform root is owned by that
account.

```bash
sudo tee /etc/systemd/system/aegis-platform.service >/dev/null <<'EOF'
[Unit]
Description=Aegis Platform background service

[Service]
Type=simple
ExecStart=/opt/aegis/bin/aegis-service-host --root /var/lib/aegis --socket aegis-platform
WorkingDirectory=/var/lib/aegis
Environment="AEGIS_LOG=info"
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=full
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
LockPersonality=true
RestrictRealtime=true

[Install]
WantedBy=multi-user.target
EOF
```

Dry-run inspection before installing (data only): use the new
`InstallExecutionPlan` / `ServiceInstallExecutor` from a future owner-authorized
packaging command; this batch does not expose it through the CLI. The rendered
plan must match the unit above and list `write-unit-file`, `reload-daemon`,
`enable`, `query` in that order with the rollbacks described in section 2.

### 7.6 Verify the unit

```bash
sudo systemd-analyze verify /etc/systemd/system/aegis-platform.service
```

Expected: no output and exit 0. Capture the command, exit status, and stderr.

### 7.7 Start / stop / restart

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now aegis-platform
sudo systemctl status aegis-platform --no-pager
sudo journalctl -u aegis-platform -n 100 --no-pager
sudo -u aegis /opt/aegis/bin/aegis service status --root /var/lib/aegis

sudo systemctl restart aegis-platform
sudo systemctl status aegis-platform --no-pager

sudo systemctl stop aegis-platform
sudo systemctl status aegis-platform --no-pager
sudo systemctl start aegis-platform
sudo systemctl status aegis-platform --no-pager
```

Expected: `active (running)` after start/restart; a live PID and reachable
socket from `aegis service status`; `run/service.json` exists; clean `inactive`
after stop; active again after start. Capture `systemctl status`,
`journalctl -u aegis-platform -n 100 --no-pager`, and `service status` output at
each point.

### 7.8 `kill -9` recovery

```bash
PID="$(python3 -c 'import json;print(json.load(open("/var/lib/aegis/run/service.json"))["pid"])')"
sudo kill -9 "$PID"
sleep 10
sudo systemctl status aegis-platform --no-pager
sudo journalctl -u aegis-platform -n 100 --no-pager
sudo -u aegis /opt/aegis/bin/aegis service status --root /var/lib/aegis
```

Expected with `Restart=on-failure` + `RestartSec=5`: systemd starts a new host
process; the unit returns to `active (running)` with a new PID. Record the old
and new PIDs and the journal lines showing the restart. If the unit crashes
repeatedly, stop it and inspect the recorded health reasons and restart policy
inputs instead of restarting blindly.

### 7.9 Reboot recovery

```bash
sudo systemctl reboot
```

After the host returns:

```bash
systemctl is-enabled aegis-platform
systemctl is-active aegis-platform
sudo -u aegis /opt/aegis/bin/aegis service status --root /var/lib/aegis
sudo journalctl -b -u aegis-platform --no-pager
```

Expected: `enabled`, `active`, live PID/socket. If not, check that
`/var/lib/aegis/run/service.json` ownership matches the unit's `User=`/`Group=`
and inspect the current-boot journal.

### 7.10 Upgrade

```bash
STAMP="$(date +%Y%m%dT%H%M%S)"
sudo systemctl stop aegis-platform

sudo cp -p /opt/aegis/bin/aegis-service-host /opt/aegis/bin/aegis-service-host.previous
sudo cp -p /opt/aegis/bin/aegis             /opt/aegis/bin/aegis.previous

sudo -u aegis /opt/aegis/bin/aegis backup --root /var/lib/aegis \
  --to "/var/backups/aegis/pre-upgrade-$STAMP"
sudo -u aegis /opt/aegis/bin/aegis upgrade --root /var/lib/aegis \
  --backup-to "/var/backups/aegis/pre-upgrade-$STAMP"

# Build the new binaries first, then install atomically.
sudo install -m 0755 target/release/aegis-service-host /opt/aegis/bin/aegis-service-host.new
sudo install -m 0755 target/release/aegis             /opt/aegis/bin/aegis.new
sudo mv /opt/aegis/bin/aegis-service-host.new /opt/aegis/bin/aegis-service-host
sudo mv /opt/aegis/bin/aegis.new             /opt/aegis/bin/aegis

sudo systemd-analyze verify /etc/systemd/system/aegis-platform.service
sudo systemctl daemon-reload
sudo systemctl start aegis-platform
sudo systemctl status aegis-platform --no-pager
sudo -u aegis /opt/aegis/bin/aegis service status --root /var/lib/aegis
```

Evidence: backup path, `sha256sum` of old/new binaries, unit verify result,
pre/post `service status`, journal excerpt.

### 7.11 Rollback

```bash
STAMP="<timestamp-of-pre-upgrade-backup>"
sudo systemctl stop aegis-platform

sudo install -m 0755 /opt/aegis/bin/aegis-service-host.previous \
  /opt/aegis/bin/aegis-service-host
sudo install -m 0755 /opt/aegis/bin/aegis.previous /opt/aegis/bin/aegis

sudo -u aegis /opt/aegis/bin/aegis verify-backup \
  --path "/var/backups/aegis/pre-upgrade-$STAMP"
sudo -u aegis /opt/aegis/bin/aegis restore \
  --from "/var/backups/aegis/pre-upgrade-$STAMP" \
  --root /var/lib/aegis \
  --backup-existing-to "/var/backups/aegis/rollback-$STAMP"

sudo systemd-analyze verify /etc/systemd/system/aegis-platform.service
sudo systemctl daemon-reload
sudo systemctl start aegis-platform
sudo systemctl status aegis-platform --no-pager
sudo -u aegis /opt/aegis/bin/aegis service status --root /var/lib/aegis
```

If rollback is needed before data was upgraded, restore only the binaries and
skip the data restore. Never delete `/var/lib/aegis` as part of rollback.

### 7.12 Evidence to capture for every run

- Host identity: Debian version (`/etc/os-release`), kernel (`uname -a`), Rust
  version (`rustc --version`).
- Build log and binary hashes.
- `systemd-analyze verify` command, exit status, stdout/stderr.
- `systemctl status`, `systemctl show <unit> --property=ActiveState,LoadState`,
  `journalctl -u aegis-platform -n 100 --no-pager` at each state change.
- `aegis service status` output and `run/service.json` metadata (not the token).
- `kill -9` scenario: old/new PIDs and restart journal lines.
- Reboot scenario: `systemctl is-enabled`, `is-active`, journal for the boot.
- Upgrade/rollback scenario: backup paths, binary hashes, restore output.
- The new read-only `RuntimeSupport` report (OS family, `systemd_present`,
  `linux_runtime`, `process_spawned=false`).

---

## 8. Not executed in this environment

- No Debian 13 (or any Linux) build, boot, or runtime.
- No `systemd-analyze verify`, `systemctl`, unit installation, enable, start,
  stop, restart, or journal inspection against a real host.
- No `kill -9`, reboot, upgrade, or rollback.
- No `sc.exe`, Windows service registration, registry edit, or boot change.
- No systemd, journald, or OS configuration changed.
- The systemd unit, execution plans, and journal approach were exercised only
  as rendered/validated data in tests.
- No `D:\SofiaDev\SofiaAdaLyra` path was read or modified.

---

## 9. Next actionable steps

1. Run the section 7 runbook on a disposable Debian 13 VM and attach the
   evidence from section 7.12 to a future acceptance record.
2. Wire `ServiceInstallExecutor` into an owner-authorized packaging command
   that defaults to dry-run output (CLI is outside this agent's scope).
3. Connect `evaluate_health` and `plan_restart` to a future supervisor loop;
   they remain pure and need only a runtime adapter.
4. Decide whether to expose `ServiceRegistration::execution_plan()` as a
   convenience method (currently available via
   `InstallExecutionPlan::for_registration`).
5. Add a Linux CI job that renders a unit, validates the execution plan, and
   runs `systemd-analyze verify`; keep the current tests host-independent.
