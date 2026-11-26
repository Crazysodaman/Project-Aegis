# Batch 7 — Agent 05: TUI External Status Snapshots

Baseline: `2bd2513`. All pre-existing uncommitted work was preserved. No commit,
deploy, reset, rebase, clean, or destructive git was run.

## Status

Implemented additive ingestion of optional external status snapshots for the
Fleet, GPU, inference, and service sections. A section is populated **only** when
its snapshot file exists, parses, validates, and is fresh (≤60s by `updated_at`,
else file mtime). Missing, stale, or malformed snapshots keep the section
`UNAVAILABLE`/`STALE` with an explicit reason; no success is ever inferred.

`serde_json` was already a normal dependency of `aegis-ui` (`=1.0.140`), so it was
used directly. **No dependency was added or changed.**

## Files changed

- `crates/aegis-ui/src/model.rs` — snapshot loader, parsers, `SnapshotState`,
  freshness check, model wiring, `summary_json` status/reason propagation, tests.
- `crates/aegis-ui/src/tui.rs` — render `STALE`/`UNAVAILABLE` reasons for the four
  sections, updated test fixtures.
- `crates/aegis-ui/src/lib.rs` — documentation only (no API change).
- `crates/aegis-ui/tests/tui_dashboard.rs` — fresh/missing/stale/malformed tests.
- `docs/implementation/BATCH7_AGENT05_TUI.md` — this document.

Not edited: `crates/aegis-ui/src/{gui,html,actions}.rs`, `crates/aegis-cli`,
workspace files, `Cargo.lock`, or any other crate.

## Snapshot location and freshness

Files are read, relative to the Aegis root:

```
<root>/run/status/fleet.json
<root>/run/status/gpu.json
<root>/run/status/inference.json
<root>/run/status/service.json
```

Each file may include a top-level `updated_at` RFC 3339 string. If present it must
parse; if absent (or `null`) the file's modification time is used. A snapshot with
an age greater than `60` seconds is `STALE`; an unparseable timestamp is treated
as malformed.

There is **no CLI emitter yet**. The files are optional; an operator (or a future
CLI/agent) must write them atomically (write to a temp file, then rename) at least
once per minute for the dashboard to show live values. Until then the sections
remain explicitly unavailable.

## Snapshot schemas

Unknown extra fields are ignored. Required fields are marked; optional fields may
be omitted or `null`.

### `fleet.json`

```json
{
  "updated_at": "2026-11-26T12:00:00Z",
  "hosts": [
    {
      "host_id": "host-a",
      "reachable": true,
      "model_count": 3,
      "latency_ms": 4,
      "error": null
    }
  ]
}
```

- `hosts` (required): array.
- per host: `host_id` (required string), `reachable` (required bool),
  `model_count` (optional u64, default `0`), `latency_ms` (optional u64, default
  `0`), `error` (optional string/null).

### `gpu.json`

```json
{
  "updated_at": "2026-11-26T12:00:00Z",
  "available": true,
  "name": "NVIDIA RTX",
  "utilization_percent": 12,
  "free_vram_mb": 6144,
  "temperature_c": 55,
  "error": null
}
```

- `available` (required bool).
- `name` (optional string), `utilization_percent` (optional u8, 0–255),
  `free_vram_mb` (optional u64), `temperature_c` (optional u8, 0–255),
  `error` (optional string/null).

`available: false` is rendered as `UNAVAILABLE` with `error` (or the default
reason), not as a healthy GPU.

### `inference.json`

```json
{
  "updated_at": "2026-11-26T12:00:00Z",
  "active": true,
  "model": "aegis-small",
  "host_id": "host-a",
  "detail": null
}
```

- `active` (required bool).
- `model`, `host_id`, `detail` (optional string/null).

### `service.json`

```json
{
  "updated_at": "2026-11-26T12:00:00Z",
  "running": true,
  "pid": 4242,
  "pid_alive": true,
  "socket_reachable": true,
  "error": null
}
```

- `running`, `pid_alive`, `socket_reachable` (required bool).
- `pid` (optional u32), `error` (optional string/null).

## Behavior

| Snapshot condition | `model.<section>` | TUI label | `summary_json` status |
| --- | --- | --- | --- |
| File missing | `None` | `UNAVAILABLE: <existing reason>` | `unavailable` |
| Malformed JSON / invalid schema / unreadable | `None` | `UNAVAILABLE: ... malformed ...` | `unavailable` |
| Older than 60s | `None` | `STALE: ...` | `stale` |
| Fresh and valid | `Some(view)` | normal values | `available` (or `none` when empty) |
| Fresh, GPU `available:false` | `Some`, unavailable | `UNAVAILABLE: <error>` | `unavailable` |

`render_dashboard`, `run_tui`, `run_tui_once`, `render_summary_json`, and the
`aegis.ui.summary.v1` schema remain backward compatible. Missing snapshots keep
the pre-existing unavailable reasons so existing callers/tests are unaffected.

## Verification

- `cargo test -p aegis-ui` — **57 passed, 0 failed**
  - 22 lib unit tests (17 pre-existing + 5 new snapshot loader tests)
  - 26 `gui_dashboard.rs` tests (unchanged)
  - 9 `tui_dashboard.rs` tests (5 pre-existing + 4 new: fresh, missing, stale,
    malformed)
- `cargo clippy -p aegis-ui --all-targets -- -D warnings` — **clean**
- No whole-workspace gates were run.

## Wired vs. still unavailable

Wired from optional fresh snapshots (with explicit fallbacks):

| Section | Source when fresh | Fallback |
| --- | --- | --- |
| Fleet hosts | `run/status/fleet.json` | `UNAVAILABLE`/`STALE` reason |
| GPU telemetry | `run/status/gpu.json` | `UNAVAILABLE`/`STALE` reason |
| Active inference | `run/status/inference.json` | `UNAVAILABLE`/`STALE` reason |
| Service status | `run/status/service.json` | `UNAVAILABLE`/`STALE` reason |

Still unavailable (unchanged):

| Section | Gap |
| --- | --- |
| Memory telemetry | No memory data source or snapshot is defined yet. |

Platform, Cores, modules, diagnostics, and database continue to come directly from
the real backend APIs exactly as in Batch 6.

## Limitations

- No CLI/agent writes `run/status/*.json` yet; until one does these sections stay
  `UNAVAILABLE`.
- Freshness is only as trustworthy as the writer's `updated_at` (or filesystem
  mtime). A stale-but-recently-touched file with a wrong `updated_at` is still
  rejected; a writer that omits `updated_at` inherits mtime semantics.
- The console remains an honest snapshot that re-reads state each refresh, not a
  live event stream.
- Feature detection is file-based and section-local; there is no cross-file
  consistency validation (e.g. inference host not in fleet).

## Next steps

1. Add a small CLI/agent emitter that atomically writes the four schemas (real
   data only, never fabricated) so operators get live TUI values.
2. Optionally add `/api/summary` to the HTTP dashboard reusing `summary_json`.
3. If a memory telemetry source is designated, add a `run/status/memory.json`
   schema and extend the same loader pattern.
4. Consider surfacing snapshot freshness (age) explicitly in the summary for
   operator debugging.
