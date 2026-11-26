# Batch 7 — Agent 08: Artifact Library (Standalone Node + Live LAN Retrieval)

Status: implemented and verified in `aegis-embodiment` only. No commit, deploy,
reset, or destructive Git operation was performed. No Artemis or Sofia-VM
installation, credential, or deployment was used.

## Audit of the Batch 6 remote protocol

Inspected before changing anything:

- `crates/aegis-embodiment/src/remote.rs`
- `crates/aegis-embodiment/src/store.rs` (per-Core store, ownership, audit,
  `recovery_manifest`)
- `crates/aegis-embodiment/src/asset.rs` (asset contracts, bounds, manifest)
- `crates/aegis-embodiment/tests/remote.rs` (in-process `127.0.0.1` tests)
- `docs/implementation/BATCH6_AGENT08_ARTIFACT_LIBRARY.md`

The existing protocol already provides:

- 4-byte big-endian length framing; request/response header frames bounded by
  `MAX_FRAME_BYTES` (64 KiB) and the data frame by `MAX_ARTIFACT_FRAME_BYTES`
  (256 MiB, aligned with `MAX_ASSET_BYTES`).
- HMAC-SHA256 request proof over a domain-separated message bound to Core UUID,
  asset id, and nonce; verified with a constant-time comparison.
- HMAC-SHA256 signed response bound to status, Core, asset, nonce, SHA-256, and
  byte length; client checks the proof, nonce, Core/asset binding, bounds, and
  the SHA-256 of the received bytes.
- Server-side ownership checks: serves only the Core opened in its
  `EmbodimentStore`; another Core returns `WrongCore`, a non-owned/deleted/unknown
  asset returns `NotFound`, and no other Core's bytes are read.
- Read/write timeouts on both sides; typed `Truncated`, `Timeout`, and
  `FrameTooLarge` errors.
- `FetchedArtifact::save_to` writes to a temporary sibling, re-hashes, then
  renames; a mismatch removes the temporary and returns `IntegrityMismatch`.
- Read-only and idempotent: repeated fetches return identical bytes/digest and do
  not mutate the serving store.

No changes were made to `remote.rs`, `store.rs`, `asset.rs`, the SQLite schema,
`lib.rs`, or any other crate.

## Changes (additive only)

New files:

- `crates/aegis-embodiment/src/bin/artifact-node.rs` — standalone server binary.
- `crates/aegis-embodiment/src/bin/artifact-fetch.rs` — minimal client binary
  used for the live two-process retrieval.
- `crates/aegis-embodiment/tests/artifact_node.rs` — spawned-binary integration
  tests plus an explicitly-ignored live LAN test.
- `docs/implementation/BATCH7_AGENT08_ARTIFACT.md` — this document.

No new dependencies. `sha2`/`uuid`/`serde_json` were already used by the crate.

### `artifact-node` (server)

```text
artifact-node <bind-addr> <storage-root> <core-uuid>
```

- Opens the existing Core store at `<storage-root>` for `<core-uuid>` (the Core
  must already be initialized and own the assets).
- Reads the shared secret only from the `AEGIS_ARTIFACT_SECRET` environment
  variable. The secret is never accepted as an argument and never logged.
- Binds `<bind-addr>` (an ephemeral port such as `127.0.0.1:0` is allowed),
  prints exactly one banner line with the resolved address and Core UUID, then
  accepts connections and answers one authenticated request per connection with
  `RemoteArtifactServer::serve_once`.
- Exits non-zero on invalid arguments, missing/short secret, or store-open
  failure. Request-level errors are logged to stderr without the secret and do
  not stop the accept loop.

### `artifact-fetch` (client)

```text
artifact-fetch <server-addr> <core-uuid> <asset-id> <destination>
```

- Reads the same `AEGIS_ARTIFACT_SECRET` environment variable only.
- Fetches via `RemoteArtifactClient::fetch_to_path`, so the destination is
  written atomically only after digest verification.
- Prints the verified byte count, SHA-256, Core UUID, and asset id on success.

## Safety properties verified by tests

- **Atomic save**: destination is written via temp-file + re-hash + rename; a
  successful save leaves no `.tmp` sibling; a save whose in-memory digest does
  not match its bytes returns `IntegrityMismatch` and creates no file.
- **Duplicate idempotency**: the same authenticated request (same nonce) issued
  twice returns identical `FetchedArtifact` values; the on-disk asset is
  unchanged; after the node stops, the reopened store reports the same revision
  and a clean audit.
- **No partial file on interruption**: a real node response is truncated in
  transit by a test proxy; the client returns `Truncated` and the destination
  file does not exist.
- **No private-memory exposure**: the node only serves live assets owned by the
  served Core. Deleted assets, unknown asset ids, and another Core's assets all
  return `NotFound`; a request naming a different Core returns `WrongCore`.
  There is no endpoint for appearance state, events, audit records, arbitrary
  files, or another Core's data.

## Tests and verification

Commands run only for this crate (no whole-workspace gates):

```text
cargo test -p aegis-embodiment
cargo clippy -p aegis-embodiment --all-targets -- -D warnings
```

Results:

- `cargo test -p aegis-embodiment`: **73 passed, 0 failed, 1 ignored**.
  - New `tests/artifact_node.rs`: 5 passed, 1 ignored (the explicit live test).
  - Pre-existing crate tests: 68 passed.
  - Ignored: `live_lan_same_host_two_process_retrieval` (requires the host LAN
    address; run explicitly).
- `cargo clippy -p aegis-embodiment --all-targets -- -D warnings`: **clean**.

New integration tests:

- `spawned_node_serves_verified_artifact_and_is_idempotent`
- `spawned_node_denies_wrong_secret`
- `spawned_node_does_not_expose_deleted_unknown_or_foreign_assets`
- `truncated_transfer_from_real_node_writes_no_file`
- `atomic_save_never_leaves_partial_or_temporary_files`
- `live_lan_same_host_two_process_retrieval` (ignored by default)

## Live LAN same-host retrieval evidence

**Label: LAN same-host, two processes on one Windows machine
(`192.168.1.71`). This is NOT Artemis and NOT a cross-host replication run.**

Command:

```powershell
cargo test -p aegis-embodiment --test artifact_node live_lan_same_host_two_process_retrieval -- --ignored --nocapture
```

What ran:

1. A temporary Core store was initialized and a real 131072-byte artifact was
   imported (stored under the temp Core directory).
2. `artifact-node` was spawned as a child process bound to `192.168.1.71:0`
   with the secret supplied via `AEGIS_ARTIFACT_SECRET` (value redacted):
   `artifact-node 192.168.1.71:0 <temp-root> <core-uuid>`.
3. `artifact-fetch` was spawned as a second child process and fetched the
   artifact over the LAN interface:
   `artifact-fetch 192.168.1.71:52861 <core-uuid> <asset-id> <temp-destination>`.
4. The server and client processes were stopped and reaped by the test.

Observed result (from the run):

```text
artifact-fetch: fetched 131072 bytes sha256=feb1e4409d009e0ec502eaabe321f86b5197a881e9b765252ec8a75d6957596d core=a9a0a604-a4e7-4a88-a985-3f071e2c8e7d asset=edbae622-672f-4637-b90e-f8c0fbd433e9
live node=192.168.1.71:52861 asset=edbae622-672f-4637-b90e-f8c0fbd433e9 bytes=131072 sha256=feb1e4409d009e0ec502eaabe321f86b5197a881e9b765252ec8a75d6957596d
test live_lan_same_host_two_process_retrieval ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 1.21s
```

- Artifact size: **131072 bytes**.
- SHA-256 (server-announced, client-verified, and re-hashed from the restored
  file): `feb1e4409d009e0ec502eaabe321f86b5197a881e9b765252ec8a75d6957596d`.
- Bound address was the real LAN interface `192.168.1.71` (resolved port
  `52861`), not loopback.
- No `artifact-node` or `artifact-fetch` process was left running afterwards.

The automated tests separately use `127.0.0.1` ephemeral ports; the live test
above is the only case that binds the LAN address, and it runs both the server
and the client as real child processes.

## Explicit non-claims

- **No replication, synchronization, mirroring, tombstone propagation, conflict
  resolution, quorum, failover, or peer discovery.** The node answers read-only
  fetches for assets the Core already owns.
- **No real cross-host retrieval.** The live evidence is two processes on the
  same machine over its LAN interface. No second machine was involved.
- **No Artemis or Sofia-VM deployment, installation, or credentials.** None were
  used, and none are claimed.
- **No TLS/mTLS, per-Core asymmetric identity, key rotation, revocation, replay
  cache, or rate limiting.** Transport is plain TCP with a symmetric shared
  secret, suitable only for a trusted private LAN.
- **No automatic re-registration into a Core database.** A restore writes
  verified bytes only; re-importing is an explicit owner action.
- No changes to `crates/aegis-cli`, workspace `Cargo.toml`, `Cargo.lock`,
  `.gitignore`, `docs/completion`, or any other crate.

## Owner approval and next steps

Owner approval is required before this node is connected to any real deployment,
especially the Sofia VM or Artemis. Recommended sequence:

1. **Sofia VM placement (owner-approved)**: choose an explicit bind address and
   firewall rule, provision `AEGIS_ARTIFACT_SECRET` out of band, run under the
   service supervisor, and test only from an owner-authorized peer. Do not treat
   the same-host LAN evidence as VM or cross-host proof.
2. **TLS / authenticated channel**: terminate or wrap the protocol in TLS (or
   mTLS with per-Core certificates) before leaving a trusted LAN; add key
   provisioning and rotation.
3. **Replay and abuse protection**: add a bounded nonce cache, request rate
   limits, and per-transfer size/space quotas.
4. **Resumable transfer**: chunked transfer with per-chunk digests for large
   artifacts, keeping the current all-or-nothing verified write semantics.
5. **Replication design (separate batch)**: define a change feed, tombstone
   model, and conflict resolution before any sync. Keep the fetch path strictly
   read-only and do not equate it with replication.
6. **Cross-host validation**: only after the above, run a real two-host test and
   document it as cross-host; until then, keep the "same-host LAN only" label.

## Files changed

- `crates/aegis-embodiment/src/bin/artifact-node.rs` (new)
- `crates/aegis-embodiment/src/bin/artifact-fetch.rs` (new)
- `crates/aegis-embodiment/tests/artifact_node.rs` (new)
- `docs/implementation/BATCH7_AGENT08_ARTIFACT.md` (new)
