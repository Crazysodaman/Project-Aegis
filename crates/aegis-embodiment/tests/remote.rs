//! LAN / protocol tests for the authenticated remote artifact fetch.
//!
//! Every listener here is an in-process `std::net::TcpListener` bound to
//! `127.0.0.1` with an ephemeral port. These are local protocol tests, not
//! Artemis or cross-host verification. No external host, service, or credential
//! is contacted.

use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_embodiment::{
    read_frame, sha256_hex, write_frame, ArtifactRequest, ArtifactResponse, AssetKind,
    EmbodimentStore, NewAsset, RemoteArtifactClient, RemoteArtifactServer, RemoteError,
    ResponseStatus, MAX_FRAME_BYTES, REMOTE_RESPONSE_SCHEMA,
};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";
const SECRET: &[u8] = b"aegis lan artifact shared secret 0123456789";
const OTHER_SECRET: &[u8] = b"a different shared secret 9876543210abcdef";
const ASSET_ID: &str = "22222222-2222-4222-8222-222222222222";

fn setup() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    let initialized = PlatformService::initialize(&root).expect("platform");
    let installer = CoreInstaller::new(&root).expect("installer");
    for (uuid, number, name) in [(CORE_A, 1, "SofA-a Ada Lyra"), (CORE_B, 2, "Mack Fuller")] {
        installer
            .initialize(
                &CoreInitializationRequest {
                    schema: CORE_INIT_REQUEST_SCHEMA.to_owned(),
                    core_uuid: uuid.to_owned(),
                    core_number: number,
                    display_name: name.to_owned(),
                    requested_at: now_rfc3339().expect("time"),
                },
                &initialized.owner_token,
            )
            .expect("core");
    }
    (temp, root)
}

fn open_store(root: &Path, core: &str) -> EmbodimentStore {
    EmbodimentStore::open(root, core).expect("store")
}

fn import(store: &EmbodimentStore, name: &str, media_type: &str, bytes: &[u8]) -> String {
    store
        .import_asset(NewAsset::new(AssetKind::Avatar, name, media_type, bytes))
        .expect("import")
        .asset_id
}

fn bind() -> (TcpListener, std::net::SocketAddr) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let address = listener.local_addr().expect("address");
    (listener, address)
}

#[test]
fn lan_round_trip_is_authenticated_and_idempotent() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let asset_id = import(&store, "portrait.png", "image/png", b"portrait-bytes");
    let expected = store.asset(&asset_id).expect("asset");

    let (listener, address) = bind();
    let server_root = root.clone();
    let secret = SECRET.to_vec();
    let handle = thread::spawn(move || {
        let server_store = EmbodimentStore::open(&server_root, CORE_A).expect("server store");
        let server = RemoteArtifactServer::new(&server_store, &secret).expect("server");
        for _ in 0..2 {
            let (stream, _) = listener.accept().expect("accept");
            server.serve_once(stream).expect("serve");
        }
    });

    let client = RemoteArtifactClient::new(SECRET).expect("client");
    let nonce = "duplicate-request-nonce";
    let first = client
        .fetch_with_nonce(address, CORE_A, &asset_id, nonce)
        .expect("first fetch");
    let second = client
        .fetch_with_nonce(address, CORE_A, &asset_id, nonce)
        .expect("duplicate fetch");
    handle.join().expect("join");

    assert_eq!(first.core_uuid, CORE_A);
    assert_eq!(first.asset_id, asset_id);
    assert_eq!(first.media_type, "image/png");
    assert_eq!(first.bytes, b"portrait-bytes");
    assert_eq!(first.sha256, expected.sha256);
    assert!(first.matches_expected_sha256(&expected.sha256));
    assert_eq!(first, second);
    assert_eq!(store.asset(&asset_id).expect("asset").revision, 1);
    assert!(store.audit().expect("audit").is_clean());
}

#[test]
fn wrong_secret_is_denied() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let asset_id = import(&store, "secret.bin", "application/octet-stream", b"secret");

    let (listener, address) = bind();
    let server_root = root.clone();
    let secret = SECRET.to_vec();
    let handle = thread::spawn(move || {
        let server_store = EmbodimentStore::open(&server_root, CORE_A).expect("server store");
        let server = RemoteArtifactServer::new(&server_store, &secret).expect("server");
        let (stream, _) = listener.accept().expect("accept");
        server.serve_once(stream)
    });

    let client = RemoteArtifactClient::new(OTHER_SECRET).expect("client");
    let error = client
        .fetch(address, CORE_A, &asset_id)
        .expect_err("must deny");
    assert!(matches!(error, RemoteError::Unauthorized));
    handle
        .join()
        .expect("join")
        .expect("server sent a signed denial");
}

#[test]
fn invalid_core_is_denied() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let asset_id = import(&store, "core.bin", "application/octet-stream", b"core");

    let (listener, address) = bind();
    let server_root = root.clone();
    let secret = SECRET.to_vec();
    let handle = thread::spawn(move || {
        let server_store = EmbodimentStore::open(&server_root, CORE_A).expect("server store");
        let server = RemoteArtifactServer::new(&server_store, &secret).expect("server");
        let (stream, _) = listener.accept().expect("accept");
        server.serve_once(stream)
    });

    let client = RemoteArtifactClient::new(SECRET).expect("client");
    let error = client
        .fetch(address, CORE_B, &asset_id)
        .expect_err("must deny foreign core");
    assert!(matches!(error, RemoteError::WrongCore));
    handle.join().expect("join").expect("serve");
}

#[test]
fn non_owned_asset_is_not_found() {
    let (_temp, root) = setup();
    let store_b = open_store(&root, CORE_B);
    let foreign_asset = import(
        &store_b,
        "foreign.bin",
        "application/octet-stream",
        b"foreign",
    );

    let (listener, address) = bind();
    let server_root = root.clone();
    let secret = SECRET.to_vec();
    let handle = thread::spawn(move || {
        let server_store = EmbodimentStore::open(&server_root, CORE_A).expect("server store");
        let server = RemoteArtifactServer::new(&server_store, &secret).expect("server");
        let (stream, _) = listener.accept().expect("accept");
        server.serve_once(stream)
    });

    let client = RemoteArtifactClient::new(SECRET).expect("client");
    let error = client
        .fetch(address, CORE_A, &foreign_asset)
        .expect_err("must not leak another Core's asset");
    assert!(matches!(error, RemoteError::NotFound));
    handle.join().expect("join").expect("serve");
}

#[test]
fn wrong_digest_is_rejected() {
    let (listener, address) = bind();
    let secret = SECRET.to_vec();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let frame = read_frame(&mut stream, MAX_FRAME_BYTES).expect("request frame");
        let request: ArtifactRequest = serde_json::from_slice(&frame).expect("request");
        let tampered = b"tampered-artifact";
        let mut response = ArtifactResponse {
            schema: REMOTE_RESPONSE_SCHEMA.to_owned(),
            status: ResponseStatus::Ok,
            core_uuid: request.core_uuid.clone(),
            asset_id: request.asset_id.clone(),
            nonce: request.nonce.clone(),
            media_type: "application/octet-stream".to_owned(),
            sha256: sha256_hex(b"expected-content"),
            bytes: tampered.len() as u64,
            proof: String::new(),
        };
        response.sign(&secret);
        let header = serde_json::to_vec(&response).expect("response");
        write_frame(&mut stream, &header, MAX_FRAME_BYTES).expect("header");
        write_frame(&mut stream, tampered, MAX_FRAME_BYTES).expect("data");
    });

    let client = RemoteArtifactClient::new(SECRET).expect("client");
    let error = client
        .fetch(address, CORE_A, ASSET_ID)
        .expect_err("must reject digest");
    assert!(matches!(error, RemoteError::IntegrityMismatch));
    handle.join().expect("join");
}

#[test]
fn truncated_transfer_is_rejected_and_writes_no_file() {
    let (_temp, root) = setup();
    let destination = root.join("restored").join("artifact.bin");
    let payload = b"payload-that-will-be-cut-short".to_vec();

    let (listener, address) = bind();
    let secret = SECRET.to_vec();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let frame = read_frame(&mut stream, MAX_FRAME_BYTES).expect("request frame");
        let request: ArtifactRequest = serde_json::from_slice(&frame).expect("request");
        let mut response = ArtifactResponse {
            schema: REMOTE_RESPONSE_SCHEMA.to_owned(),
            status: ResponseStatus::Ok,
            core_uuid: request.core_uuid.clone(),
            asset_id: request.asset_id.clone(),
            nonce: request.nonce.clone(),
            media_type: "application/octet-stream".to_owned(),
            sha256: sha256_hex(&payload),
            bytes: payload.len() as u64,
            proof: String::new(),
        };
        response.sign(&secret);
        let header = serde_json::to_vec(&response).expect("response");
        write_frame(&mut stream, &header, MAX_FRAME_BYTES).expect("header");
        stream
            .write_all(&(payload.len() as u32).to_be_bytes())
            .expect("length prefix");
        let half = payload.len() / 2;
        stream.write_all(&payload[..half]).expect("partial data");
        stream.flush().expect("flush");
    });

    let client = RemoteArtifactClient::new(SECRET).expect("client");
    let error = client
        .fetch_to_path(address, CORE_A, ASSET_ID, &destination)
        .expect_err("must reject truncated transfer");
    assert!(matches!(error, RemoteError::Truncated));
    assert!(!destination.exists());
    handle.join().expect("join");
}

#[test]
fn oversized_frame_is_rejected() {
    let (listener, address) = bind();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        read_frame(&mut stream, MAX_FRAME_BYTES).expect("request frame");
        let too_large = (MAX_FRAME_BYTES as u32) + 1;
        stream
            .write_all(&too_large.to_be_bytes())
            .expect("length prefix");
        stream.flush().expect("flush");
    });

    let client = RemoteArtifactClient::new(SECRET).expect("client");
    let error = client
        .fetch(address, CORE_A, ASSET_ID)
        .expect_err("must reject oversized frame");
    assert!(matches!(error, RemoteError::FrameTooLarge(_)));
    handle.join().expect("join");
}
