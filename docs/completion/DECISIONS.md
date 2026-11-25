# Decisions

## Locked architecture

- Rust-first workspace in `D:\Aegis`; legacy repository read-only.
- Platform, Safe, and per-Core SQLite files are physically separate.
- Core filenames use canonical immutable Core UUIDs.
- Fresh initialization uses `fresh_no_migration` and no historical import.
- Integrity = content digest, not authenticity.
- Owner proof is required for mutating Core lifecycle operations.
- Display names are mutable; UUIDs and Core numbers are immutable.
- Cores #4/#5 remain prohibited; #6 design-only until implemented; #7 blank.
- A-04, B-01, B-06 remain provisional.
- No hidden autonomy, privilege escalation, or personality-based authorization.

## Foundation decisions

- Core DB app marker `AEGC`; Platform `AEGP`; Safe `AEGS`; memory `AEGM`.
- Initialization markers live under `<root>/init/`, not inside the Core directory.
- `database_instance_uuid` is distinct from `core_uuid` and is recorded in the manifest.
- Core init/rename/deactivate/reactivate/uninstall-preserve require Safe owner proof.
- UUIDs are canonicalized at every identity boundary.

## Roster, installer, and host decisions (2026-11-24)

- The approved roster lives in a dedicated `aegis-roster` crate as data, not in generic Platform code.
- Roster #1/#2/#3/#6 enforce their approved display name at roster-based install; renames remain an owner-authorized post-install action.
- Roster #4/#5 are `Prohibited`, carry no identity, and cannot be installed.
- Roster #7 has no default name; an owner-supplied display name is mandatory, and no identity attributes are invented.
- Installer state is recorded in `platform.db` (`platform_config`), so it is included in normal backups.
- Upgrade refuses downgrades and always takes a verified backup outside the storage root before applying changes.
- Upgrade is idempotent at the current version (`already_current`) and performs no backup in that case.
- Uninstall preserves data by default. Delete mode requires an explicit `--delete-data` choice, a backup, and a confirmation equal to the Platform callsign.
- Deletion refuses symlinks/reparse points at every level and removes only known Aegis artifacts.
- Host identity is stored in `platform.db` and is separate from every Core identity.
- Host enrollment is an explicit owner-authorized trust transition with append-only history; there is no implicit enrollment or network discovery.
- Host enrollment is local trust metadata only and grants no Core/module privileges.

## Module execution and conversation decisions (2026-11-24)

- Module execution is process-based for this homelab: only `process`/`native` entrypoints run, and only from an enabled module's own package directory.
- Package install verifies declared assets by size and SHA-256, stages them, registers the manifest, and only then moves the staging directory into place.
- Module children run with a cleared environment, no owner token, and no Platform secrets. Isolation is lifecycle/environment isolation, not an OS sandbox.
- A module is `running` only while its process is alive and its heartbeat file is fresh; registry/DB state alone is never treated as proof.
- Startup and stop are bounded; stop sends a sentinel first and falls back to forced termination. Crash detection is derived from process liveness plus heartbeat age.
- Module runtime scratch state lives under `<root>/run/modules/<id>`; packages live under `<root>/modules/<id>`.
- Conversation data lives in the owning Core's `conversation.db`, bound to both the immutable Core UUID and the Core database instance UUID; a copied database is rejected.
- Conversation history is append-only for messages and events. Sessions are archived or soft-deleted, not hard-deleted.
- Memory context is retrieved by tokenizing the latest user turn into distinct search terms and deduplicating matches by memory id.
- Personality and memory are placed in the system prompt as descriptive context only; they are never authorization inputs.
- Model tool calls are stored and surfaced as data; the conversation layer never executes a tool or grants a permission.
- Conversation CLI mutations require owner proof; conversation reads are owner-gated as private Core data.

## Embodiment and asset decisions (2026-11-24)

- Each Core owns its asset library and appearance configuration in `assets.db` under its own directory, bound to the Core UUID and Core database instance UUID.
- Assets are stored as files under `<core>/assets/<asset_id>/<safe-name>` and verified by SHA-256 on read; this is content integrity, not authenticity.
- Asset import rejects symlinks/reparse points, path traversal, empty content, oversize content, and invalid names/kinds before writing.
- Asset deletion is a soft delete; the file is retained for owner recovery.
- Appearance is versioned with a monotonically increasing revision. Theme colors must be `#RRGGBB`.
- Avatar/expression/gesture/wardrobe/voice-profile bindings require the referenced asset to exist, be live, and have the matching kind.
- The Aegis palette (`#9400D3`, `#00C2FF`, `#39FF14`) is the default theme for every Core.
- These are stable interfaces and configuration; no renderer or audio synthesis provider is required or claimed.

## Tool execution and retrieval decisions (2026-11-24)

- A model tool call is a proposal, never authority. The tool bridge checks a Platform-owned grant (default deny for writes), STOP state, and owner approval for sensitive tools before execution.
- Only built-in, bounded tools execute in this stage: `memory.search`, `memory.create`, `platform.status`, `module.list`, `core.inspect`, `time.now`. No shell, arbitrary code, or model-supplied executable.
- `memory.search` and `memory.create` are bound to the calling Core; a tool argument cannot select another Core's memory.
- Sensitive tools require an explicit owner token at execution time. Read-only tools are enabled by default; `memory.create` is denied until granted.
- Every attempt (executed, denied, approval-required, failed) is appended to the append-only Safe audit chain. Argument values are never audited; only key names and outcome.
- Conversation tool results are persisted as `tool` messages and an optional follow-up assistant turn can be generated after tool results.
- Memory retrieval is a `MemoryRetriever` trait. The only implemented strategy is explicit keyword matching (`KeywordRetriever`); semantic retrieval is not claimed.
- The assembled turn reports its retrieval method so diagnostics and UI cannot imply semantic relevance that was not performed.

## Module tools and embedding decisions (2026-11-24)

- Modules declare tools in their signed-by-digest manifest: name, description, permission token, sensitivity/approval, optional timeout, and an input JSON schema. Declarations are validated at install.
- Platform-facing module tool names are namespaced: `module.<module_id>.<tool_name>`. Module grants live in the same Platform grant table as built-in tools.
- Module tools execute only when the module lifecycle state is `Enabled`. Disable, revoke, STOP, and absent grants deny execution before spawn.
- Module tool execution is a one-shot child process of the module package with a private input/output file, cleared environment, caller-Core UUID, timeout, and output cap. It receives no owner token or database paths.
- Module argument validation enforces JSON-object shape, serialized size, and the schema's required keys. Full JSON Schema type validation is not implemented.
- Module tool results must be `{ "ok": bool, "output": ... }` or `{ "ok": false, "error": "..." }`.
- Module tools cannot grant permissions, bypass the central executor, or call their own path outside `aegis-tools`.
- Embeddings are provider-independent. The local `local-hash` provider is deterministic feature hashing, not a learned semantic model; it is labeled accordingly.
- A real Ollama `/api/embeddings` adapter exists for configured local models. Live verification is required before claiming semantic retrieval is active.
- Embedding vectors are cached per Core in `embeddings.db`, bound to the Core UUID and Core database instance UUID, keyed by content SHA-256 and provider/model. Provider/model changes invalidate cache entries.
- Retrieval reports the actual method used. If the embedding provider/cache fails, retrieval falls back to keyword and reports `keyword(fallback: ...)`.
- The conversation turn exposes `retrieval_method` and `retrieval_semantic` so callers cannot mistake keyword fallback for semantic retrieval.

## Social runtime decisions (2026-11-24)

- Group conversation data is Platform-owned in `platform.db`; per-Core relationship records are Core-private in `<core>/social.db` bound to the Core UUID and instance UUID.
- Groups are private by default. A Core must be an explicit member to list, read, or post. Membership removal takes effect immediately for subsequent operations.
- Only the owner may add or remove members. Cores cannot self-enroll or administer membership.
- The owner retains administrative read/post access to groups for inspection and management.
- Group messages are immutable and attribution includes sender kind and ID. Group events are append-only.
- Relationship records are upserted per `(core, target_kind, target_id)` with revision history; relationship data is never shared across Cores.
- Social operations require owner proof at the CLI boundary; Core-context reads/posts use explicit `--as-core` membership checks.

## Approved Core profile and Fleet decisions (2026-11-24)

- Approved Core profiles are data, not Platform code. They encode only explicitly approved attributes and never invent missing identity data.
- Sofía uses a preservation profile: existing identity/personality/assets are authoritative; the profile records the approved voice reference `sofia_r9_bluehour_REFERENCE_AIR_medium` and refuses to overwrite an existing personality.
- Mack and Nova have approved profile traits/values/appearance; applying a profile is refused if a personality already exists.
- Vera points to the approved external specification and duplicates none of its measurements, hair, armor, colors, voice, or personality here.
- Core #7 remains blank; no identity fields are populated. Cores #4/#5 are prohibited/design-only.
- Fleet hosts are untrusted on add. Enrollment/revocation/removal and policy changes require owner proof.
- Fleet health/model discovery uses the host's configured Ollama endpoint. No GPU utilization or VRAM metrics are fabricated; absent measurements are reported as unavailable.
- Placement uses only enrolled, trusted hosts with a successful health probe; model routing additionally verifies current model availability.
- Fleet adaptive policy currently stores the gaming-yield flag; actual host load/GPU detection remains unimplemented and unverified.

## Live cognition and GPU decisions (2026-11-25)

- Chat requests expose an optional provider thinking-mode override (`think`) because local reasoning models may spend the entire token budget on hidden reasoning and return empty visible content.
- Empty provider content is treated as an explicit `EmptyResponse` failure; it is recorded as `turn.empty_response` and no assistant message is persisted.
- Live verification is scoped to the actual host/model tested: Ollama 0.40.17, `qwen3.5:9b` with `think:false`, RTX 3080 Ti.
- GPU telemetry is read from `nvidia-smi`; missing or unparseable telemetry is reported unavailable and GPU admission fails closed for background inference.
- GPU admission denies on gaming-yield utilization, low free VRAM, high temperature, or missing telemetry. The default policy yields at >25% utilization, <2048 MB free VRAM, or >80 C.
- Policy fields were added via additive SQLite columns with defaults, preserving existing Fleet policy rows.
- Live gaming-load yield, remote Venus traffic, and live embedding generation are not claimed.

## Embedding and yield decisions (2026-11-25)

- Ollama CLI key-path mismatch was resolved by copying the existing trusted key from `D:\ollama` to the expected profile path; no signature verification was bypassed.
- Live embeddings use `nomic-embed-text` (768 dimensions) through the real Ollama `/api/embeddings` endpoint.
- The embedding HTTP adapter decodes chunked transfer encoding; malformed/truncated chunks fail closed.
- Semantic retrieval is selected explicitly (`--retrieval ollama --embedding-model ...`) and reports `retrieval_method=embedding:ollama:<model>`, `retrieval_semantic=true`. Keyword and local feature-hash methods remain distinct and honestly labeled.
- Cooperative GPU yield unloads an Ollama model with `keep_alive: 0` via `/api/generate`. This is a best-effort yield, not guaranteed preemption of an active generation.
- Automatic gaming-load detection/preemption is not implemented or live-verified.

## Pending architectural decisions

See `docs/aegis-discovery-v0.2.0/06_OWNER_DECISIONS_LOCKED.md`. Do not settle provisional items without owner authority.
