# Wave 4b — Development Tools and CLI

Wave 4b exposes the per-Core development substrate from Wave 4 through the
owner-authorized tool bridge and the `aegis` CLI. Every call is caller-bound to
the executing Core, appended to the Safe audit chain, and STOP-gated. Applying,
rejecting, or reverting a personality change additionally requires the owner
token.

## Tools (`crates/aegis-tools`)

| Tool | Sensitivity | Input |
| --- | --- | --- |
| `preference.record` | `personal_write` | `{ key, valence (-1..1), weight (0..1), source? }` |
| `preference.list` | `read_only` | `{ limit? }` |
| `development.reflect` | `personal_write` | `{}` |
| `development.propose` | `personal_write` | `{ rationale, values?: string[], traits?: object }` |
| `development.list` | `read_only` | `{ limit? }` |
| `development.apply` | `sensitive` | `{ proposal_id }` |
| `development.reject` | `sensitive` | `{ proposal_id }` |
| `development.revert` | `sensitive` | `{}` |

Implementation:

- Each handler opens a `DevelopmentStore` bound to the caller Core
  (`DevelopmentStore::open(root, caller_core)`), so tools can never read or write
  another Core's development state.
- `preference.record` validates the numeric bounds before recording evidence;
  `development.propose` accepts an optional values array and a bounded
  `string -> number` trait map, validates trait values in `-1..1`, and lets the
  store validate the full proposal.
- The executor's `run` now receives `Option<&str>` owner token (minimal change).
  Non-sensitive handlers ignore it; `development.apply`, `development.reject`,
  and `development.revert` pass it to the store, which verifies the Safe owner
  proof. Execution without a token never reaches those handlers because the
  existing sensitive-approval gate returns `ApprovalRequired`.
- Module tool behavior is unchanged: module bindings still execute through
  `run_module_tool` and never receive the owner token.

## CLI (`crates/aegis-cli`)

New living commands:

```text
aegis living preference record --root <path> --core <id> --key <key> --valence <n> --weight <n> [--source <text>]
aegis living preference list --root <path> --core <id> [--limit <n>]
aegis living development reflect --root <path> --core <id>
aegis living development propose --root <path> --core <id> --rationale <text> [--values a,b] [--traits key=value,key=value]
aegis living development list --root <path> --core <id> [--limit <n>]
aegis living development apply --root <path> --core <id> --proposal-id <id>
aegis living development reject --root <path> --core <id> --proposal-id <id>
aegis living development revert --root <path> --core <id>
```

`--values a,b` is split into a JSON string array and `--traits
"key=value,key=value"` is parsed into a JSON object of numbers. Apply, reject,
and revert fetch the Platform owner token and pass it to the executor; the other
commands do not need an approval token.

## Tests

`crates/aegis-tools/tests/development_tools.rs` uses the real `ToolExecutor`
against canonical Core #1 and #2 in a real Platform root:

1. `preference.record` forms a preference, `preference.list` shows it, and
   `development.reflect` summarizes it.
2. `development.propose` lists a proposal; apply without a token returns
   `ApprovalRequired`; apply with the Platform owner token executes and appends
   personality growth; revert with the token executes and appends a restored
   revision.
3. Rejecting without a token returns `ApprovalRequired`; rejecting with the token
   succeeds and a rejected proposal cannot be applied.
4. Mack cannot see Sofía's preferences or proposals, and cannot apply Sofía's
   proposal even with the owner token.

Existing tool and CLI tests are preserved.

## Verification

```text
cargo test -p aegis-tools -p aegis-cli
cargo clippy -p aegis-tools -p aegis-cli --all-targets -- -D warnings
```

## What is real / what is not

Real:

- Preferences, reflections, proposals, apply/reject/revert all persist through
  the real Wave 4 `DevelopmentStore` in the Core's `personal.db`; apply and
  revert append real personality growth revisions in `memory.db`.
- Sensitive operations require the local Safe owner proof.
- Cross-Core isolation is enforced by the caller-bound executor.

Bounded:

- `preference.record` is the only evidence-writing tool; there is no tool to
  record a bare experience.
- Trait values are restricted to `-1..1`; other proposal bounds come from the
  contracts.
- There is no background scheduler or automatic reflection/apply loop.
