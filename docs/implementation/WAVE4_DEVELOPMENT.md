# Wave 4 — Personality / Experience Development

Wave 4 adds a per-Core personality-development substrate. Experience evidence
accumulates into bounded preferences, reflections summarize the strongest
preferences, and personality changes are proposed, owner-approved, and recorded
through the existing append-only personality growth history. Identity, the
Constitution, and authorization are never touched.

## Contracts (`aegis-contracts::living`)

Added before the existing `#[cfg(test)]` module:

- `Preference`: schema, Core UUID, preference key, `value: f32`
  (`-1.0..=1.0`), `confidence: f32` (`0.0..=1.0`), `evidence_count: u64`,
  `updated_at`, `revision`; `validate`.
- `Reflection`: schema, reflection id, Core UUID, bounded summary,
  `created_at`; `validate`.
- `PersonalityProposalStatus`: `Proposed`, `Applied`, `Rejected` with
  `as_str` / `parse`.
- `PersonalityProposal`: schema, proposal id, Core UUID, rationale,
  `proposed_values: Vec<String>`, `proposed_traits: BTreeMap<String, f32>`,
  status, timestamps, revision; `validate` enforces bounded text, value and
  trait bounds, counts, and `revision >= 1`.

New bounds: `MAX_REFLECTION_SUMMARY_CHARS`, `MAX_PROPOSAL_RATIONALE_CHARS`,
`MAX_PROPOSED_VALUES`, `MAX_PROPOSED_VALUE_CHARS`, `MAX_PROPOSED_TRAITS`.

## Per-Core database

Development state is additive inside the existing `<core>/personal.db`. The
store verifies the immutable `personal_ownership` row (Core UUID **and**
database instance UUID) before touching development tables, so a copied
database from another Core or another Core database instance is rejected.

Additive tables:

- `development_preferences` — one EMA preference row per `(Core, key)`.
- `development_evidence` — append-only evidence log (UPDATE/DELETE triggers).
- `development_reflections` — stored bounded reflections.
- `development_proposals` — proposals plus their serialized contract payload.
- `development_events` — append-only development event ledger (UPDATE/DELETE
  triggers).

`DevelopmentStore::open(root, core_uuid)` opens `personal.db` through the
existing `PersonalStore`, confirms ownership, then initializes only the additive
development tables.

## API (`DevelopmentStore`)

- `open(root, core_uuid)`.
- `record_experience(summary, valence, weight, source)` — persists evidence and
  records a real `PersonalStore` experience.
- `record_preference_evidence(key, valence, weight, source)` — appends evidence,
  updates the preference with an EMA value, growing confidence, incremented
  evidence count and revision, and appends an event.
- `get_preference(key)`, `list_preferences(limit)`.
- `reflect()` — bounded summary of the strongest preferences
  (`confidence * |value|`), stored as a `Reflection`.
- `propose_personality_change(rationale, values, traits)` — validates and stores
  a `Proposed` proposal; nothing is applied.
- `get_proposal(id)`, `list_proposals(limit)`.
- `reject_proposal(owner_token, id)` — Safe owner proof required.
- `apply_proposal(owner_token, id)` — Safe owner proof required; loads the
  current `PersonalityProfile` from `memory.db`, merges proposed values and
  traits within bounds, stores the result through the append-only personality
  growth history, and marks the proposal `Applied`.
- `revert_personality(owner_token)` — Safe owner proof required; restores the
  previous growth revision by appending it as a new revision.
- `list_events(limit)`.

`CoreSdkContext::open` opens development automatically and
`CoreSdkContext::development()` borrows the store.

## Tests

`crates/aegis-contracts/src/living.rs` adds parse/validation tests for
preferences, reflections, proposal status, and proposals.

`crates/aegis-core-sdk/tests/development.rs` covers 9 scenarios:

1. Evidence forms a preference; conflicting evidence changes its value while
   confidence grows.
2. Value/confidence/evidence bounds and invalid input rejection.
3. Preferences, reflections, and proposals survive restart.
4. Reflection is bounded and uses the strongest preferences.
5. Apply requires owner proof, actually changes personality growth, and leaves
   `CoreIdentity` unchanged.
6. Rejected proposals cannot be applied; rejection and missing proposals are
   handled.
7. Revert requires owner proof and restores the previous personality revision.
8. Sofía/Mack preference isolation and rejection of a foreign `personal.db` copy.
9. Additive tables exist and development events are append-only; the real
   `PersonalStore` experience is recorded.

Commands run:

```text
cargo test -p aegis-core-sdk -p aegis-contracts
cargo clippy -p aegis-contracts -p aegis-core-sdk --all-targets -- -D warnings
```

## What is real / what is not

Real:

- Evidence and preferences persist in `personal.db` and survive restart.
- `record_experience` writes a genuine `PersonalStore` experience as well as
  evidence.
- Owner approval uses the local Safe owner proof from `aegis-safe`.
- Applying and reverting a proposal writes real append-only personality growth
  revisions in `memory.db`.
- Cross-Core isolation and foreign-database rejection are enforced.

Not real / bounded:

- The EMA and confidence updates are deterministic heuristics, not a learned
  model or an LLM.
- Reflections are generated text summaries of stored preferences, not free-form
  introspection.
- There is no daemon or background reflection scheduler.
- Revert restores the immediately previous growth revision (one step); it is not
  an arbitrary history browser.
- Proposal application merges descriptive values/traits only; it never modifies
  identity, the Constitution, capabilities, or authorization.

## Notes

- No new third-party crates were added.
- `crates/aegis-core-sdk/Cargo.toml` had a broken literal `\`r\`n` sequence on
  the pre-existing `aegis-safe` dependency line; it was repaired so the already
  added workspace dependency resolves.
