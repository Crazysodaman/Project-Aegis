use std::sync::Arc;
use std::thread;
use std::time::Duration;

use aegis_contracts::ipc::{IpcRequest, METHOD_PLATFORM_STATUS, METHOD_SAFE_STOP_SET};
use aegis_contracts::status::{EffectPingResult, PlatformStatus};
use aegis_ipc::{request_status, request_stop_clear, request_stop_set, IpcClient, IpcServer};
use aegis_platform::PlatformService;
use serde_json::json;
use uuid::Uuid;

fn unique_socket() -> String {
    format!("aegis-batch1-{}", Uuid::new_v4())
}

fn fixture() -> (tempfile::TempDir, Arc<PlatformService>, String) {
    let temp = tempfile::tempdir().expect("tempdir");
    let initialized = PlatformService::initialize(temp.path()).expect("initialize");
    let token = initialized.owner_token;
    let service = Arc::new(PlatformService::start(temp.path()).expect("start"));
    (temp, service, token)
}

#[test]
fn ipc_status_and_stop_sequence() {
    let (_temp, service, token) = fixture();
    let name = unique_socket();
    let server = IpcServer::new(service.clone(), name.clone());
    let handle = thread::spawn(move || server.serve(6));
    thread::sleep(Duration::from_millis(200));

    let client = IpcClient::new(name);
    let status = client.call(&request_status(&token)).expect("status");
    let status: PlatformStatus =
        serde_json::from_value(status.result.expect("status result")).expect("status body");
    assert!(status.zero_core);

    let ping = client
        .call(&IpcRequest::new("platform.effect.ping", json!({}), &token))
        .expect("ping");
    assert_eq!(
        serde_json::from_value::<EffectPingResult>(ping.result.expect("ping result"))
            .expect("ping body"),
        EffectPingResult::ok()
    );

    let stopped = client
        .call(&request_stop_set(&token, "ipc test"))
        .expect("stop set");
    assert_eq!(stopped.result.expect("stop result")["active"], json!(true));

    let blocked = client
        .call(&IpcRequest::new("platform.effect.ping", json!({}), &token))
        .expect("blocked ping");
    assert_eq!(blocked.error.expect("blocked error").code, "stop_active");

    let cleared = client.call(&request_stop_clear(&token)).expect("clear");
    assert_eq!(
        cleared.result.expect("clear result")["active"],
        json!(false)
    );

    let allowed = client
        .call(&IpcRequest::new("platform.effect.ping", json!({}), &token))
        .expect("allowed ping");
    assert!(allowed.result.is_some());

    handle.join().expect("server join").expect("server result");
}

#[test]
fn ipc_rejects_wrong_owner_token() {
    let (_temp, service, _token) = fixture();
    let name = unique_socket();
    let server = IpcServer::new(service, name.clone());
    let handle = thread::spawn(move || server.serve(1));
    thread::sleep(Duration::from_millis(200));
    let response = IpcClient::new(name)
        .call(&request_status("wrong-token"))
        .expect("response");
    assert_eq!(response.error.expect("error").code, "unauthorized");
    handle.join().expect("join").expect("server");
}

#[test]
fn ipc_rejects_unsupported_protocol() {
    let (_temp, service, token) = fixture();
    let name = unique_socket();
    let server = IpcServer::new(service, name.clone());
    let handle = thread::spawn(move || server.serve(1));
    thread::sleep(Duration::from_millis(200));

    let mut request = IpcRequest::new(METHOD_PLATFORM_STATUS, json!({}), token);
    request.protocol = "9.9".to_owned();
    let line = serde_json::to_string(&request).expect("raw request");
    let response = IpcClient::new(name).call_line(&line).expect("response");
    assert_eq!(response.error.expect("error").code, "unsupported_protocol");
    handle.join().expect("join").expect("server");
}

#[test]
fn ipc_rejects_malformed_json() {
    let (_temp, service, _token) = fixture();
    let name = unique_socket();
    let server = IpcServer::new(service, name.clone());
    let handle = thread::spawn(move || server.serve(1));
    thread::sleep(Duration::from_millis(200));
    let response = IpcClient::new(name).call_line("{").expect("response");
    assert_eq!(response.error.expect("error").code, "invalid_json");
    handle.join().expect("join").expect("server");
}

#[test]
fn ipc_stop_set_requires_a_reason() {
    let (_temp, service, token) = fixture();
    let name = unique_socket();
    let server = IpcServer::new(service, name.clone());
    let handle = thread::spawn(move || server.serve(1));
    thread::sleep(Duration::from_millis(200));
    let request = IpcRequest::new(METHOD_SAFE_STOP_SET, json!({}), token);
    let response = IpcClient::new(name).call(&request).expect("response");
    assert_eq!(response.error.expect("error").code, "invalid_request");
    handle.join().expect("join").expect("server");
}
