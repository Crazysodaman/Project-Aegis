# OPENCODE WORKORDER
# Project Aegis v0.2.0 Read-Only Architecture and Repository Audit

**Workorder ID:** AEGIS-WO-AUDIT-002  
**Version:** 1.0.0 (aligned to blueprint v0.2.0)  
**Prepared:** 22 November 2026  
**Target:** Project Aegis  
**Repository:** `D:\SofiaDev\SofiaAdaLyra`  
**Governing blueprint:** `Project_Aegis_Master_Architecture_Blueprint_v0.2.0.md`  
**Blueprint document ID:** AEGIS-MAB-002  
**Execution mode:** READ-ONLY discovery, architecture validation, and evidence inventory.  
**Stop condition:** Stop after producing the seven audit deliverables. Do not implement, refactor, migrate, install, activate, commit, or push.

---

## 0. Instruction to the executing OpenCode session

You are performing a comprehensive, read-only architecture and repository audit of Project Aegis against the v0.2.0 master blueprint.

This workorder is not an implementation request. It is an evidence-gathering and gap-analysis request. Your value is accurate classification, not optimistic reporting. If something cannot be verified, say so. If something is missing, say so. If code contradicts the blueprint, record it. Never convert a design statement, a file name, a passing unit test, or a successful build into a claim of runtime verification.

Read this entire workorder before acting.

---

## 1. Non-negotiable restrictions

During this audit you must not:

- Implement or modify application code.
- Refactor source.
- Modify, migrate, vacuum, or write to any database.
- Run database migrations.
- Modify, regenerate, rename, or delete assets.
- Install, upgrade, or remove packages or dependencies.
- Modify services, scheduled tasks, startup entries, or registry state.
- Change branches, create branches, commit, amend, push, tag, or open pull requests.
- Activate, initialize, register, package, or deactivate any Core.
- Initialize or activate Cores #4 or #5.
- Invent identity details for Core #7.
- Alter Sofía's Core UUID or Constitution digest.
- Import historical experiences into Sofía's new database.
- Perform any production database cutover.
- Delete or clean up legacy material.
- Claim live verification without recorded evidence.
- Dump private, intimate, or sensitive conversation content into audit documents.
- Copy secrets, tokens, credentials, or owner proof material into deliverables.

Read-only inspection may include copying a database file to a temporary directory and opening the copy in SQLite read-only mode. The original file must not be opened with a writable connection, and no `-wal` or `-shm` side effects may be forced on the original.

If a required check would violate these restrictions, stop that check and record it as "not performed: read-only restriction" with the reason.

---

## 2. Authority and source hierarchy

Resolve conflicts in this order:

1. Explicit owner decisions recorded in the v0.2.0 blueprint (especially sections 25 and 33).
2. The v0.2.0 master blueprint text.
3. The v0.1.0 baseline (historical context only; v0.2.0 supersedes it).
4. Existing repository code, schemas, assets, and documentation.
5. Prior audit reports and progress documents.

The repository is evidence, not authority. Existing behavior does not override an approved owner decision merely because it exists.

Locate the blueprint before starting. Candidate locations:

- `D:\SofiaDev\SofiaAdaLyra\Project_Aegis_Master_Architecture_Blueprint_v0.2.0.md`
- `D:\SofiaDev\SofiaAdaLyra\docs\`
- `D:\Aegis\Project_Aegis_Master_Architecture_Blueprint_v0.2.0.md`

If multiple copies exist, compare them and record which one you treated as governing. Do not edit any blueprint copy during the audit.

---

## 3. Step 0 — Verify repository state and record the audit frame

Record and include in `00_EXECUTIVE_AUDIT.md`:

1. Absolute repository path.
2. Whether the path exists and is a Git repository.
3. Current branch.
4. HEAD commit full SHA and short SHA.
5. Working-tree status (`git status --short`), including modified, staged, and untracked files.
6. Whether the tree is clean or dirty. If dirty, list every modified path. Do not revert or stash anything.
7. Operating system, shell, toolchain versions relevant to inspection (Rust, Python, SQLite, Node if present).
8. Date and time of the audit, with timezone.
9. The exact blueprint path and SHA-256 digest used as the governing document.
10. A statement that no repository, database, asset, or service state was modified.

Known baseline to re-verify, not assume:

- Repository: `D:\SofiaDev\SofiaAdaLyra`
- Branch: `main`
- HEAD previously reported: `d1666bef53fae9cf2681cca64bea9ba86a93a78f`
- Prior working tree has been reported dirty. Re-verify and report the current state.

If the branch or HEAD differs from the prior report, record the difference and continue read-only. Do not attempt to restore the prior state.

---

## 4. Step 1 — Inventory the repository

Produce a complete inventory. For each area, record paths, sizes where useful, and a one-line purpose assessment based only on inspection.

At minimum inventory:

1. **Source trees**
   - Python packages and modules.
   - Rust workspace members and crates.
   - Any JavaScript, TypeScript, shell, PowerShell, or other source.
2. **Services and entry points**
   - CLI entry points.
   - TUI and GUI entry points.
   - Daemons, brokers, workers, agents.
   - Service definitions, systemd units, Windows services, scheduled tasks.
3. **APIs and IPC**
   - HTTP/Unix-socket/TCP endpoints.
   - Contracts and schemas.
   - Authentication and authorization hooks.
   - Event/message formats.
4. **Databases and schemas**
   - SQLite files and any other database files.
   - Schema definitions, migrations, initialization code.
   - Which component owns each database.
5. **Assets**
   - Avatars, rigs, animations, voices, wardrobe, textures, models.
   - Asset manifests, digests, provenance metadata.
6. **Packages and installers**
   - Platform packages.
   - Core packages.
   - Module packages.
   - Installer scripts and packaging manifests.
7. **Tests**
   - Unit, contract, integration, cross-host, failure-injection, backup/restore, UI, performance, acceptance.
   - Which tests actually exist versus which are referenced but absent.
8. **Documentation**
   - Architecture, roadmap, audits, operations, recovery, specifications.
9. **Configuration and state**
   - Config files, environment templates, state directories, caches.
   - Do not print secret values.

Record counts and notable gaps. Do not claim a component is functional because a directory or file exists.

---

## 5. Step 2 — Inspect databases read-only

For every database discovered:

1. Record the path, size, last-write time, and owning component.
2. Copy the file (and any `-wal`/`-shm` files, if present) to a temporary directory outside the repository.
3. Open the copy read-only. Use `sqlite3` with `file:...?mode=ro` or an equivalent read-only connection.
4. Record:
   - Schema version or migration state.
   - Table and index inventory.
   - Row counts per table (no personal content dumps).
   - Foreign-key relationships where discoverable.
   - Ownership markers such as Core UUID, database instance UUID, initialization mode, and creation timestamp.
5. Determine whether the database is Platform, Safe, or a Core database, and whether ownership is actually enforced by the code or only stated in documentation.
6. Identify any evidence of cross-Core data in a single database, shared memory tables, or shared conversation tables.

Privacy rule: report schema, counts, and structural findings only. Do not reproduce conversation text, intimate content, credentials, owner proof material, or private relationship detail. If an example is needed, use a redacted structural description.

Never write to the original database. Never run `VACUUM`, `ANALYZE` that writes, migrations, or `PRAGMA` statements that mutate state.

---

## 6. Step 3 — Inspect Core identities and ownership

Verify and record:

1. Core roster and identifiers.
2. Core #1 Sofía Ada Lyra:
   - Core UUID `a9a0a604-a4e7-4a88-a985-3f071e2c8e7d`.
   - Constitution SHA-256 `F30AFEDF195F77907D32A108CA35D4DB751646BDE3FD902BE66E7B78F4B89DC0`.
   - Whether any code or data alters, regenerates, or bypasses these values.
   - Fresh-database initialization path and whether historical imports are possible.
   - Whether the legacy database remains preserved and read-only.
   - Voice asset `sofia_r9_bluehour_REFERENCE_AIR_medium` status. It is expected to be **NOT VERIFIED** unless actually located and verified.
3. Core #2 Mack Fuller, Core #3 Nova Arden, Core #6 Vera / S-039:
   - Whether approved identity attributes exist in code or assets.
   - Whether canonical Vera specifications and asset digests are locatable.
4. Cores #4 and #5:
   - Whether any implementation, package, registration, or activation exists. If so, record it as a contradiction.
5. Core #7:
   - Whether an unwritten/blank universal slot exists without invented identity data. If any name, gender, species, avatar, voice, or personality is assigned, record it as a contradiction.
6. For every Core database, confirm independent instance identity and backup/restore lifecycle. Record any shared Core database as a high-severity contradiction.

Do not create, initialize, or activate any Core.

---

## 7. Step 4 — Inspect subsystem capabilities

For each subsystem below, record what exists, where it exists, its data ownership, its interface surface, and its evidence status. Cover at minimum:

1. Platform identity, authority, and local state.
2. Platform service, CLI, TUI, GUI, and broker.
3. Host enrollment, Fleet inventory, placement, migration.
4. Core lifecycle, autostart, worker isolation.
5. Core identity and Constitution.
6. Personality and personality development.
7. Cognition, provider abstraction, model routing, dual cognition.
8. Memory, retrieval, learning, reflection, continuity.
9. Emotion and expression.
10. Relationships, social context, social agency, asymmetry.
11. Conversation, messaging, General Chat, private chat, group chat.
12. Social presence, invitations, availability, quiet periods, activity budgets.
13. Autonomy, goals, background activity, scheduling.
14. Modules, permissions, capability broker, sandboxing.
15. Inference and remote execution.
16. Voice and speech (TTS, STT, voice assets, retention).
17. Avatar, expressions, gestures, animation, wardrobe.
18. Identity-specific anatomy and body plans.
19. Action and animation registry.
20. Sofía's fox ears and tail assets, rigs, and animations.
21. Adult and mature capability support, consent, eligibility, privacy.
22. World/context and time handling.
23. Safe, owner approval, STOP, audit.
24. Backup, restore, export, recovery.
25. Installer, packaging, upgrade, rollback.
26. Rust UI and Developer workspace.
27. Legacy Python launch paths and compatibility roles.
28. Observability, diagnostics, logging, privacy of logs.

For each, answer:

- Is it present, partial, design-only, or absent?
- Which files and contracts implement it?
- Which database or store owns its data?
- What authorization gates protect it?
- What tests exist and what is their actual status?
- What runtime evidence exists?
- Does it satisfy, partially satisfy, or contradict the v0.2.0 blueprint?

---

## 8. Step 5 — Verify Sofía's canonical fox features

This is a required, explicit verification. Do not rely on name matching alone.

1. Search for fox ear and tail assets, rigs, animation clips, blend trees, morph targets, texture sets, and compatibility metadata.
2. Search for independent ear and tail movement, expressive states, emotion/context mapping, animation blending, and voice/expression synchronization.
3. Search for persistence across outfits and avatar updates, and compatibility with future rendering systems.
4. Record canonical file paths, formats, digests where available, and owning component.
5. Record any missing, placeholder, generated-substitute, or unverified item.
6. Confirm that other Cores do not inherit Sofía's fox features.
7. If the canonical assets cannot be found, report them as **MISSING OR UNVERIFIED**. Do not regenerate, replace, or remove anything.

Classify each of the following separately: asset present, rig present, animation present, runtime integration present, outfit persistence verified, avatar-update persistence verified, and owner-accepted.

---

## 9. Step 6 — Classify every capability with the evidence vocabulary

For each capability, assign a status using all applicable labels. Do not collapse labels.

- **Designed:** described in documentation or a specification.
- **Coded:** source exists.
- **Built:** compiles or packages successfully, as actually observed.
- **Unit-tested:** unit tests exist and were executed successfully, with the command and result recorded.
- **Integration-tested:** integration tests exist and were executed successfully, with the command and result recorded.
- **Installed:** installed into a runtime environment, as actually observed.
- **Runtime-tested:** exercised in a running process, with evidence.
- **Live-verified:** observed in the intended live environment, with evidence.
- **Owner-accepted:** explicitly accepted by the owner.

Also assign a disposition:

- **Existing and verified**
- **Existing, unverified**
- **Partially implemented**
- **Designed only**
- **New build required**
- **Contradicts blueprint**
- **Retire candidate** (requires owner approval; do not retire anything)
- **Unknown / blocked from inspection**

A file name is not evidence of behavior. A test file is not evidence that the test passes. A passing build is not evidence of runtime correctness.

---

## 10. Step 7 — Identify contradictions

Compare the repository against the v0.2.0 blueprint and record every contradiction, including:

1. Shared Core database or shared private memory.
2. General Chat data stored in a Core private database.
3. Platform hardcoding Sofía-specific assumptions.
4. Cores #4 or #5 implemented, registered, or activated.
5. Core #7 identity invented.
6. Sofía UUID or Constitution digest changed.
7. Historical data imported into Sofía's new database.
8. Legacy database mutated or deleted.
9. Cross-Core direct database access.
10. Social trust used for technical authorization.
11. Relationships automatically reciprocated, synchronized, or averaged.
12. Core disagreement treated as malfunction.
13. Adult capability enabled by default or without per-Core choice.
14. Consent assumed from prior interaction, relationship, attraction, or silence.
15. Consent withdrawal or STOP not halting dependent actions.
16. Intimate content leaking into logs, summaries, or unrelated memories.
17. Identity-specific anatomy, including Sofía's fox features, lost on outfit change.
18. Action registry lacking prerequisites, consent, interruption, or cancellation.
19. Provider replacement erasing identity, relationships, preferences, or continuity.
20. Any implementation claim not backed by evidence.

For each contradiction, record severity, affected paths, blueprint section, observable evidence, and recommended resolution category. Do not fix it.

---

## 11. Required deliverables

Create exactly these seven files, in this directory:

`docs/aegis-discovery-v0.2.0/`

### `00_EXECUTIVE_AUDIT.md`

- Audit frame from Step 0.
- Repository state, branch, HEAD, working tree.
- Blueprint path and digest.
- Scope of inspection and what was not inspected.
- Overall readiness assessment.
- Highest-severity findings.
- Counts by classification and disposition.
- Explicit statement of read-only compliance.
- Explicit list of unverified or blocked areas.
- No implementation claims.

### `01_CAPABILITY_PRESERVATION_MATRIX.md`

A row per capability with columns:

- Capability or subsystem.
- Blueprint section.
- Existing implementation locations.
- Data stores and ownership.
- Dependencies.
- Tests and actual status.
- Runtime evidence.
- Intended behavior.
- New design destination.
- Compatibility needs.
- Risks.
- Classification labels.
- Disposition.
- Owner decision required (yes/no).

Include all subsystems listed in section 7 of this workorder.

### `02_ARCHITECTURE_GAP_AND_CONTRADICTION_REGISTER.md`

A row per gap or contradiction with:

- ID.
- Title.
- Type: gap, contradiction, risk, ambiguity, obsolete assumption.
- Severity.
- Blueprint section.
- Evidence paths.
- Current behavior.
- Required behavior.
- Impact.
- Recommended resolution category.
- Blocking status.
- Owner decision required.

### `03_SUBORDINATE_SPECIFICATION_PLAN.md`

Cover all seventeen specifications from blueprint section 31. For each:

- Purpose and scope.
- Proposed owner.
- Status and version.
- Dependencies.
- Interfaces.
- Data contracts.
- Data ownership.
- Authorization.
- State transitions.
- Failure behavior.
- Versioning and migrations.
- Observability.
- Acceptance tests.
- Open owner decisions.
- Existing repository material that informs it.
- Write order and dependency position.

Do not invent owner decisions.

### `04_REBUILD_DEPENDENCY_GRAPH.md`

- A dependency-ordered rebuild graph.
- Phases and prerequisites.
- Which capabilities can be preserved, improved, replaced, or retired.
- Which items block which phases.
- Critical path.
- Parallelizable work.
- Data and identity preservation gates.
- Explicit statement that ordering is a recommendation pending owner approval.

### `05_TEST_AND_EVIDENCE_PLAN.md`

- Test layers required by blueprint section 21.1.
- All mandatory acceptance areas from section 21.2.
- All twenty-two additional tests from section 30.
- For each: preconditions, evidence required, owner acceptance criteria.
- Current test inventory and actual executed status.
- Gaps between claimed and demonstrated coverage.
- Rules for recording commands, outputs, timestamps, environment, and artifacts.
- Explicit prohibition on claiming a test passed without execution.

### `06_OWNER_DECISIONS.md`

- Only genuinely unresolved decisions that materially affect design.
- Include the open decisions from blueprint sections 26 and 33.
- Group into short interview rounds.
- For each: recommendation with tradeoffs, and what it blocks.
- Do not re-ask already-approved decisions.
- Do not resolve decisions by assumption.

---

## 12. Evidence standards

Every finding must be traceable to evidence. For each:

- Provide repository-relative paths, and line numbers where useful.
- Provide the inspection command or method.
- Distinguish direct observation from inference.
- State confidence when inference is unavoidable.
- Record exact commands and summarized results for executed tests.
- Record the environment in which a test was executed.
- Record what was not executed and why.

Do not include secrets, tokens, credentials, owner proof material, private conversation text, intimate content, or unnecessary personal data in any deliverable. Redact structurally.

---

## 13. Completion and stop condition

When the seven deliverables are written and verified:

1. Report the absolute output paths.
2. Report file sizes.
3. Report the repository branch, HEAD, and working-tree status observed at the start and end of the audit.
4. Confirm that no application code, database, asset, package, service, or task was modified.
5. List every area not inspected and why.
6. List the highest-severity findings.
7. State how many capabilities are verified, unverified, partial, design-only, missing, or contradictory.
8. Stop.

Do not begin implementation. Do not propose that implementation start until the owner has reviewed the deliverables and resolved the open decisions.

---

## 14. Final standard

A useful audit is honest about uncertainty. Preserve approved identity and capability without overstating what exists. Treat the repository as evidence and the blueprint as authority. If a claim cannot be supported, mark it unverified. If the evidence contradicts the blueprint, record the contradiction rather than smoothing it over.

Accuracy over optimism. Evidence over assertion. Read-only throughout.
