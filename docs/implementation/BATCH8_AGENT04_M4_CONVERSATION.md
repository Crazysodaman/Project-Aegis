# Batch 8 — Agent 04 / Milestone 4: Conversation & Runtime Integration

Status: **Complete**
Crate: `aegis-conversation`
Baseline HEAD: `2bd2513`
Verification: `cargo test -p aegis-conversation`, `cargo clippy -p aegis-conversation --all-targets -- -D warnings`

## Scope

M4 wires the already-implemented shared world contract
(`aegis_contracts::world::WorldSnapshot`) and per-Core personal contracts
(`aegis_contracts::personal::{ContinuitySummary, EmotionState, Activity}`) into
the provider-independent conversation engine, without changing the engine's
authority posture.

Files changed:

- `crates/aegis-conversation/src/engine.rs`
- `crates/aegis-conversation/tests/world_integration.rs` (new)
- `crates/aegis-conversation/src/lib.rs` (public surface unchanged; no code edit required)
- `docs/implementation/BATCH8_AGENT04_M4_CONVERSATION.md` (this file)

No new dependencies, no changes to `Cargo.toml`/`Cargo.lock`, no changes to
`aegis-cli`, contracts, core-sdk, or platform.

## Integration points

### `TurnOptions`

New optional fields, all defaulting to "absent" so existing callers and
`TurnOptions::new(model)` behavior are unchanged:

| Field | Type | Builder |
| --- | --- | --- |
| `world` | `Option<WorldSnapshot>` | `with_world(WorldSnapshot)` |
| `continuity` | `Option<ContinuitySummary>` | `with_continuity(ContinuitySummary)` |
| `emotion` | `Option<EmotionState>` | `with_emotion(EmotionState)` |
| `activities` | `Vec<Activity>` | `with_activities(Vec<Activity>)` |

All pre-existing builders and defaults (`memory_limit`, `history_limit`,
temperature, token/timeout, think, system instructions) are preserved.

### Prompt assembly

`compose_system_prompt` now accepts the full `TurnOptions` and returns an
internal `ComposedPrompt` carrying both the text and usage flags. Sections, in
order:

1. Identity + descriptive-context disclaimer (unchanged).
2. Personality tone/values (unchanged).
3. **Shared world context** — local date/time, `time_of_day`, season, timezone.
   Weather is rendered only when `WorldSnapshot::weather` is present and always
   includes `provenance.source`, `observed_at` (or `time unknown`), and the
   snapshot `freshness`. Stale/unavailable snapshots add an explicit freshness
   note; a missing observation renders `Weather: not provided (freshness: …)`.
4. **Continuity** — `last_active_at`, `elapsed_offline_seconds`, recent
   experiences, open threads, and continuity activities.
5. **Internal emotional tone** — the six dimensions plus overall intensity,
   formatted numerically. No single mood label is synthesized.
6. Additional `TurnOptions::activities` (self-reported state).
7. Private memories (unchanged, per-snippet bound).
8. Caller-supplied system instructions (unchanged).

### Bounds / truncation

- Per-list-item render cap: `MAX_PROMPT_ITEM_CHARS` (160).
- Per-section list cap: `MAX_PROMPT_LIST_ITEMS` (6).
- Whole prompt hard cap: `MAX_SYSTEM_PROMPT_CHARS` (8,000).
- Truncation is character-based (`char::take`), never byte slicing, so it is
  UTF-8 safe and cannot panic. `truncate_prompt` reserves one slot for the
  trailing `…`, guaranteeing the result is `<= MAX_SYSTEM_PROMPT_CHARS`.
- Non-finite emotion dimensions render as `n/a` instead of panicking or
  producing invalid floats.
- No transcript/history dumping: only bounded summaries are included, sourced
  from the contract types and the supplied options.

### Usage reporting

`PreparedTurn`, `ConversationTurn`, and `AssistantTurn` gained:

- `world_used: bool`
- `continuity_used: bool`
- `emotion_used: bool`
- `activities_used: bool`

These flags are set from the inputs actually composed into the prompt (all
existing fields/API preserved; this is purely additive). They let callers and
tests prove that context was consumed.

### Continuity update

After a **successful** turn (`send_turn`, `send_turn_streaming`,
`continue_after_tools`) the engine calls:

```rust
sdk.personal()?.update_last_active(Some(session_id))
```

The update is best-effort: a personal-store/open failure does **not** fail the
turn. The failure is surfaced as `personal_update_warning: Option<String>` on
the returned `ConversationTurn` / `AssistantTurn`. `None` means the update
succeeded.

## Tests and results

New file `crates/aegis-conversation/tests/world_integration.rs` (6 tests, no
live LLM required):

1. `same_world_different_cores_produce_distinct_bounded_prompts` — two Cores
   under the same `WorldSnapshot` with different personality/emotion produce
   different bounded prompts; world/emotion/activity flags true.
2. `stale_and_unavailable_world_still_produce_valid_prompts` — stale (with
   weather) and unavailable (no weather) snapshots both yield valid prompts and
   explicit freshness text.
3. `prompt_length_is_capped_with_long_and_multibyte_inputs` — ~20k multibyte
   characters in continuity/activities/instructions; prompt is exactly capped at
   `MAX_SYSTEM_PROMPT_CHARS` and ends with the safe ellipsis.
4. `continuity_values_and_context_flags_are_included` — elapsed seconds,
   experiences, threads, and activities appear in the prompt; flags true.
5. `successful_turn_updates_personal_continuity_and_reports_flags` — a real
   turn updates `last_session_id` via `PersonalStore::continuity_summary()` and
   reports no warning.
6. `default_options_preserve_single_core_behavior` — default options set no
   context flags, keep identity/personality content, and still complete a turn.

Results:

```
cargo test -p aegis-conversation
  unittests src/lib.rs .......... 1 passed
  tests/conversation.rs ........ 13 passed
  tests/world_integration.rs ... 6 passed
  total ......................... 20 passed, 0 failed

cargo clippy -p aegis-conversation --all-targets -- -D warnings
  Finished ... (no warnings)
```

Existing conversation tests were not modified and remain green.

## What is actually used

- `WorldSnapshot`: timezone, local date/time, time-of-day, season, freshness,
  provenance source, optional weather (condition, temperature, observed_at), and
  explicit stale/unavailable messaging.
- `ContinuitySummary`: last active, elapsed offline seconds, recent experiences,
  open threads, activities.
- `EmotionState`: all six dimensions and intensity (numeric, no mood label).
- `Activity`: status, title, updated timestamp.
- `PersonalStore::update_last_active(Some(session_id))` after successful turns.

## Limitations

- The world snapshot's `forecast`, `calendar`, and `sun` fields are not rendered
  in this milestone (kept bounded and optional; can be added later behind the
  same caps).
- Only `update_last_active` is written; no bounded experience is appended, to
  avoid unintentionally growing the personal event log from the engine.
- Usage flags indicate that an input was composed, not how many characters
  survived the final global truncation.
- No engine-level caching of world/continuity; callers supply a fresh snapshot
  and summary per turn (matching the existing `TurnOptions` model).
- The CLI does not yet populate these options; that wiring is out of scope for
  M4 and reserved for the runtime/CLI milestone.

## Next steps

- Populate `with_world`/`with_continuity`/`with_emotion` from the Platform world
  service and the Core personal store in the CLI/runtime.
- Optionally append a bounded experience summary after turns once runtime
  semantics are defined.
- Add forecast/calendar rendering if product requirements call for it.
