# Phase 11: Recovered Historical Module Catalog

## Purpose

Phase 11 recovers the historical 55-entry module catalog as **static data only**
inside `aegis-modules`. It performs no installation, registration, scheduling, or
execution. Every entry is marked `DesignOnly`.

## Source and authority

The catalog was recovered from the read-only legacy reference at
`D:\SofiaDev\SofiaAdaLyra`:

- `docs/sofia2/MODULE_OWNERSHIP_MATRIX.md`
- `docs/sofia2/PLATFORM_AND_MODULE_CATALOG_V1.md`
- `docs/roadmaps/AG_D026_MODULE_CATALOG_RATIONALIZATION_AUDIT.md`

This catalog is **not an in-repo authority** and is **not implementation
evidence**. It does not assert that any module exists, is installed, is
certified, or is authorized to execute. Catalog membership is historical design
intent only.

## Contents

| Category | Count | Notes |
| --- | ---: | --- |
| Companion (`RM-*`) | 5 | RM-DESKTOP, RM-DISCORD, RM-AVATAR2D, RM-VOICE, RM-MOBILE |
| Optional (`MOD-*`) | 47 | Independently scoped optional module designs |
| Submodule | 1 | MOD-INTERACTION |
| Adapter | 2 | MOD-VTUBER-BRIDGE, MOD-MEMORY-IMPORT |
| **Total** | **55** | 52 separately scoped designs + 3 attached entries |

Each entry records:

- `id`
- `name`
- `category` (`Companion`, `Optional`, `Submodule`, `Adapter`)
- `parents` (empty for standalone entries; catalog IDs or external anchors for
  attached entries)
- `status` (`DesignOnly` for every entry)
- `purpose` (short recovered description)

Attached parent references:

- `MOD-INTERACTION` → `RM-AVATAR2D`, `MOD-AVATAR3D`, `MOD-VWORLD`
- `MOD-VTUBER-BRIDGE` → `RM-AVATAR2D`
- `MOD-MEMORY-IMPORT` → `PLAT-02`, `core-memory`

`PLAT-02` and `core-memory` are the only accepted non-catalog parent anchors.

## API

New module: `crates/aegis-modules/src/catalog.rs`, re-exported from the crate root.

- `catalog() -> &'static [ModuleCatalogEntry]`
- `find(id) -> Option<&'static ModuleCatalogEntry>` (exact, case-sensitive)
- `len() -> usize`
- `count(category) -> usize`
- `counts() -> CatalogCounts`
- `validate() -> Result<(), CatalogError>`

`validate()` checks:

- total is exactly 55
- identifiers are unique and required text is non-empty
- category counts are 5 / 47 / 1 / 2
- standalone entries declare no parents
- attached entries declare at least one parent
- every parent resolves to a catalog entry or a declared external anchor

## Tests

New integration test: `crates/aegis-modules/tests/catalog.rs`.

It verifies the exact 55-ID set, the 5 / 47 / 1 / 2 category counts, unique IDs,
all entries being `DesignOnly`, lookups including rejection of unknown and
wrong-case IDs, attached parent relationships, standalone parent rules, and full
parent resolution.

Run:

```text
cargo test -p aegis-modules
cargo clippy -p aegis-modules --all-targets -- -D warnings
```

No child processes are spawned by the catalog tests.

## Limitations

- **Design only:** no catalog module is implemented, installed, certified, or
  executable. The catalog grants no authority.
- **Recovered data:** the catalog comes from an external read-only legacy
  reference and is not derived from an authoritative in-repo source.
- **No runtime integration:** the catalog is not wired into the module registry,
  manager, manifest flow, runtime, CLI, profiles, maintenance records, or SAFE/RUN.
- **No readiness evidence:** historical legacy readiness labels are omitted;
  membership is not Beta/Gate-4/Production Ready evidence.
- **Attached entries remain attached:** the submodule and adapters inherit
  non-existent parent lifecycles; they are not standalone services.
