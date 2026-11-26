# Batch 6 — Agent 03 — Debian 13 Runtime & Services

**Date:** 2026-11-25
**Repository:** `D:\Aegis`
**HEAD:** `2bd2513`
**Owner scope:** `crates/aegis-service/src/**`, `crates/aegis-service/tests/**`,
`docs/implementation/BATCH6_AGENT03_SERVICES.md`
**Environment used for this batch:** Windows host, PowerShell 5.1, Rust 1.88+.
**Linux runtime:** **NONE.** No Debian or other Linux system was booted, built,
or exercised. No service was installed, enabled, started, stopped, or removed.
No `systemctl`, `sc.exe`, service registration, or system configuration was run.

---

## 1. Summary of changes

All changes are additive to `aegis-service`. Existing `ServiceManager`,
`ServiceRegistration`, rendering, and every prior public API are unchanged.

| Area | New source | Purpose |
|---|---|---|
| Supervision | `src/supervision.rs` | Pure restart policy with delay, attempt budget, and backoff. |
| Health | `src/health.rs` | Pure desired-vs-actual health summarisation with reasons. |
| Install plans | `src/registration.rs` (extended) | Ordered install steps with idempotency notes as data. |

`src/lib.rs` adds `pub mod health; pub mod supervision;` and re-exports the new
types. To preserve backward compatibility, the supervision struct is exposed as
`aegis_service::supervision::RestartPolicy` and at the crate root as
`SupervisionPolicy`; the existing rendering enum `aegis_service::RestartPolicy`
is untouched.

---

## 2. Supervision policy model

`src/supervision.rs` — no clock reads, no process handles, no OS calls.

### Types

- `RestartCondition` — `Never`, `OnFailure`, `Always`.
- `Backoff` — `Fixed`, `Linear { step }`, `Exponential { multiplier }`.
- `RestartPolicy` — `condition`, `delay`, `max_attempts` (`0` = unlimited),
  `backoff`, `max_delay`, `reset_after`.
- `RestartDecision` — `Restart { attempt, delay }`, `Stop { reason,
  attempts_reset }`, `Exhausted { attempts }`.
- `StopReason` — `PolicyNever`, `SuccessfulExit`.
- `SupervisionError` — validation failures.

### Validation

`RestartPolicy::validate()` (and every planning call) rejects:

- zero `delay` or zero `max_delay`;
- `delay > max_delay`;
- `max_attempts > MAX_RESTART_ATTEMPTS` (100);
- `Linear` backoff with a zero step;
- `Exponential` backoff with `multiplier < 2`;
- zero `reset_after`.

### Decision function

```rust
RestartPolicy::plan_restart(exit_status: Option<i32>, attempt: u32, now: Duration)
    -> Result<RestartDecision, SupervisionError>

// free-function form
supervision::plan_restart(&policy, exit_status, attempt, now)
```

- `exit_status == Some(0)` is success; `None` is signal termination.
- `now` is the elapsed runtime of the attempt that just exited (pure input, not
  a wall clock).
- A successful exit, or `now >= reset_after`, resets the attempt counter.
- `OnFailure` stops with `SuccessfulExit` on success.
- `Always` restarts even on success.
- `Never` stops with `PolicyNever`.
- When `max_attempts != 0` and the effective attempt is at the budget, returns
  `Exhausted`.
- Delay is computed from the backoff strategy and clamped to `max_delay`.

`SupervisionPolicy::delay_for_attempt(attempt)` is also public for inspection.

---

## 3. Health and diagnostics

`src/health.rs` — pure input folding; it does not enumerate services, read
Windows SCM, or touch IPC.

### Inputs

`HealthInputs` (builder): `desired_running`, `recorded_state_present`, `pid`,
`pid_alive`, `heartbeat_age`, `heartbeat_timeout` (default
`DEFAULT_HEARTBEAT_TIMEOUT = 30s`).

### Output

`ServiceHealth`: `level` (`Ok`/`Warn`/`Error`), `desired_running`,
`actual_running`, `recorded_state_present`, `pid`, `heartbeat_age`, and
`reasons: Vec<HealthReason>` where each reason has `level`, a stable `code`, and
a message.

### Rules

Desired running:

| Observation | Level | Code |
|---|---|---|
| no recorded state | Error | `state-missing` |
| no pid | Error | `pid-missing` |
| pid not alive | Error | `pid-dead` |
| no heartbeat | Warn | `heartbeat-missing` |
| heartbeat older than timeout (or timeout is zero) | Error | `heartbeat-stale` |
| all good | Ok | `healthy` |

Desired stopped:

| Observation | Level | Code |
|---|---|---|
| state present, process alive | Warn | `unexpected-running` |
| state present, process dead | Warn | `stale-state` |
| no state | Ok | `stopped` |

The overall level is the maximum severity among reported reasons.

---

## 4. Install plans (data only)

`ServiceRegistration::install_plan()` returns an `InstallPlan`:

- `platform`, `service_name`, optional `unit_path`, ordered `steps`;
- each `InstallStep` has `order`, `action`, `description`, an optional
  `InstallCommand`, `idempotent`, and a `note`;
- `InstallPlan::render()` / `Display` produce display text only.

### systemd plan (4 steps)

1. `write-unit-file` — write to `/etc/systemd/system/<name>.service`; idempotent
   (deterministic bytes).
2. `reload-daemon` — `systemctl daemon-reload`; idempotent.
3. `enable` — `systemctl enable --now <name>`; idempotent.
4. `query` — `systemctl status <name> --no-pager`; idempotent, read-only.

### Windows SCM plan (4 steps)

1. `query` — `sc.exe query <name>`; idempotent, read-only.
2. `create-service` — `sc.exe create ...`; **not** idempotent (fails if it
   exists; skip when step 1 succeeds).
3. `start` — `sc.exe start <name>`; **not** idempotent (fails if running).
4. `query` — verify; idempotent, read-only.

No step is executed. Commands are argv data; `render()` is display text and is
not guaranteed to round-trip through a shell.

---

## 5. Exact files changed

| File | Change |
|---|---|
| `crates/aegis-service/src/supervision.rs` | New: restart/supervision policy, backoff, plan decision, validation. |
| `crates/aegis-service/src/health.rs` | New: `HealthLevel`, `HealthInputs`, `HealthReason`, `ServiceHealth`, `evaluate_health`. |
| `crates/aegis-service/src/registration.rs` | Added `InstallAction`, `InstallCommand`, `InstallStep`, `InstallPlan`, `ServiceRegistration::install_plan()`. |
| `crates/aegis-service/src/lib.rs` | Added module declarations and re-exports; no existing API changed. |
| `crates/aegis-service/tests/supervision.rs` | New: 11 tests. |
| `crates/aegis-service/tests/health.rs` | New: 11 tests. |
| `crates/aegis-service/tests/registration.rs` | Added 3 install-plan tests (12 existing preserved). |
| `docs/implementation/BATCH6_AGENT03_SERVICES.md` | This runbook. |

No file outside the owner scope was edited. No dependency, workspace manifest,
lockfile, `.gitignore`, CLI, completion doc, or `D:\SofiaDev\SofiaAdaLyra` path
was touched. No commit, deploy, reset, rebase, clean, or destructive git command
was run.

---

## 6. Verification

Run in `D:\Aegis`:

| Command | Result |
|---|---|
| `cargo test -p aegis-service` | **43 passed, 0 failed** (11 health + 15 registration + 6 service + 11 supervision; 0 unit/doc) |
| `cargo clippy -p aegis-service --all-targets -- -D warnings` | **exit 0** |
| `cargo fmt -p aegis-service -- --check` | **exit 0** |

Test coverage added:

- fixed, linear, and exponential backoff values and `max_delay` clamping;
- `OnFailure` restart vs successful-exit stop; `Always` restart on success;
  `Never` stop;
- max-attempt exhaustion boundary;
- attempt reset on successful exit and on `now >= reset_after`;
- free-function/method equivalence;
- every invalid-policy variant and that invalid policies block planning;
- health OK/Warn/Error for missing state, missing/dead pid, missing/stale
  heartbeat, unexpected running, stale stopped state, zero timeout, and
  worst-reason selection;
- systemd and Windows install plan order, actions, idempotency flags, commands,
  unit path, rendering, and determinism.

---

## 7. Debian 13 deployment / upgrade / rollback runbook

> The commands below are the intended operator procedure. They were **not
> executed** in this environment (Windows only). Validate on a disposable
> Debian 13 host before production. The generated unit is data; installation is
> an explicit operator action.

### 7.1 Paths

| Purpose | Path |
|---|---|
| Service host binary | `/opt/aegis/bin/aegis-service-host` |
| CLI binary | `/opt/aegis/bin/aegis` |
| Storage/platform root | `/var/lib/aegis` |
| Run/state directory | `/var/lib/aegis/run` |
| Recorded service state | `/var/lib/aegis/run/service.json` |
| Owner token store | managed by `aegis-safe` under `/var/lib/aegis` |
| systemd unit | `/etc/systemd/system/aegis-platform.service` |
| Backups | `/var/backups/aegis/` |
| Source checkout | `/srv/aegis-src` |
| Service account | `aegis` (system, nologin) |

The local IPC socket name is `aegis-platform` (interprocess endpoint, not a TCP
port).

### 7.2 Build on Debian 13

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup toolchain install 1.88.0
cd /srv/aegis-src
cargo build --release -p aegis-service -p aegis-cli
```

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

Do **not** pass `--print-owner-token` in production; it is a development aid.

### 7.5 Service unit example

Write exactly this file, then optionally add `User=aegis` / `Group=aegis` under
`[Service]` when the platform was initialised as that account:

```ini
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
```

This matches the data produced by
`ServiceRegistration::for_platform(Linux, "aegis-platform",
"/opt/aegis/bin/aegis-service-host")` with `--root /var/lib/aegis --socket
aegis-platform`, `WorkingDirectory=/var/lib/aegis`, and `AEGIS_LOG=info`.

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

### 7.6 Verify and enable

```bash
sudo systemd-analyze verify /etc/systemd/system/aegis-platform.service
sudo systemctl daemon-reload
sudo systemctl enable --now aegis-platform
sudo systemctl status aegis-platform --no-pager
sudo journalctl -u aegis-platform -n 100 --no-pager
sudo -u aegis /opt/aegis/bin/aegis service status --root /var/lib/aegis
```

Expected: unit is `active (running)`; `service status` reports a live pid and a
reachable socket; `run/service.json` exists.

### 7.7 Restart / supervision checks

```bash
sudo systemctl restart aegis-platform
sudo kill -9 "$(python3 -c 'import json;print(json.load(open("/var/lib/aegis/run/service.json"))["pid"])')"
sleep 10
sudo systemctl status aegis-platform --no-pager
```

Expected with `Restart=on-failure` + `RestartSec=5`: systemd starts a new host
process and the service returns to `active (running)`.

### 7.8 Reboot recovery

```bash
sudo systemctl reboot
```

After the host returns:

```bash
systemctl is-enabled aegis-platform
systemctl is-active aegis-platform
sudo -u aegis /opt/aegis/bin/aegis service status --root /var/lib/aegis
```

Expected: `enabled`, `active`, and a live pid/socket. If not, inspect
`journalctl -b -u aegis-platform --no-pager` and confirm
`/var/lib/aegis/run/service.json` ownership matches the unit's `User=`.

### 7.9 Upgrade

```bash
STAMP="$(date +%Y%m%dT%H%M%S)"
sudo systemctl stop aegis-platform

sudo -u aegis /opt/aegis/bin/aegis verify-backup --path /var/backups/aegis/latest || true
sudo -u aegis /opt/aegis/bin/aegis backup --root /var/lib/aegis \
  --to "/var/backups/aegis/pre-upgrade-$STAMP"
sudo -u aegis /opt/aegis/bin/aegis upgrade --root /var/lib/aegis \
  --backup-to "/var/backups/aegis/pre-upgrade-$STAMP"

# Build the new binaries in /srv/aegis-src first, then install atomically.
sudo install -m 0755 target/release/aegis-service-host /opt/aegis/bin/aegis-service-host.new
sudo install -m 0755 target/release/aegis             /opt/aegis/bin/aegis.new
sudo mv /opt/aegis/bin/aegis-service-host.new /opt/aegis/bin/aegis-service-host
sudo mv /opt/aegis/bin/aegis.new             /opt/aegis/bin/aegis

sudo systemctl daemon-reload
sudo systemctl start aegis-platform
sudo systemctl status aegis-platform --no-pager
sudo -u aegis /opt/aegis/bin/aegis service status --root /var/lib/aegis
```

### 7.10 Rollback

```bash
STAMP="<timestamp-of-pre-upgrade-backup>"
sudo systemctl stop aegis-platform

# Restore the previous binaries (keep copies before every upgrade).
sudo install -m 0755 /opt/aegis/bin/aegis-service-host.previous \
  /opt/aegis/bin/aegis-service-host
sudo install -m 0755 /opt/aegis/bin/aegis.previous /opt/aegis/bin/aegis

# Verify and restore data.
sudo -u aegis /opt/aegis/bin/aegis verify-backup \
  --path "/var/backups/aegis/pre-upgrade-$STAMP"
sudo -u aegis /opt/aegis/bin/aegis restore \
  --from "/var/backups/aegis/pre-upgrade-$STAMP" \
  --root /var/lib/aegis \
  --backup-existing-to "/var/backups/aegis/rollback-$STAMP"

sudo systemctl daemon-reload
sudo systemctl start aegis-platform
sudo systemctl status aegis-platform --no-pager
sudo -u aegis /opt/aegis/bin/aegis service status --root /var/lib/aegis
```

Before each upgrade, copy the current binaries so rollback has a source:

```bash
sudo cp -p /opt/aegis/bin/aegis-service-host /opt/aegis/bin/aegis-service-host.previous
sudo cp -p /opt/aegis/bin/aegis             /opt/aegis/bin/aegis.previous
```

### 7.11 Disable and remove (manual, explicit)

```bash
sudo systemctl disable --now aegis-platform
sudo rm -f /etc/systemd/system/aegis-platform.service
sudo systemctl daemon-reload
sudo systemctl reset-failed
```

Data under `/var/lib/aegis` is intentionally left in place unless the operator
explicitly removes it.

---

## 8. Not executed in this environment

- No Debian 13 (or any Linux) build, boot, or runtime.
- No `systemd-analyze verify`, `systemctl`, `systemd-ctl`, or unit installation.
- No `sc.exe`, Windows service registration, registry edit, or boot change.
- No service started/stopped/restarted; no OS configuration changed.
- The Linux systemd unit was exercised only as rendered text in tests.
- The Unix owner-token backend of `aegis-safe` was not exercised on Linux.

## 9. Next actionable steps

1. Run the runbook on a disposable Debian 13 VM and record evidence:
   `systemd-analyze verify`, start/enable, `kill -9` recovery, reboot recovery,
   upgrade, rollback.
2. Wire `install_plan()` into an owner-authorized packaging command in a later
   batch (CLI is outside this agent's scope). Default to dry-run/data output.
3. Feed `plan_restart` and `evaluate_health` into a future supervisor loop;
   they are pure and need a runtime adapter only.
4. Decide storage-root-aware hardening (`ReadWritePaths`, `User=`/`Group=`) in
   the registration model.
5. Add a Linux CI job that renders a unit and runs `systemd-analyze verify`.
