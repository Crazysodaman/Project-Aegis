# AEGIS FINAL — Agent 14 (Modules & Extensibility)

HEAD audited: `2bd2513` (working tree also contains concurrent edits by other agents).
Scope touched: `crates/aegis-modules/src/**`, `crates/aegis-modules/tests/**`, this report.
No other crate, CLI, workspace, or `D:\SofiaDev\SofiaAdaLyra` file was modified.

## Files

- `crates/aegis-modules/src/manifest.rs`: additive `ModuleDependency.min_version` /
  `max_version` (optional, inclusive, plain `major.minor.patch`), `version_matches`,
  `has_version_constraint`, `version_expectation`, validation (`min <= max`, malformed
  bounds rejected), and 2 unit tests. Existing `version_requirement` behavior and
  serialization are unchanged; the new fields are `#[serde(default, skip_serializing_if)]`.
- `crates/aegis-modules/src/registry.rs`: read-only `check_enable` / `check_disable`;
  `enable` and `disable` now run the same preconditions in their transaction; disable is
  refused when an enabled module declares a dependency on the target (`DependencyInUse`);
  dependency checks now apply `version_requirement` **and** min/max bounds.
- `crates/aegis-modules/src/manager.rs`: owner-facing `check_enable` / `check_disable`
  delegation (read-only, no owner token needed).
- `crates/aegis-modules/tests/modules.rs`: 4 new integration tests plus stronger
  manager-check coverage. Existing literals updated for the new optional fields.
- No plugin framework, module loader, new dependency, or new crate was added.

## Audit summary

- Discovery: there is no marketplace or remote auto-discovery. Installation is
  owner-initiated from a validated manifest or a local package directory; the registry is
  Platform-owned in `platform.db`. No module receives Platform/Core data.
- Install: manifest validation, duplicate-ID rejection, optional expected digest,
  real-file/real-directory checks (symlink/reparse rejected), per-asset size + SHA-256,
  disk-budget check, entrypoint must be a declared asset, staging then rename, rollback on
  failure.
- Activation/permissions: manifest permission requests are declarative only. Enable
  requires at least one declared permission, valid integrity, legal transition, and
  available/compatible dependencies. It grants no data access.
- Lifecycle: `installed → enabled → disabled → uninstalled` (plus `failed`); illegal
  transitions rejected; events are append-only at the database-trigger level; disable and
  uninstall preserve module data.
- Version: manifest semver, SDK min/max against built-in SDK `1.0.0`, dependency
  `version_requirement`, and new optional min/max bounds. Malformed versions fail closed.
- Failure isolation: child environment cleared (only `SYSTEMROOT` retained on Windows,
  needed to launch), no owner token/secrets; cwd is the module package; entrypoint must
  resolve inside it. Startup/stop timeouts, heartbeat freshness, crash detection, watchdog
  runtime budget, and forced process-tree termination on Windows (`taskkill /T /F`).
  Real-process tests (start/status/stop, crash, forced stop, process tree, recovery, tool
  lifecycle gating) remain in place; no `aegis-example-module` child remained after the
  test run.

## Enforced checks added

- `min_version`/`max_version` are optional inclusive bounds; when combined with
  `version_requirement`, all constraints must pass.
- Install, `enable`, and `check_enable` share the same dependency availability/version
  path, so a provider removed or downgraded after consumer install is caught before enable.
- `check_enable` / `check_disable` are read-only and usable by UI/CLI/automation to
  preview a mutation without changing registry state.
- `disable` and `check_disable` reject a provider that a currently enabled module depends
  on; after the dependent is disabled the provider can be disabled.

## Verification

- `cargo test -p aegis-modules`: PASS, 57 tests, 0 failed.
  - lib unit: 16 (2 new)
  - `isolation`: 3
  - `modules`: 20 (4 new, 1 strengthened)
  - `runtime`: 18 (real child-process tests preserved)
  - doc-tests: 0
- `cargo clippy -p aegis-modules --all-targets --no-deps -- -D warnings`: PASS, exit 0.
- `cargo fmt -p aegis-modules -- --check`: clean.
- Leak check after tests: no `aegis-example-module` process left running.

## Limitations (no sandbox)

- No operating-system sandbox is implemented or claimed: no Windows Job
  Objects/AppContainer, Linux namespaces/seccomp/cgroups, filesystem jail, network
  isolation, or OS memory/CPU cap. Module isolation is lifecycle, environment, and policy
  based only; children run with the Platform user's rights.
- The manifest digest is content integrity, not a publisher signature.
- `disable` does not terminate a live module process; callers must call
  `ModuleRuntime::stop` (tool/start gating still denies a disabled module).
- `uninstall` is not dependency-gated (unchanged); only enable/disable are.
- Min/max bounds accept plain `major.minor.patch` only (no pre-release/build metadata).
- Discovery remains local and owner-driven; no plugin framework or remote registry.
