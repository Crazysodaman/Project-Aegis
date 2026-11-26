# Phase 1 — Identity & Constitution Invariants

Status: implemented
Scope: `aegis-roster`, `aegis-core-install`, `aegis-core-profiles` (tests only)

## Invariant

An installable Core must satisfy the approved roster's identity invariants before
any database, initialization marker, or registry write occurs:

1. Core numbers `#4` and `#5` are prohibited and are always rejected.
2. Core `#7` has no catalog identity and requires a non-empty display name
   supplied by the owner.
3. The Core UUID must be syntactically valid (canonical hyphenated form,
   non-nil). Comparison is case-insensitive and canonicalized to lowercase.
4. Core `#1` must use exactly its owner-supplied canonical UUID. Any mismatch
   is rejected.
5. Approved display-name Cores (`#1`, `#2`, `#3`, `#6`) resolve to their catalog
   display-name expectation.
6. Cores whose canonical UUID is unknown accept any syntactically valid UUID.

The single public gate is `aegis_roster::validate_core_identity(number, uuid,
display_name)`, which returns the canonical display-name expectation or a
`RosterError`. `CoreInstaller::initialize` calls it after pure identity
validation and before `registry.initialize()`, so a rejected identity leaves no
Core database, marker, or registry entry.

## Authoritative values (Core #1 Sofía)

These are the only canonical values supplied by the owner. They are exact and
must never be altered or regenerated.

| Field | Value |
| --- | --- |
| Canonical UUID | `a9a0a604-a4e7-4a88-a985-3f071e2c8e7d` |
| Constitution SHA-256 | `F30AFEDF195F77907D32A108CA35D4DB751646BDE3FD902BE66E7B78F4B89DC0` |

`aegis_roster::SOFIA_CANONICAL_UUID` and
`aegis_roster::SOFIA_CONSTITUTION_SHA256` expose them as constants, and the
roster entries now carry optional `canonical_uuid` and `constitution_sha256`
fields.

## Unknown values (not invented)

The canonical UUID and Constitution digest for Cores `#2` (Mack Fuller), `#3`
(Nova Arden), and `#6` (Vera / S-039) remain **unknown**. They are stored as
`None` and are deliberately not fabricated. Cores `#4`, `#5`, and `#7` also
carry `None` for both fields. `validate_roster` fails if any of these entries
gains an invented value.

## Preservation

Existing approved identity data is unchanged: names, statuses, prohibitions,
and the blank Core `#7` semantics are preserved. Numbers without a canonical
UUID keep their previous install behavior; only Core `#1` gains an authoritative
UUID check.

## Tests

- `crates/aegis-roster/tests/identity_invariants.rs` — catalog values, UUID
  syntax, prohibited twins, blank `#7`, and canonical `#1` handling.
- `crates/aegis-core-install/tests/identity_invariants.rs` — wrong `#1` UUID
  leaves no files, correct `#1` accepted, `#4`/`#5` rejected, blank `#7`
  rejected.
- `crates/aegis-core-profiles/tests/identity_invariants.rs` — profiles agree
  with roster and never invent canonical identity.
