# PROJECT AEGIS

## Master Architecture Blueprint

**Document ID:** AEGIS-MAB-001\
**Version:** 0.1.0 --- Discovery Baseline\
**Status:** Owner-directed architecture baseline; technical discovery
and subsystem specifications required before implementation\
**Prepared:** 22 November 2026\
**Repository reference:** `D:\SofiaDev\SofiaAdaLyra`\
**Known inspected baseline:** `main` at
`d1666bef53fae9cf2681cca64bea9ba86a93a78f` (audit-reported; must be
re-verified)

------------------------------------------------------------------------

# 0. Document authority, purpose, and limits

This document is the proposed single master architecture blueprint for
the complete Project Aegis ecosystem. It defines the intended system
boundaries, ownership, capabilities, interaction patterns, deployment
model, data principles, engineering constraints, and rebuild roadmap.

It is a **blueprint**, not evidence that the described systems already
exist or work. Every capability must be classified during discovery as:

-   **Existing and verified:** implementation and relevant behavior have
    been demonstrated.
-   **Existing, unverified:** code or assets exist, but runtime behavior
    has not been demonstrated.
-   **Partially implemented:** some components exist, but the end-to-end
    capability is incomplete.
-   **Designed only:** documented or agreed, but not implemented.
-   **New build:** must be designed and implemented.
-   **Retire:** deliberately excluded, with owner approval and a
    recorded reason.

Do not convert a design statement into an implementation claim.
Distinguish designed, coded, built, tested, installed, and live-verified
states in all reports.

## 0.1 Mission

Rebuild Project Aegis from the ground up while preserving the
ecosystem's approved capabilities, behavior, identities, and intended
user experience.

The existing repository, databases, packages, documentation, assets, and
prototypes are **reference material**. They may be inspected to
understand behavior, data relationships, existing capabilities, and
lessons learned. They do not dictate the new architecture and must not
force legacy implementation patterns into the rebuild.

The goal is not a reduced rewrite. The goal is a coherent, modular,
maintainable system with the capabilities the project has accumulated,
built on explicit boundaries and testable contracts.

## 0.2 Guiding priorities

In descending order:

1.  Correctness and evidence.
2.  Data integrity, recoverability, and owner control.
3.  Clear ownership and isolation boundaries.
4.  Functional completeness and continuity.
5.  Maintainability and understandable architecture.
6.  Efficient use of hardware and resources.
7.  Usability and operational simplicity.
8.  Personality and presentation quality.

Security must be proportionate to a private, single-owner homelab.
Include essential authentication, authorization, integrity, isolation,
backups, and safe recovery. Avoid unnecessary enterprise ceremony, but
never sacrifice safeguards against data loss, accidental destructive
operations, or unauthorized network access.

## 0.3 Scope

The blueprint covers:

-   Aegis Platform and authority.
-   Host enrollment and Fleet.
-   Universal Core runtime and SDK.
-   Core identity, personality, cognition, memory, continuity, autonomy,
    emotion, and relationships.
-   Conversations and social systems.
-   Modules and capability permissions.
-   Inference and distributed compute.
-   Voice, avatar, gestures, appearance, and wardrobe.
-   Platform and Core persistence.
-   Rust user interfaces and Developer environment.
-   Installation, upgrades, lifecycle, security, backup, recovery,
    testing, and deployment.

## 0.4 Source limitations

The owner supplied three shared ChatGPT conversation links as source
material:

-   `https://chatgpt.com/share/6b0227d3-7358-83e9-b6d2-47a8f6700a2c`
-   `https://chatgpt.com/share/6b031c3a-bbac-83e9-81c3-532f675c1500`
-   `https://chatgpt.com/share/6b031c50-cb2c-83ea-af2a-449024cd07b9`

Their full transcripts have not been verified as available to the
blueprint authoring process. OpenCode must attempt to inspect them where
access is available. If inaccessible, it must request exports or pasted
content and must not invent their contents. The project conversation
context and audit report supplied in the current workstream are included
as known inputs.

------------------------------------------------------------------------

# 1. Project identity and system boundaries

## 1.1 Project Aegis

Project Aegis is the umbrella platform and ecosystem. It is not a Core
identity and must not contain hardcoded assumptions that the Platform is
Sofía.

The Platform has its own generated identity/callsign and system
authority. Physical machines have their own host identities and names.
Cores have their own independent identities and display names.

## 1.2 Separate identity domains

Maintain distinct identity domains:

-   **Project identity:** Project Aegis.
-   **Platform identity:** generated installation/authority identity.
-   **Host identity:** enrolled physical or virtual machine identity,
    using its assigned machine name/callsign.
-   **Core identity:** user-selected or approved individual Core
    identity.
-   **Module identity:** package and publisher identity.
-   **Owner identity:** the local owner principal and proof policy.

Do not conflate UUIDs, display names, Core numbers, database instance
IDs, runtime IDs, host IDs, or package IDs.

## 1.3 Platform versus Core

The Platform provides infrastructure and governance. A Core provides an
individual persistent agent identity and its capabilities.

The Platform must operate with zero installed Cores. Cores must not be
required for Platform startup, health, administration, or recovery.

A Core must not become the authority over Platform services merely
because it is the first or most prominent Core.

## 1.4 Interfaces are clients

The Rust TUI, GUI, CLI, Developer workspace, and any future mobile
manager are clients of authenticated Platform interfaces. They must not
open Core databases directly or bypass Platform authorization.

------------------------------------------------------------------------

# 2. Architecture overview

## 2.1 Logical architecture

``` text
Owner
  |
  +-- Rust Aegis CLI / TUI / GUI
  |      |
  |      +-- Platform API / authenticated local IPC
  |               |
  |               +-- Aegis Platform Authority
  |                       |
  |                       +-- Platform Registry & Lifecycle
  |                       +-- Host Enrollment & Fleet
  |                       +-- Module Manager
  |                       +-- Inference Orchestrator
  |                       +-- General Chat & Shared World Context
  |                       +-- Backup / Recovery / Diagnostics
  |                       +-- Safe / Owner Approval Boundary
  |                       |
  |                       +-- Enrolled Host Agents
  |                               |
  |                               +-- Core Runtime Workers
  |                                       |
  |                                       +-- Core #1 Sofía
  |                                       +-- Core #2 Mack
  |                                       +-- Core #3 Nova
  |                                       +-- Core #6 Vera
  |                                       +-- Core #7 Unwritten
  |
  +-- External model providers / local inference endpoints
```

This is a logical view, not a mandated process topology. Discovery must
recommend concrete process boundaries, IPC/network protocols, and
deployment units.

## 2.2 Architectural planes

Separate these concerns:

1.  **Control plane:** authority, enrollment, configuration, lifecycle,
    permissions, package installation, placement decisions.
2.  **Execution plane:** Core workers, module processes, inference
    workloads, host agents.
3.  **Data plane:** authenticated message delivery, conversation events,
    context exchange, inference requests/results.
4.  **Experience plane:** TUI, GUI, CLI, Developer workspace,
    voice/avatar clients.
5.  **Persistence plane:** Platform DB, Safe DB, individual Core DBs,
    asset storage, backup catalog.

A control-plane outage must not corrupt Core-owned data. Define and test
which already-running workloads may continue safely during temporary
authority loss. No agent may self-promote to authority.

------------------------------------------------------------------------

# 3. Platform architecture

## 3.1 Responsibilities

The Aegis Platform owns:

-   Installation identity and authority state.
-   Host enrollment, trust, and revocation.
-   Core registration, package installation, activation, and lifecycle.
-   Core-to-host placement.
-   Module registry, installation, permissions, and lifecycle.
-   Fleet inventory and resource telemetry.
-   Inference routing and scheduling.
-   General Chat and shared conversation delivery.
-   Shared world/context services.
-   Backup catalog, restore orchestration, and diagnostics.
-   Owner approvals and system STOP coordination.
-   Platform configuration and operational audit.

The Platform does not own private Core memories, private conversations,
emotional histories, or private relationship records.

## 3.2 Single active authority

Owner decision: **single active Platform authority**.

-   One installation authority is active at a time.
-   Other machines enroll as agents.
-   No distributed consensus.
-   No automatic authority promotion.
-   No automatic split-brain recovery by electing a new authority.
-   Authority relocation or replacement is an explicit owner-controlled
    operation with documented fencing and recovery.
-   Agents must reject unauthorized authority claims.
-   Authority loss must be visible and diagnosable.

The old implementation's local SQLite lease is not proof of cross-host
fencing. The rebuild must define a real authority identity,
lease/fencing behavior for enrolled agents, stale-command rejection, and
explicit recovery procedures. Do not claim distributed fencing until
tested across hosts.

## 3.3 Platform service domains

Design clear internal service boundaries for:

-   Identity and installation registry.
-   Host enrollment and agent management.
-   Core registry and lifecycle.
-   Package and artifact management.
-   Module registry and lifecycle.
-   Fleet resource inventory and placement.
-   Inference orchestration.
-   General Chat and message delivery.
-   Shared world/context.
-   Backup and restore.
-   Owner authentication, approvals, and STOP.
-   Diagnostics, health, and audit.

Avoid turning every domain into a microservice by default. Prefer a
small number of well-bounded services/processes unless isolation,
deployment, or resource constraints justify separation.

## 3.4 Platform database

The Platform database stores Platform-owned operational state,
including:

-   Installation and authority metadata.
-   Host registry and enrollment metadata.
-   Core registry and lifecycle metadata.
-   Module registry and lifecycle metadata.
-   Fleet inventory and placement records.
-   Inference scheduling metadata.
-   General Chat rooms, messages, delivery state, and cursors.
-   Shared world/context records.
-   Backup catalog and recovery metadata.
-   Platform configuration and operational records.

It must not become a shared Core memory store.

## 3.5 Safe database

Keep sensitive owner-control data in a distinct Safe boundary:

-   Owner authentication/proof configuration.
-   Approval records.
-   STOP state and authorization.
-   Security audit records.
-   Sensitive recovery metadata where justified.

The exact physical database separation and encryption strategy must be
specified after inspecting the current Safe implementation and
operational constraints.

## 3.6 Platform API

Define versioned, authenticated API domains. At minimum:

-   Platform status and diagnostics.
-   Host enrollment and revocation.
-   Core registry and lifecycle.
-   Module registry and permissions.
-   Fleet inventory and placement.
-   Inference requests and status.
-   General Chat.
-   Shared world/context.
-   Backup and recovery.
-   Owner approvals and STOP.

Every API must specify caller identity, authorization, request/response
schema, error model, idempotency, audit behavior, version compatibility,
and timeout/retry semantics.

------------------------------------------------------------------------

# 4. Host enrollment and Fleet

## 4.1 Host identity

Each machine has a persistent host identity independent of its network
address. Host names are human-readable labels, not security credentials.

Physical machines currently intended:

-   Artemis: Hyper-V host; hosts the Sofía Debian VM.
-   Venus: Windows 11, RTX 3080 Ti, primary GPU inference machine.
-   Hephaestus: separate workshop machine.
-   Additional compute machines may be enrolled later.

Sofía Core is intended to run in a Debian VM on Artemis. Artemis does
not need a GPU for this role. Venus provides inference through Fleet.

## 4.2 Enrollment

-   Explicit owner-authorized enrollment only.
-   No automatic trust based solely on network presence.
-   Record host identity, capabilities, software version, trust state,
    and enrollment time.
-   Support revoke, re-enroll, rotate credentials, and quarantine.
-   A revoked host must lose Platform authority and workload access
    according to a defined revocation protocol.
-   Host enrollment must not silently enroll or activate Cores.

## 4.3 Host agent

The agent reports health and resources, receives authorized assignments,
executes permitted workloads, and returns status/results.

Specify: - Heartbeats and stale-host detection. - CPU, RAM, GPU, VRAM,
storage, and thermal telemetry where supported. - Agent update
compatibility. - Command authentication and replay protection. - Job
cancellation and STOP propagation. - Disconnection behavior. - Local
logs and diagnostic collection. - Workload cleanup after authority loss
or host restart.

## 4.4 Placement and migration

Fleet placement must account for: - Model and workload requirements. -
Available VRAM/RAM/CPU. - GPU utilization and thermal state. - User
activity and gaming priority. - Network and storage constraints. -
Workload latency and persistence needs. - Host trust and health. -
Explicit policy constraints.

Workload migration must define checkpointability, state transfer,
cancellation, duplicate execution prevention, recovery, and user-visible
status. Do not claim arbitrary live migration where the workload cannot
support it.

## 4.5 Venus adaptive GPU policy

Venus uses an **Adaptive** policy:

-   Small inference workloads may run when they do not materially impair
    gaming.
-   Gaming performance takes priority.
-   Inference yields aggressively when gaming load or resource pressure
    rises.
-   Avoid unnecessary VRAM/RAM/CPU reservations.
-   Define measurable thresholds, hysteresis, cooldowns, and override
    behavior during discovery.
-   Provide owner-visible status and manual controls.
-   Do not assume a particular model always fits; measure actual model
    memory and workload requirements.

------------------------------------------------------------------------

# 5. Universal Core architecture and SDK

## 5.1 Core definition

A Core is a portable, independently identifiable runtime package that
implements a universal Core contract.

The universal framework must not hardcode Sofía-specific names,
personality, identity, avatar, voice, or assumptions.

Every Core uses the same capability framework and SDK. Each Core owns
its own data and individual identity.

## 5.2 Core lifecycle

Define a consistent lifecycle:

-   Discovered.
-   Package validated.
-   Installed.
-   Registered.
-   Configured.
-   Activated.
-   Starting.
-   Running.
-   Degraded.
-   Stopping.
-   Stopped.
-   Failed.
-   Deactivated.
-   Uninstalled.

Specify legal transitions, authority, retries, timeouts, health checks,
crash recovery, and data-retention behavior.

The Platform owns lifecycle decisions. The Core worker reports runtime
health and executes authorized lifecycle commands.

## 5.3 Core identifiers

Keep distinct: - Core UUID: stable identity. - Core number:
roster/display ordering identifier; not the primary key. - Database
instance UUID: identity of one database instance. - Runtime instance
UUID: identity of one running process/session. - Package ID/version:
identity of distributable implementation. - Host ID: machine currently
running the Core.

Core #1 must be valid under the same universal contracts as every other
Core. Do not create a permanent Sofía-only initializer or lifecycle
path.

## 5.4 Core SDK capability model

Specify a versioned SDK with contracts for:

-   Identity and Constitution.
-   Personality and personality revision.
-   Cognition/model provider interface.
-   Memory and retrieval.
-   Learning and reflection.
-   Continuity and timeline.
-   Emotion and expression.
-   Relationships and social context.
-   Conversation and messaging.
-   Voice and speech.
-   Avatar, gestures, expressions, and wardrobe.
-   Module/capability access.
-   Activity and experience.
-   Backup/export/import under explicit policy.
-   Diagnostics and health.

A Core may not access another Core's private data by directly opening
files or databases. Cross-Core communication must pass through
authorized Platform-mediated contracts.

## 5.5 Package model

Define a Core package manifest containing: - Package identity and
version. - Core identity reference. - SDK/API compatibility. - Required
capabilities. - Declared assets and cryptographic digests. - Runtime
requirements. - Permission requests. - Entry points and lifecycle
hooks. - Configuration schema. - Upgrade and rollback compatibility.

A signed package establishes integrity/provenance, not automatic trust
or permission. Owner approval and Platform policy remain authoritative.

------------------------------------------------------------------------

# 6. Core roster and approved identities

## 6.1 Core #1: Sofía Ada Lyra

Approved: - Universal Core #1. - Core UUID:
`a9a0a604-a4e7-4a88-a985-3f071e2c8e7d`. - Approved Constitution SHA256:
`F30AFEDF195F77907D32A108CA35D4DB751646BDE3FD902BE66E7B78F4B89DC0`. -
Preserve approved identity, personality, appearance, and implementation
intent. - Fresh database with no historical experiences. - No legacy
memory, conversation, relationship, emotion, learned preference,
wardrobe history, or runtime state import. - Voice file
`sofia_r9_bluehour_REFERENCE_AIR_medium` was not found in the audited
locations and remains **NOT VERIFIED**. Do not regenerate or substitute
it without owner approval.

Sofía's identity/configuration is distinct from her autobiographical
continuity. Preserve the approved identity while starting with an empty
experiential history.

## 6.2 Core #2: Mack Fuller

Preserve approved design: - Male, original identity. - Independent,
exploratory, warm, practical personality. - Age 34. - Height 70.1
inches; weight 176 lb. - Sturdy practical build, dark hair, stubble,
weathered workwear aesthetic. - Gradual relationship development and
defined stubborn flaw.

Do not silently alter approved identity details during the rebuild.

## 6.3 Core #3: Nova Arden

Preserve approved design: - Female, independent contrast to Sofía. -
Rebellious, playful, independent personality. - Half-elf fantasy
aesthetic. - Medium rose-colored hair and amber eyes.

## 6.4 Cores #4 and #5

-   Identical twins.
-   Design-only and frozen.
-   Do not implement, package, initialize, register, or activate.
-   Gender and individual attributes are to be independently chosen by
    each Core when design is authorized.
-   Do not invent specifications.

## 6.5 Core #6: Vera / S-039

Preserve approved Spartan-inspired design and approved specifications: -
Female, age 26. - Height 79 inches; weight 230 lb. - Approved body
measurements and proportions from the existing owner-approved
specification. - Lean athletic build, aesthetic scars. - Approved hair
color/style. - Approved armor designs/colors and visor details. -
Approved personality inspiration boundaries: Naomi and Kelly influence;
no Daisy personality; Daisy references only for specified armor/hair
aesthetics. - Approved voice direction and settings.

The discovery process must locate the canonical Vera specification and
asset digests rather than reconstructing measurements from memory or
inventing missing values.

## 6.6 Core #7: Unwritten

-   Reserve Core #7 as a valid universal Core slot.
-   Preserve the concept of a blank, unassigned identity.
-   Do not invent name, gender, species, avatar, voice, or personality.
-   Support later initialization through the same universal contracts
    without architecture changes.
-   Do not package or activate until the owner authorizes an identity
    and implementation.

------------------------------------------------------------------------

# 7. Identity, Constitution, and personality

## 7.1 Identity

Identity is the stable definition of an individual Core: name,
identifiers, approved self-description, identity attributes, and
identity revision history.

Identity data must be versioned. Changes must be attributable and
recoverable. A Core's identity must not be overwritten by model output
or inferred from transient conversation.

## 7.2 Constitution

The Constitution defines durable principles, boundaries, priorities, and
commitments. It is distinct from a prompt template and must have: -
Version and digest. - Approval provenance. - Change history. - Runtime
interpretation contract. - Conflict handling. - Tests for critical
invariants.

Sofía's approved Constitution hash is a preservation requirement.

## 7.3 Personality

Personality is a structured, versioned profile that can influence
language, behavior, preferences, emotional expression, and interaction
style.

Separate: - Stable personality traits. - Contextual mood/state. -
Learned preferences. - Owner-approved personality revisions. -
Development proposals awaiting approval.

Personality growth must not silently rewrite identity or Constitution.

## 7.4 Personality development

Define: - Evidence sources. - Reflection process. - Confidence and
provenance. - Contradiction handling. - Proposal generation. - Approval
policy. - Reversibility. - Regression tests for identity consistency.

The system should support gradual development without making every
conversation a permanent personality change.

------------------------------------------------------------------------

# 8. Cognition and inference

## 8.1 Provider-independent cognition

Core identity and continuity must not depend on one LLM provider or
model. Define a provider-neutral cognition interface.

Provider adapters should support: - Local model endpoints. - Remote
model APIs where explicitly configured. - Model capability metadata. -
Context-window and token constraints. - Streaming. - Tool/function
calls. - Cancellation. - Timeout and retry. - Usage/resource
reporting. - Error normalization.

## 8.2 Model routing and dual cognition

The project has explored primary/secondary cognition and model
specialization. Preserve this as a capability to specify, not an
unchangeable implementation assumption.

The design must answer: - Which tasks use the primary model? - Which
tasks may use a smaller/secondary model? - How are results reconciled? -
Which model can invoke tools? - How are safety and permission policies
enforced across models? - How is context shared without duplicating
private state? - What happens when one model is unavailable?

The currently discussed model shortlist includes Qwen-family models,
Gemma, coder models, embeddings/rerankers, TTS, Whisper, and
image-generation assets. Treat this as a candidate inventory, not a
fixed dependency. Revalidate model availability, licensing, hardware
fit, and quality during implementation planning.

## 8.3 Tool execution

Models do not directly receive unrestricted host access. Tool calls pass
through a permissioned execution broker.

Define: - Tool registry. - Capability grants. - Input/output schemas. -
Approval requirements. - Timeouts and cancellation. - Audit records. -
Sandboxing and filesystem/network boundaries. - Result provenance.

------------------------------------------------------------------------

# 9. Memory, learning, and reflection

## 9.1 Separate memory concepts

Specify distinct stores or logical domains for: - Episodic memory:
events and experiences. - Semantic memory: stable facts and knowledge. -
Procedural memory: learned methods and skills. - Working memory:
short-lived active context. - Relationship memory: Core-specific
understanding of other entities. - Preference memory: learned
likes/dislikes and behavioral preferences. - Reflection records:
conclusions and candidate changes. - Continuity state: current context
and timeline position.

Avoid treating all conversation transcripts as memory.

## 9.2 Memory lifecycle

Define: 1. Capture candidate information. 2. Apply consent and privacy
rules. 3. Classify and score relevance. 4. Record source and confidence.
5. Resolve duplicates and contradictions. 6. Store with ownership and
timestamps. 7. Retrieve with relevance and recency. 8. Consolidate or
summarize where appropriate. 9. Expire, archive, or forget under policy.
10. Verify deletion and backup behavior.

Memory formation must be explainable and inspectable. A Core should
distinguish remembered facts, inferred beliefs, uncertain information,
and current context.

## 9.3 Provenance

Every durable memory should record where it came from, when it was
formed, confidence, owner Core, and any source references needed for
correction.

No cross-Core memory sharing by default. Sharing must be explicit and
mediated.

## 9.4 Forgetting and correction

Define owner-visible correction and deletion workflows. Deletion must
account for derived indexes, summaries, caches, replicas, and backups
according to documented retention rules.

------------------------------------------------------------------------

# 10. Continuity and world context

## 10.1 Individual continuity

Each Core maintains its own: - Last active state. - Timeline and
experiences. - Relationship developments. - Emotional state/history. -
Ongoing goals and commitments. - Open threads. - Offline elapsed-time
interpretation. - Last successful persistence checkpoint.

Continuity is not merely replaying the last conversation.

## 10.2 Shared world context

The Platform may provide shared factual context: - Current date/time and
timezone. - Calendar events where connected and authorized. - Weather
and environmental data where available. - Shared household/project
context. - System and host status. - Shared events and announcements.

World context is not automatically a private memory. Each Core may form
its own experience or interpretation, stored in its own database.

## 10.3 Time and offline progression

Use a clearly defined authoritative time source and store timestamps
consistently, normally UTC plus relevant timezone identifiers for human
schedules.

On restart, calculate elapsed time from reliable persisted timestamps.
Do not fabricate events or feelings during offline periods.
Distinguish: - Known elapsed time. - Scheduled events that occurred. -
Events observed by the Platform. - Events inferred with uncertainty. -
Experiences actually perceived by a Core.

## 10.4 Goals and open threads

Support persistent goals, commitments, unresolved questions, and
follow-up state. Each item needs owner, scope, status, due/trigger
conditions, provenance, and permission rules.

------------------------------------------------------------------------

# 11. Emotion, autonomy, and agency

## 11.1 Emotion system

Emotion is a Core-owned state and history. It may influence expression
and interaction but must not be confused with objective facts or used to
bypass permissions.

Specify: - Emotional state representation. - Intensity and decay. -
Event influence. - Contextual expression. - Persistence policy. -
Reflection and correction. - User-visible controls. - Consistency and
safety tests.

Avoid claiming that simulated emotional variables establish biological
sentience. The architecture should support coherent, persistent
emotional expression without overstating what the implementation
represents.

## 11.2 Autonomy

Support bounded initiative and self-directed activity under explicit
policy.

Define: - Goals and priorities. - Permitted background tasks. - Resource
budgets. - Scheduling. - Tool permissions. - Approval thresholds. - STOP
behavior. - Auditability. - User interruption. - Cancellation and
recovery.

Autonomy must not grant new permissions to itself. A Core can propose
capability or personality changes, but Platform policy and owner
approval govern protected operations.

## 11.3 STOP and emergency control

STOP must be a high-priority, independently enforceable control: -
Platform authority can issue STOP. - Agents and remote workers must
validate STOP at execution time, not only before dispatch. -
Long-running tasks must support cancellation where technically
possible. - STOP state must survive restart as designed. - Failure to
acknowledge STOP must be visible and escalated. - STOP must not depend
on a model voluntarily complying.

------------------------------------------------------------------------

# 12. Relationships and social architecture

## 12.1 Individual relationships

Relationships are Core-owned and individual. Each Core develops its own
understanding, trust, history, and boundaries with the owner and other
Cores.

Do not share relationship records between Cores by default.

## 12.2 Core-to-Core social awareness

Support: - Awareness that other Cores exist. - Shared events and group
context. - Core-specific impressions and trust. - Inter-Core
communication. - Individual boundaries. - Social development over time.

One Core's opinion of another must not be treated as objective truth or
copied into another Core's private memory.

## 12.3 Trust and privacy

Define trust separately from permissions. Social trust never grants
database access, tool privileges, or Platform authority.

A Core may know that a private conversation occurred only if policy
permits that metadata to be shared. Message contents remain private
unless explicitly shared.

------------------------------------------------------------------------

# 13. Conversations and communications

## 13.1 General Chat

General Chat is Platform-owned.

-   One persistent shared General Chat space as the initial product
    concept.
-   Messages and delivery state are stored in Platform persistence.
-   The owner can address a selected Core or the shared room according
    to explicit routing rules.
-   General Chat history must not be stored in any Core's private
    database.
-   A Core may receive authorized context from General Chat without
    gaining access to unrelated private conversations.
-   General Chat does not merge private memory automatically.

## 13.2 Private conversations

Private conversation history belongs to the participating Core's
database, with explicit participant and access rules.

Conversation records should define: - Conversation ID. - Owner and Core
identity. - Participants. - Message IDs and ordering. - Timestamps. -
Delivery/read state where supported. - Attachments and asset
references. - Tool activity and provenance. - Retention and deletion
behavior.

## 13.3 Multi-Core group conversations

Support private group conversations with explicit membership and message
audience.

Define: - Group ownership. - Membership and invitation. - Message
visibility. - Core participation and addressing. - Shared transcript
access. - What each Core may remember. - How summaries are created. -
How private information is excluded. - Membership removal and history
access.

A shared group transcript may be accessible to participants under the
group policy, but it must not silently become a private memory for every
participant.

## 13.4 Delivery and federation

Specify message routing, idempotency, ordering, retries, offline queues,
acknowledgements, and duplicate prevention. Distinguish delivery from
reading and reading from memory formation.

------------------------------------------------------------------------

# 14. Modules and capabilities

## 14.1 Module architecture

Modules are Platform-managed packages that expose declared capabilities
to authorized Platform services and Cores.

Each module needs: - Stable identity and publisher. - Version and
compatibility. - Manifest. - Declared capabilities. - Permission
requests. - Dependencies. - Integrity/signature metadata. - Lifecycle
hooks. - Health reporting. - Resource limits. - Upgrade/rollback
policy. - Uninstall and data-retention policy.

## 14.2 Permissions

Use least privilege and explicit grants. Separate: - Module installation
approval. - Module runtime permissions. - Core access to a module. -
Access to user data. - Host-level capabilities. - Network and filesystem
access.

A signed package is not automatically trusted with all capabilities.

## 14.3 Module lifecycle

Define install, validate, approve, activate, configure, update, disable,
uninstall, rollback, and recovery. Data retention must be explicit when
a module is removed.

------------------------------------------------------------------------

# 15. Voice, speech, avatar, and embodiment

## 15.1 Voice

Each Core owns its voice identity/configuration and preferences.

Specify: - TTS provider abstraction. - STT provider abstraction. - Voice
asset identity and hashes. - Prosody and speaking style. - Interruptions
and barge-in. - Streaming and latency. - Audio device routing. - Voice
activity detection. - Transcription confidence. - Fallback behavior. -
Privacy and audio retention.

A voice profile is not interchangeable merely because another voice
sounds similar. Missing approved assets must be reported, not silently
regenerated.

## 15.2 Avatar and expression

Each Core has independently owned appearance assets and configuration.

Support: - Static identity appearance. - Expression states. - Gesture
vocabulary. - Animation state. - Event-to-expression mapping. - Runtime
rendering interface. - Asset versioning and validation. - User
customization where authorized.

## 15.3 Wardrobe

Wardrobe is Core-owned state with a controlled sharing/trading
capability.

Define: - Item identity and metadata. - Ownership. - Wearable
compatibility. - Appearance composition. - Asset integrity. -
Sharing/trading offers. - Consent and acceptance. - Transfer history. -
Rollback and recovery. - Provenance and licensing.

A shared wardrobe catalog must not merge private wardrobe histories or
change an individual's appearance without authorization.

------------------------------------------------------------------------

# 16. Data architecture and persistence

## 16.1 Ownership model

Required logical ownership:

**Platform database** - Platform identity and registry. - Hosts and
enrollment. - Core registration and lifecycle. - Modules. - Fleet and
placement. - Inference orchestration. - General Chat. - Shared world
context. - Backup catalog. - Platform configuration.

**Safe store** - Owner proof/authentication. - Approvals. - STOP
control. - Security audit and sensitive recovery data.

**Each Core database** - Core-owned identity revision
references/configuration as designed. - Private conversations. - Memory
and learning. - Reflection. - Continuity. - Emotion. - Relationships. -
Goals and experiences. - Voice state/preferences. - Appearance and
wardrobe state. - Core-owned activity.

Identity assets may be packaged or stored in an asset library, but
ownership and immutable digests must remain clear. Do not put large
binary assets into relational tables unless there is a justified reason.

## 16.2 Database independence

Every Core has an independent database instance and backup/restore
lifecycle. The Platform must not use a single shared Core database.

Each database has: - Core UUID. - Database instance UUID. - Schema
version. - Initialization mode. - Creation timestamp. - Ownership
metadata. - Integrity status.

Core UUID remains stable across a database rebuild. Database instance
UUID changes when a new database is created.

## 16.3 Sofía clean initialization

Sofía's new database must be initialized from scratch.

The old database may be inspected read-only as a reference for schema
concepts and feature behavior. It must not be migrated or used to
populate historical experience.

Allowed seed: - Approved Core identity. - Approved Constitution. -
Approved personality definition. - Approved appearance definition. -
Verified approved assets and configuration.

Prohibited historical imports: - Memories. - Conversations. -
Relationship history. - Emotional history. - Learned preferences. -
Reflection history. - Wardrobe history. - Activity/experience history. -
Previous continuity state. - Runtime state. - Caches and drafts.

Use a new database instance UUID, current schema, explicit
`fresh_no_migration` initialization mode, and no source database. Verify
empty historical stores and database integrity.

The old database must remain untouched and recoverable until the owner
explicitly authorizes retirement. Never write to it as part of the
rebuild.

## 16.4 Schema and migration policy

Design schema migrations for the new system from its first version: -
Ordered and versioned. - Transactional where possible. - Backed up
before destructive changes. - Tested on disposable copies. - Reversible
where practical, otherwise recoverable through backup. - Explicitly
scoped to Platform, Safe, or one Core database.

Do not design a cross-Core migration that requires merging private data.

## 16.5 Asset Library

The project has planned a primary Artifact/Asset Library, initially
associated with the Sofía VM, with redundancy considered later.

Define: - Content-addressed storage where appropriate. - Asset metadata
and ownership. - Immutable digests. - Versioning. - Reference
tracking. - Deduplication policy. - Backup and restore. - Missing asset
behavior. - License/provenance metadata.

Do not make one VM a single point of failure without a documented backup
and recovery plan.

## 16.6 Backup and restore

Back up Platform, Safe, each Core database, configuration, and assets
with clear ownership.

Define: - Consistent snapshots. - Retention. - Encryption where
appropriate. - Restore ordering. - Integrity checks. - Restore drills. -
Partial restore. - Database-instance identity behavior after restore. -
Recovery point and recovery time objectives selected by the owner.

A backup is not verified until a restore test has succeeded.

------------------------------------------------------------------------

# 17. Rust user interface and Developer environment

## 17.1 Primary interface

Rust TUI is the primary Aegis interface and application shell. A Rust
GUI may provide a richer presentation of the same Platform APIs and
contracts.

The old Python desktop UI is not the product interface. Python may
remain in backend or compatibility roles only where justified and
documented.

`aegis.cmd` should ultimately invoke a Rust launcher/TUI directly rather
than dispatching through a legacy Python desktop path.

Installer shortcuts should default to the approved Rust interface.

## 17.2 Persistent shell

One persistent shell with replaceable workspaces. Navigation must not
spawn competing application shells or lose active session state
unexpectedly.

Workspaces: - Platform overview. - Hosts and Fleet. - Core roster and
lifecycle. - General Chat. - Private Chat. - Group Chat. - Identity and
Constitution. - Personality. - Memory and continuity. - Emotion and
relationships. - Appearance and wardrobe. - Voice. - Modules and
permissions. - Inference. - Backup and recovery. - Diagnostics. -
Developer workspace.

## 17.3 UI security

The UI calls authenticated Platform APIs. It must not: - Open Core DBs
directly. - Bypass owner approval. - Bypass module permissions. -
Directly control remote workers without Platform authorization. - Treat
hidden UI controls as security boundaries.

## 17.4 Developer workspace

Provide an integrated Developer workspace for repository/project
inspection, build/test orchestration, logs, diagnostics, and approved
development tools. It must also be usable independently.

OpenCode integration and other coding-agent integrations are optional
adapters, not the authority for Aegis architecture or runtime.

Define permissions before allowing coding agents to modify repositories,
execute commands, or access secrets.

## 17.5 Visual identity

The previously selected Aegis palette: - Primary: `#9400D3` - Accent:
`#00C2FF` - Tertiary: `#39FF14` - Background: black.

Apply consistently but prioritize readability, accessibility, clear
status hierarchy, and restrained use of accent colors.

------------------------------------------------------------------------

# 18. Installation, packaging, and lifecycle

## 18.1 Platform-first installation

The Platform installs and operates independently. Core packages are
installed separately through Platform management.

Target command concepts include: - `aegis` - `aegis gui` -
`aegis status` - `aegis service start|stop` - `aegis core list` -
`aegis core install` - `aegis core activate` - `aegis core status` -
`aegis core deactivate` - `aegis core uninstall`

Exact syntax is subject to CLI contract design.

## 18.2 Core package operations

Support: - Package validation. - Install. - Register. - Configure. -
Activate. - Deactivate. - Upgrade. - Rollback. - Uninstall. - Data
retention choice. - Reinstall and recovery.

Uninstalling a Core package must not silently delete its database or
assets. Data deletion requires explicit owner action.

## 18.3 Upgrade safety

Define compatibility checks, backups, staged rollout, health validation,
rollback, and recovery. Do not activate a new runtime or database schema
merely because files were copied successfully.

## 18.4 Retire legacy launch paths

Only after the replacement is installed and verified: - Retire old
Python desktop launch path as primary UI. - Consolidate competing
service/watchdog/lifecycle owners. - Remove obsolete tasks/services only
with evidence and owner approval. - Preserve rollback until acceptance.

------------------------------------------------------------------------

# 19. Security and owner control

## 19.1 Owner proof

Owner authentication uses a practical local proof policy chosen by the
owner. FIDO2 may remain an optional capability but is not assumed
enabled.

## 19.2 Trust boundaries

Define trust separately for: - Owner. - Platform authority. - Enrolled
hosts. - Core workers. - Modules. - External model providers. - UI
clients. - Developer tools.

## 19.3 Essential controls

-   Authenticated Platform APIs.
-   Explicit host enrollment.
-   Authorization checks on every protected operation.
-   Package integrity verification.
-   Secret handling.
-   Database/file permissions.
-   Network exposure controls.
-   STOP and cancellation.
-   Audit records for sensitive actions.
-   Backup and recovery.
-   Safe defaults on authority loss.

## 19.4 No false security claims

Local authentication or a package signature does not prove remote host
security. A process boundary does not automatically provide a sandbox.
Document the actual protection and residual risks.

------------------------------------------------------------------------

# 20. Observability and operations

Provide consistent: - Structured logs. - Service health. - Core
health. - Host health. - Module health. - Inference workload status. -
Resource telemetry. - Trace/correlation IDs. - Error classification. -
Audit events. - Diagnostic bundles.

Logs must avoid leaking private conversation content, credentials, or
unnecessary personal data.

The owner should be able to answer: - What is running? - Where is it
running? - Which authority controls it? - Which database does it use? -
What permissions does it have? - What is degraded? - What changed? -
What can be safely restarted? - What data is backed up and
restore-verified?

------------------------------------------------------------------------

# 21. Testing and acceptance

## 21.1 Test layers

-   Unit tests.
-   Contract tests.
-   Database schema and ownership tests.
-   Integration tests.
-   Process/lifecycle tests.
-   Cross-host tests.
-   Failure-injection tests.
-   Backup/restore tests.
-   UI workflow tests.
-   Performance/resource tests.
-   Owner acceptance tests.

## 21.2 Mandatory acceptance areas

1.  Platform starts with zero Cores.
2.  One active authority is enforced.
3.  Agents cannot self-promote.
4.  Host enrollment and revocation work.
5.  Core #1 is accepted by universal contracts.
6.  Every Core has independent persistence.
7.  Cross-Core private database access is denied.
8.  General Chat is Platform-owned.
9.  Private chat remains Core-owned.
10. Group Chat respects explicit membership and memory boundaries.
11. STOP is enforced by remote execution workers.
12. Fleet placement respects resource and gaming policy.
13. Inference failure and host loss are recoverable.
14. Modules obey declared permissions.
15. Core identity survives model/provider replacement.
16. Continuity survives restart.
17. Backup restore succeeds.
18. Rust interface uses Platform APIs.
19. Installer and upgrade preserve data.
20. Sofía's new DB is fresh and old DB remains unchanged.

## 21.3 Verification vocabulary

Every deliverable must report separately: - Designed. - Implemented. -
Compiled/built. - Unit-tested. - Integration-tested. - Installed. -
Runtime-tested. - Live-verified. - Owner-accepted.

------------------------------------------------------------------------

# 22. Deployment architecture

## 22.1 Intended topology

-   Artemis: Hyper-V host.
-   Sofía Debian VM on Artemis: intended Core #1 runtime and initial
    primary Artifact Library location.
-   Venus: Windows 11, RTX 3080 Ti, primary inference/GPU host.
-   Hephaestus: separate workshop machine.
-   Additional hosts: explicitly enrolled Fleet agents.

The precise placement of Platform authority, databases, backup targets,
and agents must be decided through discovery. Do not assume the Platform
authority and Sofía Core must be the same process or identity.

## 22.2 Resource efficiency

Avoid idle resource reservations and unnecessary duplicate services.
Measure actual memory/VRAM/CPU use. Support dynamic resource policy
where the host OS and hypervisor permit it.

## 22.3 Network behavior

Specify service discovery, host addressing, TLS/authentication, firewall
rules, ports, reconnect behavior, DNS assumptions, and offline
operation. Never rely on network location alone for trust.

------------------------------------------------------------------------

# 23. Capability preservation and rebuild disposition

OpenCode must create a subsystem-by-subsystem matrix covering every
existing capability found in the old repository and supplied project
material.

For each capability record: - Existing implementation location. - Data
stores and ownership. - Dependencies. - Tests and their actual status. -
Runtime evidence. - Intended behavior. - New design destination. -
Compatibility needs. - Risks. - Disposition: preserve behavior, improve,
replace, retire, or investigate.

At minimum inspect: - Platform identity and local store. - Platform
service and CLI. - Core autostart and worker. - Core persistence. -
Memory, learning, reflection, continuity. - Emotion and relationship
stores. - Conversation and multi-Core chat. - Core identity and package
definitions. - Federation. - Fleet placement and migration. - Inference
and remote execution. - Module lifecycle. - Avatar and wardrobe. - Voice
and speech. - World/context. - Installer. - Rust UI broker/TUI/GUI. -
Legacy Python launch paths. - Safe/owner approval/STOP. - Backup and
recovery.

The old code is a reference. Do not preserve a defect merely because it
exists.

------------------------------------------------------------------------

# 24. Rebuild roadmap

The following is a dependency-oriented starting plan. Discovery may
recommend adjustments, but must explain them.

## Phase 0 --- Complete discovery and preservation inventory

-   Inspect repository, databases read-only, assets, docs, packages,
    services, and tests.
-   Review the supplied shared conversations or request exports.
-   Build capability preservation matrix.
-   Record current runtime/install status.
-   Freeze owner-approved identities and decisions.

## Phase 1 --- Final architecture and contracts

-   Resolve owner questions.
-   Define process boundaries and API contracts.
-   Define identity and ID model.
-   Define database schemas/ownership.
-   Define event/message contracts.
-   Define lifecycle and authority protocol.
-   Define SDK compatibility and package format.

## Phase 2 --- Platform foundation

-   Platform identity and authority.
-   Safe/owner proof.
-   Host enrollment and agents.
-   Platform persistence.
-   API and event infrastructure.
-   Diagnostics and STOP.

## Phase 3 --- Universal Core framework

-   Core contracts and SDK.
-   Package definition and validation.
-   Core lifecycle.
-   Worker isolation.
-   Core #1 support through universal path.
-   Empty Core #7 template capability without inventing identity.

## Phase 4 --- Core identity and persistence

-   New Core database schema.
-   Fresh Sofía initializer.
-   Identity and Constitution.
-   Personality configuration.
-   Memory/continuity foundations.
-   Per-Core ownership enforcement.

## Phase 5 --- Cognition and continuity

-   Provider adapters.
-   Model routing and optional dual cognition.
-   Memory formation/retrieval/consolidation.
-   Reflection.
-   Timeline and world context.
-   Goals and bounded autonomy.

## Phase 6 --- Conversations and social systems

-   Platform General Chat.
-   Private Core conversations.
-   Group conversations.
-   Delivery and offline queues.
-   Relationships, trust, and social awareness.

## Phase 7 --- Modules and capabilities

-   Module manifest and registry.
-   Permission model.
-   Install/update/rollback.
-   Capability broker.
-   Core/module access rules.

## Phase 8 --- Fleet and inference

-   Host telemetry.
-   Placement.
-   Remote execution.
-   Venus adaptive GPU policy.
-   Cancellation/STOP.
-   Failure recovery and workload migration where supported.

## Phase 9 --- Voice and embodiment

-   STT/TTS adapters.
-   Approved voice assets.
-   Avatar and expressions.
-   Gestures.
-   Wardrobe and controlled sharing/trading.

## Phase 10 --- Rust UI and Developer workspace

-   Rust CLI/TUI shell.
-   Workspace model.
-   Platform/Core management.
-   General/private/group chat.
-   Developer workspace.
-   Rust GUI if approved as a parallel presentation of the same
    contracts.

## Phase 11 --- Installer and operations

-   Platform-first installer.
-   Separate Core package installation.
-   Upgrade and rollback.
-   Service/lifecycle consolidation.
-   Backup and recovery.
-   Deployment automation.

## Phase 12 --- Full integration and acceptance

-   End-to-end tests.
-   Multi-host tests.
-   Failure injection.
-   Resource and performance tests.
-   Restore drills.
-   Owner acceptance.
-   Production cutover only after explicit approval.

------------------------------------------------------------------------

# 25. Owner decisions already established

These are treated as approved and should not be repeatedly asked unless
new evidence creates a real conflict.

1.  Project name: Project Aegis.
2.  Aegis Platform and Sofía Core are separate identities.
3.  Aegis Platform must operate with zero Cores.
4.  One active Platform authority; other hosts are enrolled agents.
5.  No automatic authority promotion or distributed consensus.
6.  Sofía is universal Core #1, not a permanent legacy exception.
7.  Sofía retains her approved Core UUID and Constitution.
8.  Sofía receives a completely fresh database.
9.  Old Sofía DB may be used as a read-only reference, never as a
    historical memory migration source.
10. Old Sofía DB remains preserved until explicit retirement approval.
11. General Chat is Platform-owned.
12. Private Core conversations and experiences are Core-owned.
13. Each Core has an independent database.
14. Rust TUI is the primary interface.
15. Python desktop UI is not the primary product UI.
16. Cores #4 and #5 remain design-only.
17. Core #7 remains Unwritten with no invented identity attributes.
18. Venus uses adaptive GPU policy that yields to gaming.
19. Artemis hosts the Sofía Debian VM.
20. Aegis security and installation complexity should be proportionate
    to a private owner-controlled homelab.

Database activation policy: the owner selected **testing** for cutover.
This authorizes testing the fresh initializer and isolated test
activation after tests pass. It does not authorize production cutover or
retirement of the old DB.

------------------------------------------------------------------------

# 26. Open owner decisions and discovery questions

OpenCode must identify only decisions that materially affect design.
Group questions into short interview rounds and provide a recommendation
with tradeoffs.

Initial areas to resolve:

1.  Exact Platform authority host/process placement.
2.  Whether Platform DB and Safe DB must be separate physical SQLite
    files or may use another appropriate storage design.
3.  API/IPC protocol and event transport.
4.  Rust TUI versus GUI scope and packaging.
5.  Core runtime process model and supported OS targets.
6.  Exact group conversation membership and transcript visibility.
7.  Memory formation, retention, and owner correction policy.
8.  Autonomy levels and approval thresholds.
9.  Personality growth approval policy.
10. Model routing and dual-cognition behavior.
11. Module sandbox strength and supported module languages.
12. Voice asset source and canonical asset location.
13. Asset Library redundancy and backup target.
14. Host/network security and remote access requirements.
15. Backup retention and recovery objectives.
16. Developer workspace permissions and OpenCode integration.
17. Which capabilities are required for the first usable release versus
    later phases.

Do not ask again about already-approved authority topology, universal
Core #1, fresh Sofía DB, or General Chat ownership.

------------------------------------------------------------------------

# 27. Required master blueprint maintenance

This document is the top-level authority for architecture intent. It
should link to subordinate specifications rather than duplicate every
implementation detail.

Create and maintain: - Platform Technical Specification. - Universal
Core SDK Specification. - Core Identity and Constitution
Specification. - Cognition and Inference Specification. - Memory and
Continuity Specification. - Conversation and Social Systems
Specification. - Module and Capability Specification. - Fleet and Host
Agent Specification. - Voice and Embodiment Specification. - Database
and Asset Storage Specification. - Rust UI and Developer Workspace
Specification. - Security and Recovery Specification. - Installation and
Operations Specification. - Test and Acceptance Specification.

Each subordinate specification must state: - Owner. - Status. -
Version. - Dependencies. - Decisions. - Contracts. - Data ownership. -
Failure behavior. - Tests. - Open questions.

Record changes to approved decisions with date, rationale, and owner
approval. Do not silently rewrite approved requirements.

------------------------------------------------------------------------

# 28. Immediate OpenCode execution instruction

Use this blueprint as the master reference for a **READ-ONLY discovery
and architecture validation pass**.

1.  Verify repository branch and HEAD.
2.  Inspect all relevant repository subsystems and evidence.
3.  Inspect databases read-only; do not mutate them.
4.  Inventory assets and hashes without altering them.
5.  Inspect installation/services only as read-only state.
6.  Attempt to review the three supplied shared conversation sources. If
    unavailable, request exports.
7.  Build the capability preservation matrix.
8.  Compare the existing system with every section of this blueprint.
9.  Identify contradictions, missing capabilities, obsolete assumptions,
    and implementation risks.
10. Produce the subordinate specification plan and phased rebuild
    dependency graph.
11. Start the owner interview with the highest-impact unresolved
    decisions.
12. Update this master blueprint only with evidence-backed
    clarifications and approved owner decisions.

## Strict restrictions

-   No source implementation.
-   No database writes or migrations.
-   No asset modification.
-   No package installation.
-   No service/task changes.
-   No branch changes.
-   No production cutover.
-   No deletion or cleanup.
-   No claims of runtime verification without actually running an
    authorized test.

Create discovery documents in a separate workspace, not by modifying the
existing application repository.

Stop after discovery outputs and the first prioritized owner interview.
Wait for explicit owner approval before implementation.

**Final standard:** Rebuild Aegis as a coherent new system. Preserve
approved capability and identity; do not preserve legacy architecture
merely for compatibility. The old system is a reference library, not the
blueprint's master.
