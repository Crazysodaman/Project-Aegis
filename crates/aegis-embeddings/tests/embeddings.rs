use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use aegis_contracts::initialization::{CoreInitializationRequest, CORE_INIT_REQUEST_SCHEMA};
use aegis_core_install::CoreInstaller;
use aegis_embeddings::{
    content_digest, cosine_similarity, EmbeddingError, EmbeddingProvider, EmbeddingStore,
    HashingEmbeddingProvider, OllamaEmbeddingProvider,
};
use aegis_platform::PlatformService;
use aegis_storage::now_rfc3339;

const CORE_A: &str = "a9a0a604-a4e7-4a88-a985-3f071e2c8e7d";
const CORE_B: &str = "11111111-1111-4111-8111-111111111111";

fn setup() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("aegis-root");
    let initialized = PlatformService::initialize(&root).expect("platform");
    let installer = CoreInstaller::new(&root).expect("installer");
    for (uuid, number, name) in [(CORE_A, 1, "Sofía Ada Lyra"), (CORE_B, 2, "Mack Fuller")] {
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

#[test]
fn hashing_embeddings_are_deterministic_and_lexically_similar() {
    let provider = HashingEmbeddingProvider::new(256);
    assert_eq!(provider.name(), "local-hash");
    assert!(!provider.semantic());
    let texts = vec![
        "The owner likes oolong tea.".to_owned(),
        "Tea preference: oolong.".to_owned(),
        "The weather is cold today.".to_owned(),
    ];
    let vectors = provider.embed(&texts).expect("embed");
    assert_eq!(vectors.len(), 3);
    assert_eq!(vectors[0].len(), 256);
    let related = cosine_similarity(&vectors[0], &vectors[1]);
    let unrelated = cosine_similarity(&vectors[0], &vectors[2]);
    assert!(related > unrelated);
    assert!(related > 0.0);
    let again = provider.embed(&texts[..1]).expect("again");
    assert_eq!(again[0], vectors[0]);
}

#[test]
fn cosine_rejects_dimension_mismatch_and_zero() {
    assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0]), 0.0);
    assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
}

#[test]
fn embedding_cache_is_bound_to_core_and_content() {
    let (_temp, root) = setup();
    let store = EmbeddingStore::open(&root, CORE_A).expect("store");
    let digest = content_digest("private memory");
    store
        .put(
            "memory-1",
            "local-hash",
            "feature-hash-v1",
            &digest,
            &[0.1, 0.2, 0.3],
        )
        .expect("put");
    assert_eq!(
        store
            .get("memory-1", "local-hash", "feature-hash-v1", &digest)
            .expect("get"),
        Some(vec![0.1, 0.2, 0.3])
    );
    assert!(store
        .get("memory-1", "local-hash", "feature-hash-v1", "other")
        .expect("get")
        .is_none());
    assert!(EmbeddingStore::open(&root, CORE_B)
        .expect("b")
        .get("memory-1", "local-hash", "feature-hash-v1", &digest)
        .expect("get")
        .is_none());

    let source = root.join("cores").join(CORE_A).join("embeddings.db");
    let target = root.join("cores").join(CORE_B).join("embeddings.db");
    std::fs::copy(&source, &target).expect("copy");
    assert!(matches!(
        EmbeddingStore::open(&root, CORE_B),
        Err(EmbeddingError::OwnershipMismatch)
    ));
}

#[test]
fn embedding_cache_prunes_and_clears() {
    let (_temp, root) = setup();
    let store = EmbeddingStore::open(&root, CORE_A).expect("store");
    store.put("a", "p", "m", "sha", &[0.1]).expect("put a");
    store.put("b", "p", "m", "sha", &[0.2]).expect("put b");
    assert_eq!(store.count().expect("count"), 2);
    assert_eq!(store.prune(&["a".to_owned()]).expect("prune"), 1);
    assert!(store.get_record("b").expect("record").is_none());
    store.clear_provider("p", "m").expect("clear");
    assert_eq!(store.count().expect("count"), 0);
}

#[test]
fn ollama_embedding_adapter_parses_real_http() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let address = listener.local_addr().expect("addr");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buffer = [0u8; 4096];
        let _ = stream.read(&mut buffer);
        thread::sleep(Duration::from_millis(30));
        let body = r#"{"embedding":[0.1,0.2,0.3]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
    });
    let provider = OllamaEmbeddingProvider::new(&format!("http://{address}"), "nomic-embed-text")
        .expect("provider");
    assert!(provider.semantic());
    let vectors = provider.embed(&["hello".to_owned()]).expect("embed");
    assert_eq!(vectors, vec![vec![0.1, 0.2, 0.3]]);
    server.join().expect("server");
}

#[test]
fn ollama_embedding_adapter_reports_http_errors() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let address = listener.local_addr().expect("addr");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buffer = [0u8; 4096];
        let _ = stream.read(&mut buffer);
        thread::sleep(Duration::from_millis(30));
        let body = r#"{"error":"model not found"}"#;
        let response = format!(
            "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
    });
    let provider = OllamaEmbeddingProvider::new(&format!("http://{address}"), "missing-model")
        .expect("provider");
    let error = provider
        .embed(&["hello".to_owned()])
        .expect_err("must fail");
    assert!(matches!(
        error,
        EmbeddingError::HttpStatus { status: 404, .. }
    ));
    server.join().expect("server");
}

#[test]
fn https_and_invalid_urls_are_rejected() {
    assert!(matches!(
        OllamaEmbeddingProvider::new("https://example.test", "model"),
        Err(EmbeddingError::UnsupportedScheme(_))
    ));
    assert!(matches!(
        OllamaEmbeddingProvider::new("not-a-url", "model"),
        Err(EmbeddingError::InvalidUrl(_))
    ));
    assert_eq!(content_digest("abc"), content_digest("abc"));
    assert_ne!(content_digest("abc"), content_digest("abd"));
}
