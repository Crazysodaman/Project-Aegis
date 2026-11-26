# Batch 6 — Agent 08: Cross-Host Artifact Library (Authenticated Remote Fetch)

Status: implemented and verified in `aegis-embodiment` only.

## Scope

This batch adds an additive, deliberately simple authenticated remote artifact
fetch protocol on top of the Batch 5 per-Core artifact library. It introduces no
new dependencies, does not change the SQLite schema, does not modify any other
crate, and does not alter existing local asset, audit, or manifest behavior.

Cross-host behavior is **protocol-level only**. The implementation is exercised
against an in-process `std::net::TcpListener` on `127.0.0.1`; this is a LAN /
protocol test, **not** Artemis deployment or cross-host verification. No Artemis
host, credential, or deployment was used.

## Added module

`crates/aegis-embodiment/src/remote.rs` (re-exported from the crate root).

### Wire format

Every message is a length-prefixed frame:

```
u32 big-endian length | payload
```

- `MAX_FRAME_BYTES` (64 KiB) bounds request and response header frames.
- `MAX_ARTIFACT_FRAME_BYTES` (256 MiB, aligned with `MAX_ASSET_BYTES`) bounds the
  data frame.
- Empty frames and over-limit frames are rejected before allocation/read.
- Both client and server set read/write timeouts (`DEFAULT_TIMEOUT_MS` 5 s,
  capped at `MAX_TIMEOUT_MS` 60 s).

Flow:

1. Client sends `ArtifactRequest { schema, core_uuid, asset_id, nonce, proof }`.
2. `proof` is hex HMAC-SHA256 over a domain-separated message bound to the Core
   UUID, asset id, and nonce.
3. Server verifies the proof in constant time, then checks that it serves the
   requested Core and that the asset is owned by that Core.
4. Server replies with a signed `ArtifactResponse` header (status, Core, asset,
   nonce echo, media type, SHA-256, byte length) followed by one data frame.
5. Client verifies the server proof, nonce/Core/asset binding, bounded length,
   and the SHA-256 of the received bytes before returning `FetchedArtifact`.

`FetchedArtifact::save_to` writes atomically: bytes are written to a temporary
file, re-hashed, then renamed; the destination is never a partial file.

### Public API

- Types: `RemoteArtifactClient`, `RemoteArtifactServer`, `ArtifactRequest`,
  `ArtifactResponse`, `ResponseStatus`, `FetchedArtifact`, `RemoteError`.
- Client: `new`, `with_timeout`, `with_max_artifact_bytes`, `fetch`,
  `fetch_with_nonce`, `fetch_to_path`.
- Server: `new`, `with_timeout`, `serve_once`.
- Helpers: `new_nonce`, `hmac_sha256`, `sha256_hex`, `write_frame`, `read_frame`.
- Bounds/schemas: `MIN_SECRET_BYTES` (16), `MAX_FRAME_BYTES`,
  `MAX_ARTIFACT_FRAME_BYTES`, `MAX_NONCE_CHARS`, `REMOTE_REQUEST_SCHEMA`,
  `REMOTE_RESPONSE_SCHEMA`, `REMOTE_PROOF_DOMAIN`.

## Safety properties

- **Authentication**: request and response are HMAC-SHA256 signed with a shared
  secret; proofs are compared with a constant-time equality check.
- **Ownership isolation**: the server only answers for the Core opened in its
  `EmbodimentStore`. `EmbodimentStore::asset`/`read_asset` are already scoped by
  `core_uuid`, so an asset owned by another Core is reported as `NotFound` and
  its bytes are never read. A request whose `core_uuid` differs from the served
  Core is rejected with `WrongCore`.
- **No private memory exposure**: only a named, live asset owned by the served
  Core is returned. There is no endpoint for arbitrary files, appearance state,
  audit records, events, or another Core's data.
- **Integrity**: the server re-hashes bytes before sending; the client verifies
  the authenticated SHA-256 before returning or writing. A wrong digest returns
  `RemoteError::IntegrityMismatch`.
- **Truncation**: short or interrupted data frames return `RemoteError::Truncated`
  and `fetch_to_path` writes nothing.
- **Bounded resources**: all frames are size-bounded and all sockets have
  timeouts; `FrameTooLarge`/`Timeout` are typed errors.
- **Idempotent, read-only**: fetching the same artifact id returns the same
  digest and bytes; the serving store's rows, revisions, and files are not
  mutated. There is no write, delete, or sync operation.

## Recovery / backup note

The Batch 5 local recovery manifest (`EmbodimentStore::recovery_manifest`)
describes every live and soft-deleted asset for one Core, including `asset_id`,
`kind`, `name`, `media_type`, `sha256`, `bytes`, `state`, and the path relative
to the Core root.

A restore using this protocol would:

1. Read the manifest entry for the lost/corrupt asset.
2. Fetch the asset by `asset_id` from an authorized serving host that owns it.
3. Verify the returned `sha256` against the manifest entry
   (`FetchedArtifact::matches_expected_sha256`) and re-check its length.
4. Only then write it with `save_to` (atomic) or a caller-owned recovery step.
5. Re-run `EmbodimentStore::audit` on the recovered Core to confirm integrity.

This restores bytes and metadata evidence; it does not automatically reinsert an
asset record into a Core database. Re-importing into the owning Core remains an
explicit owner action.

**Not implemented (by design, in this batch):**

- Replication or background synchronization between hosts.
- Tombstone propagation or distributed delete semantics.
- Conflict resolution for concurrent edits, deletes, or divergent revisions.
- Multi-host redundancy, quorum, failover, or peer discovery.
- TLS/mTLS, key rotation, per-Core asymmetric identity, or revocation.
- Resumable/partial transfer, throughput or disk-space quotas.
- Any destructive sync, mirroring, or remote delete.

The shared secret is symmetric and must be provisioned out of band; the
transport is plain TCP and must stay on a trusted LAN or loopback. This is not a
replacement for TLS and is not intended for the public internet.

## Tests

`crates/aegis-embodiment/tests/remote.rs` (in-process `127.0.0.1` listeners,
LAN/protocol only):

- `lan_round_trip_is_authenticated_and_idempotent` — signed fetch succeeds,
  duplicate request is idempotent, store revision unchanged, audit clean.
- `wrong_secret_is_denied` — wrong shared secret is denied.
- `invalid_core_is_denied` — request for a non-served Core gets `WrongCore`.
- `non_owned_asset_is_not_found` — another Core's asset id is `NotFound`, no leak.
- `wrong_digest_is_rejected` — authenticated header with mismatched bytes fails
  `IntegrityMismatch`.
- `truncated_transfer_is_rejected_and_writes_no_file` — partial data yields
  `Truncated`; destination file is absent.
- `oversized_frame_is_rejected` — over-limit length prefix yields `FrameTooLarge`.

Unit tests in `remote.rs`:

- `hmac_matches_reference_vector` (RFC 4231 case 2).
- `hex_round_trips_and_rejects_bad_input`.
- `request_and_response_proofs_are_bound_to_fields`.
- `short_secret_and_bad_nonce_are_rejected`.

## Verification

- `cargo test -p aegis-embodiment` — 51 passed, 0 failed (11 new: 4 unit + 7
  integration; 40 pre-existing).
- `cargo clippy -p aegis-embodiment --all-targets -- -D warnings` — clean.

Only `cargo test`/`cargo clippy` for `aegis-embodiment` were run; no
whole-workspace gates were executed.

## Simulated vs real

- **Real**: HMAC-SHA256 signing/verification, constant-time comparison,
  SHA-256 digest checks, frame length bounds, timeouts, TCP framing, per-Core
  ownership checks, atomic verified writes, typed error handling.
- **Simulated / local-only**: the peer is an in-process `TcpListener` on
  `127.0.0.1`. No Artemis host was contacted and no cross-host deployment was
  performed or claimed.

## Limitations

- Plain TCP with a symmetric shared secret; no TLS, mutual certificate identity,
  replay cache, or key management.
- One request per connection; no multiplexing, keep-alive, or bulk/batch fetch.
- Not resumable; an interrupted transfer is discarded and must be retried from
  the beginning.
- A denial/failure after framing is surfaced via a signed status when the
  request parsed; a malformed frame simply drops the connection.
- Large artifacts are buffered in memory on both sides; very large libraries
  require per-transfer size planning.
- Restore writes bytes only; re-registration into a Core database is an explicit
  owner action, not automatic.

## Next steps

1. Add TLS or an authenticated channel and per-Core key provisioning/rotation.
2. Add replay protection (nonce cache) and request rate/space limits.
3. Add resumable/chunked transfer with per-chunk digests.
4. Define a replication/change-feed and tombstone model before enabling any
   sync; keep the fetch path strictly read-only.
5. Implement and test conflict resolution and multi-host redundancy separately.
6. Validate on real hosts (including Artemis) before making cross-host claims.

## Files changed

- `crates/aegis-embodiment/src/remote.rs` (new)
- `crates/aegis-embodiment/src/lib.rs` (module + re-exports)
- `crates/aegis-embodiment/tests/remote.rs` (new)
- `docs/implementation/BATCH6_AGENT08_ARTIFACT_LIBRARY.md` (new)
