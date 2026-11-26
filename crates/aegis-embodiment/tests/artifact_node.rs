//! Spawned-binary integration tests for the standalone `artifact-node` server.
//!
//! These tests start the real compiled executable as a child process and talk to
//! it over TCP. All automated listeners bind to `127.0.0.1`; no external host,
//! service, or credential is contacted. The live same-host LAN retrieval is
//! `#[ignore]`d and must be requested explicitly.
//!
//! This is a LAN / protocol test. It is **not** Artemis deployment, cross-host
//! replication, or synchronization.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_embodiment::{
    read_frame, sha256_hex, write_frame, AssetKind, EmbodimentStore, FetchedArtifact, NewAsset,
    RemoteArtifactClient, RemoteError, MAX_ARTIFACT_FRAME_BYTES, MAX_FRAME_BYTES,
};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";
const SECRET: &str = "aegis artifact node shared secret 0123456789";
const OTHER_SECRET: &str = "an entirely different shared secret 9876543210";
const SECRET_ENV: &str = "AEGIS_ARTIFACT_SECRET";
const NODE_BIN: &str = env!("CARGO_BIN_EXE_artifact-node");
const FETCH_BIN: &str = env!("CARGO_BIN_EXE_artifact-fetch");

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

fn core_asset_path(root: &Path, core: &str, relative_path: &str) -> PathBuf {
    root.join("cores").join(core).join(relative_path)
}

fn temporary_files(directory: &Path) -> Vec<String> {
    fs::read_dir(directory)
        .expect("read dir")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect()
}

/// Child-process guard: kills and reaps the server when the test ends.
struct Node {
    child: Child,
    address: SocketAddr,
}

impl Node {
    fn spawn(bind: &str, root: &Path, core: &str, secret: &str) -> Self {
        let mut child = Command::new(NODE_BIN)
            .arg(bind)
            .arg(root)
            .arg(core)
            .env(SECRET_ENV, secret)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn artifact-node");
        let stdout = child.stdout.take().expect("node stdout");
        let mut reader = BufReader::new(stdout);
        let mut banner = String::new();
        reader.read_line(&mut banner).expect("read banner");
        let rest = banner
            .trim()
            .strip_prefix("artifact-node listening on ")
            .unwrap_or_else(|| panic!("unexpected node banner: {banner:?}"));
        let address: SocketAddr = rest
            .split_whitespace()
            .next()
            .expect("address in banner")
            .parse()
            .expect("parse address");
        Self { child, address }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn spawned_node_serves_verified_artifact_and_is_idempotent() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let payload = b"portrait-bytes-for-the-spawned-artifact-node";
    let asset_id = import(&store, "portrait.bin", "application/octet-stream", payload);
    let expected = store.asset(&asset_id).expect("asset");
    let disk_path = core_asset_path(&root, CORE_A, &expected.relative_path);
    drop(store);

    let node = Node::spawn("127.0.0.1:0", &root, CORE_A, SECRET);
    let client = RemoteArtifactClient::new(SECRET).expect("client");

    let nonce = "spawned-duplicate-request-nonce";
    let first = client
        .fetch_with_nonce(node.address, CORE_A, &asset_id, nonce)
        .expect("first fetch");
    let second = client
        .fetch_with_nonce(node.address, CORE_A, &asset_id, nonce)
        .expect("duplicate fetch");

    assert_eq!(first, second);
    assert_eq!(first.core_uuid, CORE_A);
    assert_eq!(first.asset_id, asset_id);
    assert_eq!(first.media_type, "application/octet-stream");
    assert_eq!(first.bytes, payload);
    assert_eq!(first.bytes.len() as u64, expected.bytes);
    assert_eq!(first.sha256, expected.sha256);
    assert!(first.matches_expected_sha256(&expected.sha256));
    assert_eq!(fs::read(&disk_path).expect("on-disk asset"), first.bytes);

    let restored = root.join("restored").join("portrait.bin");
    let saved = client
        .fetch_to_path(node.address, CORE_A, &asset_id, &restored)
        .expect("verified save");
    assert_eq!(saved.bytes, first.bytes);
    assert_eq!(fs::read(&restored).expect("restored file"), first.bytes);
    assert!(
        temporary_files(restored.parent().expect("parent")).is_empty(),
        "atomic save left a temporary file behind"
    );

    drop(node);

    let reopened = open_store(&root, CORE_A);
    assert_eq!(
        reopened.asset(&asset_id).expect("asset").revision,
        expected.revision,
        "read-only fetch must not mutate the serving store"
    );
    assert!(reopened.audit().expect("audit").is_clean());
}

#[test]
fn spawned_node_denies_wrong_secret() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let asset_id = import(&store, "denied.bin", "application/octet-stream", b"denied");
    drop(store);

    let node = Node::spawn("127.0.0.1:0", &root, CORE_A, SECRET);
    let client = RemoteArtifactClient::new(OTHER_SECRET).expect("client");
    let error = client
        .fetch(node.address, CORE_A, &asset_id)
        .expect_err("wrong secret must be denied");
    assert!(matches!(error, RemoteError::Unauthorized));
}

#[test]
fn spawned_node_does_not_expose_deleted_unknown_or_foreign_assets() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let deleted_id = import(
        &store,
        "deleted.bin",
        "application/octet-stream",
        b"deleted-secret-bytes",
    );
    store.delete_asset(&deleted_id).expect("delete");
    let store_b = open_store(&root, CORE_B);
    let foreign_id = import(
        &store_b,
        "foreign.bin",
        "application/octet-stream",
        b"foreign-secret-bytes",
    );
    drop(store_b);
    drop(store);

    let node = Node::spawn("127.0.0.1:0", &root, CORE_A, SECRET);
    let client = RemoteArtifactClient::new(SECRET).expect("client");

    assert!(matches!(
        client.fetch(node.address, CORE_A, &deleted_id),
        Err(RemoteError::NotFound)
    ));
    assert!(matches!(
        client.fetch(node.address, CORE_A, &foreign_id),
        Err(RemoteError::NotFound)
    ));
    assert!(matches!(
        client.fetch(node.address, CORE_A, "00000000-0000-4000-8000-000000000000"),
        Err(RemoteError::NotFound)
    ));
    assert!(matches!(
        client.fetch(node.address, CORE_B, &deleted_id),
        Err(RemoteError::WrongCore)
    ));
}

#[test]
fn truncated_transfer_from_real_node_writes_no_file() {
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let payload = vec![7u8; 64 * 1024];
    let asset_id = import(&store, "truncate.bin", "application/octet-stream", &payload);
    drop(store);

    let node = Node::spawn("127.0.0.1:0", &root, CORE_A, SECRET);
    let node_address = node.address;

    // Forward a genuine authenticated exchange but cut the data frame in half,
    // so the client must detect truncation produced by a real node response.
    let proxy = TcpListener::bind(("127.0.0.1", 0)).expect("proxy bind");
    let proxy_address = proxy.local_addr().expect("proxy address");
    let proxy_handle = thread::spawn(move || {
        let (mut downstream, _) = proxy.accept().expect("proxy accept");
        let mut upstream = TcpStream::connect(node_address).expect("proxy upstream");
        let request = read_frame(&mut downstream, MAX_FRAME_BYTES).expect("request");
        write_frame(&mut upstream, &request, MAX_FRAME_BYTES).expect("forward request");
        let header = read_frame(&mut upstream, MAX_FRAME_BYTES).expect("header");
        write_frame(&mut downstream, &header, MAX_FRAME_BYTES).expect("forward header");
        let data = read_frame(&mut upstream, MAX_ARTIFACT_FRAME_BYTES).expect("data");
        let half = data.len() / 2;
        downstream
            .write_all(&(data.len() as u32).to_be_bytes())
            .expect("length prefix");
        downstream.write_all(&data[..half]).expect("partial data");
        downstream.flush().expect("flush");
    });

    let destination = root.join("restored").join("truncated.bin");
    let client = RemoteArtifactClient::new(SECRET).expect("client");
    let error = client
        .fetch_to_path(proxy_address, CORE_A, &asset_id, &destination)
        .expect_err("truncation must be rejected");
    assert!(matches!(error, RemoteError::Truncated));
    assert!(
        !destination.exists(),
        "partial transfer must not be written"
    );

    proxy_handle.join().expect("proxy join");
}

#[test]
fn atomic_save_never_leaves_partial_or_temporary_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bytes = b"atomic-save-payload".to_vec();
    let good = FetchedArtifact {
        core_uuid: CORE_A.to_owned(),
        asset_id: "33333333-3333-4333-8333-333333333333".to_owned(),
        media_type: "application/octet-stream".to_owned(),
        sha256: sha256_hex(&bytes),
        bytes: bytes.clone(),
    };
    let destination = temp.path().join("nested").join("artifact.bin");
    good.save_to(&destination).expect("atomic save");
    assert_eq!(fs::read(&destination).expect("saved"), bytes);
    assert!(
        temporary_files(destination.parent().expect("parent")).is_empty(),
        "saved file left a temporary sibling"
    );

    let tampered = FetchedArtifact {
        sha256: sha256_hex(b"a-different-content"),
        ..good
    };
    let rejected = temp.path().join("nested").join("tampered.bin");
    assert!(matches!(
        tampered.save_to(&rejected),
        Err(RemoteError::IntegrityMismatch)
    ));
    assert!(!rejected.exists(), "a failed save must not create a file");
}

#[test]
#[ignore = "requires the host LAN address; run explicitly with --ignored --nocapture"]
fn live_lan_same_host_two_process_retrieval() {
    let bind = std::env::var("AEGIS_LIVE_BIND").unwrap_or_else(|_| "192.168.1.71:0".to_owned());
    let (_temp, root) = setup();
    let store = open_store(&root, CORE_A);
    let payload: Vec<u8> = (0..131_072u32).map(|value| (value % 251) as u8).collect();
    let asset_id = import(&store, "live-lan.bin", "application/octet-stream", &payload);
    let expected = store.asset(&asset_id).expect("asset");
    drop(store);

    let node = Node::spawn(&bind, &root, CORE_A, SECRET);
    let destination = root.join("lan-restored").join("live-lan.bin");
    let output = Command::new(FETCH_BIN)
        .arg(node.address.to_string())
        .arg(CORE_A)
        .arg(&asset_id)
        .arg(&destination)
        .env(SECRET_ENV, SECRET)
        .output()
        .expect("run artifact-fetch");
    assert!(
        output.status.success(),
        "artifact-fetch failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("artifact-fetch: {}", stdout.trim());
    println!(
        "live node={} asset={} bytes={} sha256={}",
        node.address, asset_id, expected.bytes, expected.sha256
    );

    let written = fs::read(&destination).expect("restored file");
    assert_eq!(written.len() as u64, expected.bytes);
    assert_eq!(sha256_hex(&written), expected.sha256);
    assert_eq!(written, payload);
    assert!(!node.address.ip().is_loopback(), "expected a LAN address");
    assert!(stdout.contains(&expected.sha256));
}
