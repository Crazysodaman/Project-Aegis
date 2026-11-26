use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use aegis_cognition::{
    CancelToken, ChatMessage, ChatRequest, InferenceError, ModelProvider, OllamaProvider,
    ProviderId, ProviderKind, ToolDefinition,
};
use serde_json::{json, Value};

struct Request {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Request> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    let mut parts = request_line.split_whitespace();
    Ok(Request {
        method: parts.next().unwrap_or("").to_owned(),
        path: parts.next().unwrap_or("").to_owned(),
        body,
    })
}

fn spawn_server<F>(connections: usize, handler: F) -> String
where
    F: Fn(Request, &mut TcpStream) + Send + 'static,
{
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind mock server");
    let address = listener.local_addr().expect("mock address");
    thread::spawn(move || {
        for _ in 0..connections {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let Ok(request) = read_request(&mut stream) else {
                continue;
            };
            handler(request, &mut stream);
        }
    });
    format!("http://{address}")
}

fn write_json(stream: &mut TcpStream, status: u16, reason: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn write_chunked(stream: &mut TcpStream, status: u16, chunks: &[&str]) {
    let head = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    );
    let _ = stream.write_all(head.as_bytes());
    for chunk in chunks {
        let bytes = chunk.as_bytes();
        let _ = write!(stream, "{:x}\r\n", bytes.len());
        let _ = stream.write_all(bytes);
        let _ = stream.write_all(b"\r\n");
    }
    let _ = stream.write_all(b"0\r\n\r\n");
    let _ = stream.flush();
}

fn chat_request() -> ChatRequest {
    ChatRequest::new("llama3:8b", vec![ChatMessage::user("hello")])
}

fn chat_status_error(status: u16) -> InferenceError {
    let base = spawn_server(1, move |_request, stream| {
        write_json(stream, status, "Error", r#"{"error":"synthetic failure"}"#);
    });
    let provider = OllamaProvider::new(base).expect("provider");
    provider
        .chat(&chat_request(), &CancelToken::new())
        .expect_err("chat must fail")
}

#[test]
fn provider_id_validation() {
    let id = ProviderId::new("ollama-local_1").expect("valid id");
    assert_eq!(id.as_str(), "ollama-local_1");
    assert!(ProviderId::new("").is_err());
    assert!(ProviderId::new("bad id").is_err());
    assert!(ProviderId::new("a".repeat(129)).is_err());
}

#[test]
fn rejects_https_and_invalid_urls() {
    assert!(matches!(
        OllamaProvider::new("https://127.0.0.1:11434"),
        Err(InferenceError::UnsupportedScheme(_))
    ));
    assert!(matches!(
        OllamaProvider::new("ftp://127.0.0.1"),
        Err(InferenceError::UnsupportedScheme(_))
    ));
    assert!(matches!(
        OllamaProvider::new("127.0.0.1:11434"),
        Err(InferenceError::InvalidUrl(_))
    ));
    assert!(matches!(
        OllamaProvider::new("http://"),
        Err(InferenceError::InvalidUrl(_))
    ));
    assert!(matches!(
        OllamaProvider::new("http://host:not-a-port"),
        Err(InferenceError::InvalidUrl(_))
    ));
}

#[test]
fn discovery_lists_models() {
    let base = spawn_server(1, |request, stream| {
        if request.method == "GET" && request.path == "/api/tags" {
            write_json(
                stream,
                200,
                "OK",
                r#"{"models":[
                    {"name":"llama3:8b","size":4700000000,"details":{"family":"llama","parameter_size":"8B","quantization_level":"Q4_K_M"}},
                    {"name":"nomic-embed-text:latest","size":274000000}
                ]}"#,
            );
        } else {
            write_json(stream, 404, "Not Found", "{}");
        }
    });
    let provider = OllamaProvider::new(base).expect("provider");
    assert_eq!(provider.id().as_str(), "ollama");
    assert_eq!(provider.kind(), ProviderKind::Local);
    let models = provider.list_models().expect("models");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].name, "llama3:8b");
    assert_eq!(models[0].size_bytes, 4_700_000_000);
    assert_eq!(models[0].family, "llama");
    assert_eq!(models[0].parameter_size, "8B");
    assert_eq!(models[0].quantization_level, "Q4_K_M");
    assert_eq!(models[1].name, "nomic-embed-text:latest");
    assert_eq!(models[1].family, "");
}

#[test]
fn health_reports_version_and_models() {
    let base = spawn_server(2, |request, stream| match request.path.as_str() {
        "/api/version" => write_json(stream, 200, "OK", r#"{"version":"0.5.7"}"#),
        "/api/tags" => write_json(
            stream,
            200,
            "OK",
            r#"{"models":[{"name":"alpha"},{"name":"beta"}]}"#,
        ),
        _ => write_json(stream, 404, "Not Found", "{}"),
    });
    let provider = OllamaProvider::new(base).expect("provider");
    let health = provider.health();
    assert!(health.reachable);
    assert_eq!(health.version.as_deref(), Some("0.5.7"));
    assert_eq!(health.model_count, 2);
    assert!(health.error.is_none());
}

#[test]
fn non_stream_chat_success() {
    let captured: Arc<Mutex<Option<Request>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&captured);
    let base = spawn_server(1, move |request, stream| {
        if request.method == "POST" && request.path == "/api/chat" {
            *sink.lock().expect("capture lock") = Some(request);
            write_json(
                stream,
                200,
                "OK",
                r#"{
                    "model":"llama3:8b",
                    "message":{
                        "role":"assistant",
                        "content":"Hello there",
                        "tool_calls":[{"function":{"name":"lookup","arguments":{"city":"Paris"}}}]
                    },
                    "done":true,
                    "total_duration":2500000000,
                    "prompt_eval_count":12,
                    "eval_count":7
                }"#,
            );
        } else {
            write_json(stream, 404, "Not Found", "{}");
        }
    });
    let provider = OllamaProvider::new(base).expect("provider");
    let request = chat_request()
        .with_temperature(0.25)
        .with_max_tokens(64)
        .with_tools(vec![ToolDefinition::new("lookup", "Look up a city")]);
    let response = provider
        .chat(&request, &CancelToken::new())
        .expect("chat succeeds");
    assert_eq!(response.model, "llama3:8b");
    assert_eq!(response.content, "Hello there");
    assert!(response.done);
    assert_eq!(response.total_duration_ms, 2500);
    assert_eq!(response.prompt_eval_count, 12);
    assert_eq!(response.eval_count, 7);
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "lookup");
    assert_eq!(response.tool_calls[0].arguments["city"], json!("Paris"));

    let request = captured
        .lock()
        .expect("capture lock")
        .take()
        .expect("captured request");
    let body: Value = serde_json::from_slice(&request.body).expect("request json");
    assert_eq!(body["model"], json!("llama3:8b"));
    assert_eq!(body["stream"], json!(false));
    assert_eq!(body["messages"][0]["role"], json!("user"));
    assert_eq!(body["messages"][0]["content"], json!("hello"));
    assert_eq!(body["options"]["temperature"], json!(0.25));
    assert_eq!(body["options"]["num_predict"], json!(64));
    assert_eq!(body["tools"][0]["function"]["name"], json!("lookup"));
    assert_eq!(body["tools"][0]["function"]["parameters"], Value::Null);
}

#[test]
fn malformed_json_is_invalid_response() {
    let base = spawn_server(1, |_request, stream| {
        write_json(stream, 200, "OK", "this is not json");
    });
    let provider = OllamaProvider::new(base).expect("provider");
    let error = provider
        .chat(&chat_request(), &CancelToken::new())
        .expect_err("chat must fail");
    assert!(matches!(error, InferenceError::InvalidResponse(_)));
}

#[test]
fn http_404_is_status_error() {
    assert!(matches!(
        chat_status_error(404),
        InferenceError::HttpStatus { status: 404, .. }
    ));
}

#[test]
fn http_500_is_status_error() {
    assert!(matches!(
        chat_status_error(500),
        InferenceError::HttpStatus { status: 500, .. }
    ));
}

#[test]
fn read_timeout_is_reported() {
    let base = spawn_server(1, |_request, stream| {
        thread::sleep(Duration::from_millis(1_500));
        write_json(
            stream,
            200,
            "OK",
            r#"{"message":{"content":"late"},"done":true}"#,
        );
    });
    let provider = OllamaProvider::new(base).expect("provider");
    let request = chat_request().with_timeout_ms(150);
    let error = provider
        .chat(&request, &CancelToken::new())
        .expect_err("chat must time out");
    assert!(matches!(error, InferenceError::Timeout(_)));
}

#[test]
fn cancellation_before_request_short_circuits() {
    let provider = OllamaProvider::new("http://127.0.0.1:1").expect("provider");
    let token = CancelToken::new();
    token.cancel();
    assert_eq!(
        provider
            .chat(&chat_request(), &token)
            .expect_err("chat must be cancelled"),
        InferenceError::Cancelled
    );
    let mut callback_called = false;
    let error = provider
        .chat_stream(&chat_request(), &token, &mut |_chunk| {
            callback_called = true;
            Ok(())
        })
        .expect_err("stream must be cancelled");
    assert_eq!(error, InferenceError::Cancelled);
    assert!(!callback_called);
}

#[test]
fn streaming_chunks_are_ordered() {
    let captured: Arc<Mutex<Option<Request>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&captured);
    let base = spawn_server(1, move |request, stream| {
        if request.method == "POST" && request.path == "/api/chat" {
            *sink.lock().expect("capture lock") = Some(request);
            write_chunked(
                stream,
                200,
                &[
                    "{\"model\":\"llama3:8b\",\"message\":{\"content\":\"Hel\"},\"done\":false}\n",
                    "{\"model\":\"llama3:8b\",\"message\":{\"content\":\"lo\"},\"done\":false}\n",
                    "{\"model\":\"llama3:8b\",\"message\":{\"content\":\" world\"},\"done\":false}\n",
                    "{\"model\":\"llama3:8b\",\"message\":{\"content\":\"\"},\"done\":true,\"total_duration\":3000000000,\"prompt_eval_count\":4,\"eval_count\":3}\n",
                ],
            );
        } else {
            write_json(stream, 404, "Not Found", "{}");
        }
    });
    let provider = OllamaProvider::new(base).expect("provider");
    let mut deltas = Vec::new();
    let mut done_flags = Vec::new();
    let response = provider
        .chat_stream(&chat_request(), &CancelToken::new(), &mut |chunk| {
            deltas.push(chunk.delta);
            done_flags.push(chunk.done);
            Ok(())
        })
        .expect("stream succeeds");
    assert_eq!(
        deltas,
        vec![
            "Hel".to_owned(),
            "lo".to_owned(),
            " world".to_owned(),
            String::new()
        ]
    );
    assert_eq!(done_flags, vec![false, false, false, true]);
    assert_eq!(response.model, "llama3:8b");
    assert_eq!(response.content, "Hello world");
    assert!(response.done);
    assert_eq!(response.total_duration_ms, 3000);
    assert_eq!(response.prompt_eval_count, 4);
    assert_eq!(response.eval_count, 3);

    let request = captured
        .lock()
        .expect("capture lock")
        .take()
        .expect("captured request");
    let body: Value = serde_json::from_slice(&request.body).expect("request json");
    assert_eq!(body["stream"], json!(true));
}

#[test]
fn streaming_cancellation_between_chunks() {
    let base = spawn_server(1, |_request, stream| {
        write_chunked(
            stream,
            200,
            &[
                "{\"model\":\"m\",\"message\":{\"content\":\"first\"},\"done\":false}\n",
                "{\"model\":\"m\",\"message\":{\"content\":\"second\"},\"done\":false}\n",
            ],
        );
    });
    let provider = OllamaProvider::new(base).expect("provider");
    let token = CancelToken::new();
    let canceller = token.clone();
    let mut seen = 0usize;
    let error = provider
        .chat_stream(&chat_request(), &token, &mut |_chunk| {
            seen += 1;
            canceller.cancel();
            Ok(())
        })
        .expect_err("stream must be cancelled");
    assert_eq!(error, InferenceError::Cancelled);
    assert_eq!(seen, 1);
}

#[test]
fn running_models_and_unload_model() {
    let base = spawn_server(2, |request, stream| {
        if request.path == "/api/ps" {
            write_json(
                stream,
                200,
                "OK",
                r#"{"models":[{"name":"qwen3.5:9b"},{"name":"nomic-embed-text"}]}"#,
            );
        } else {
            assert_eq!(request.path, "/api/generate");
            let body = String::from_utf8_lossy(&request.body);
            assert!(body.contains("\"keep_alive\":0"));
            assert!(body.contains("qwen3.5:9b"));
            write_json(stream, 200, "OK", r#"{"done":true}"#);
        }
    });
    let provider = OllamaProvider::new(base).expect("provider");
    let models = provider.running_models().expect("running models");
    assert_eq!(models, vec!["qwen3.5:9b", "nomic-embed-text"]);
    provider.unload_model("qwen3.5:9b").expect("unload");
}

#[test]
fn live_ollama_smoke() {
    if std::env::var("AEGIS_OLLAMA_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping live Ollama test: set AEGIS_OLLAMA_LIVE=1 to enable");
        return;
    }
    let Ok(url) = std::env::var("AEGIS_OLLAMA_URL") else {
        eprintln!("skipping live Ollama test: AEGIS_OLLAMA_URL is not set");
        return;
    };
    let provider = OllamaProvider::new(url).expect("valid live URL");
    let health = provider.health();
    assert!(health.reachable, "Ollama unreachable: {:?}", health.error);
    let models = provider.list_models().expect("list live models");
    let model = match std::env::var("AEGIS_OLLAMA_MODEL") {
        Ok(name) => models
            .iter()
            .find(|info| info.name == name)
            .unwrap_or_else(|| panic!("configured live model {name} is not installed")),
        Err(_) => {
            let Some(model) = models.first() else {
                eprintln!("skipping live chat: Ollama has no installed models");
                return;
            };
            model
        }
    };
    let request = ChatRequest::new(
        model.name.clone(),
        vec![ChatMessage::user("Reply with the single word ok")],
    )
    .with_max_tokens(32)
    .with_timeout_ms(120_000)
    .with_think(
        if std::env::var("AEGIS_OLLAMA_NO_THINK").ok().as_deref() == Some("1") {
            Some(false)
        } else {
            None
        },
    );
    let response = provider
        .chat(&request, &CancelToken::new())
        .expect("live chat");
    assert!(!response.content.trim().is_empty());

    let mut chunks = 0usize;
    let stream = provider
        .chat_stream(&request, &CancelToken::new(), &mut |chunk| {
            if !chunk.delta.is_empty() {
                chunks += 1;
            }
            Ok(())
        })
        .expect("live stream");
    assert!(chunks > 0, "live stream produced no content chunks");
    assert!(!stream.content.trim().is_empty());

    let cancel = CancelToken::new();
    let mut observed = 0usize;
    let cancelled = provider.chat_stream(&request, &cancel, &mut |chunk| {
        if !chunk.delta.is_empty() {
            observed += 1;
            cancel.cancel();
        }
        Ok(())
    });
    assert!(matches!(cancelled, Err(InferenceError::Cancelled)));
    assert!(observed > 0, "cancellation ran before any chunk arrived");
}

#[test]
fn live_ollama_cancel_under_load_then_recovers() {
    if std::env::var("AEGIS_OLLAMA_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping live cancellation test: set AEGIS_OLLAMA_LIVE=1 to enable");
        return;
    }
    let Ok(url) = std::env::var("AEGIS_OLLAMA_URL") else {
        eprintln!("skipping live cancellation test: AEGIS_OLLAMA_URL is not set");
        return;
    };
    let Ok(model) = std::env::var("AEGIS_OLLAMA_MODEL") else {
        eprintln!("skipping live cancellation test: AEGIS_OLLAMA_MODEL is not set");
        return;
    };
    let provider = OllamaProvider::new(url).expect("valid live URL");
    let request = ChatRequest::new(
        model,
        vec![ChatMessage::user(
            "Write a very long numbered list of facts about the ocean.",
        )],
    )
    .with_max_tokens(2048)
    .with_timeout_ms(120_000)
    .with_think(Some(false));

    let cancel = CancelToken::new();
    let mut observed = 0usize;
    let started = std::time::Instant::now();
    let result = provider.chat_stream(&request, &cancel, &mut |chunk| {
        if !chunk.delta.is_empty() {
            observed += 1;
            if observed >= 3 || started.elapsed() >= Duration::from_secs(2) {
                cancel.cancel();
            }
        }
        Ok(())
    });
    assert!(
        matches!(result, Err(InferenceError::Cancelled)),
        "expected cancellation, got {result:?}"
    );
    assert!(observed > 0, "cancellation ran before any chunk arrived");

    // The provider must remain usable after an aborted generation.
    let health = provider.health();
    assert!(
        health.reachable,
        "provider unhealthy after cancel: {:?}",
        health.error
    );
    let follow_up = provider
        .chat(
            &ChatRequest::new(
                request.model.clone(),
                vec![ChatMessage::user("Reply with the single word ok")],
            )
            .with_max_tokens(16)
            .with_timeout_ms(120_000)
            .with_think(Some(false)),
            &CancelToken::new(),
        )
        .expect("follow-up chat after cancellation");
    assert!(!follow_up.content.trim().is_empty());
}
