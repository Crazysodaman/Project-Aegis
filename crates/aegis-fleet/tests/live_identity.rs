//! Genuine live HMAC remote-identity re-verification over the LAN interface.
//!
//! This test spawns the real `aegis-fleet-identity` binary in `serve` mode on a
//! local LAN address, then runs the same binary in `verify` mode as a separate
//! process. It confirms a matching shared secret yields `verified=true` and a
//! wrong secret fails closed. The shared secret is passed only through the
//! `AEGIS_REMOTE_SECRET` environment variable, never on the command line.
//!
//! It is opt-in to keep the default `cargo test -p aegis-fleet` run offline and
//! deterministic:
//!
//! ```text
//! $env:AEGIS_FLEET_LIVE_HMAC = "1"
//! cargo test -p aegis-fleet --test live_identity -- --nocapture
//! ```
//!
//! Override the bind address with `AEGIS_FLEET_LAN_BIND` (default
//! `192.168.1.71:0`, where `:0` selects a free port). Artemis is never touched.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const HOST_UUID: &str = "11111111-1111-4111-8111-111111111111";
const CALLSIGN: &str = "Venus";
const SECRET: &str = "batch7-live-hmac-secret-0123456789";
const WRONG_SECRET: &str = "batch7-wrong-hmac-secret-9876543210";

fn identity_binary() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_aegis-fleet-identity"))
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run_verify(address: &str, secret: &str) -> Output {
    Command::new(identity_binary())
        .args(["verify", "--connect", address, "--expect-host", HOST_UUID])
        .env("AEGIS_REMOTE_SECRET", secret)
        .output()
        .expect("run verify")
}

#[test]
fn live_hmac_identity_over_lan_interface() {
    if std::env::var("AEGIS_FLEET_LIVE_HMAC").ok().as_deref() != Some("1") {
        eprintln!(
            "skipping live HMAC test: set AEGIS_FLEET_LIVE_HMAC=1 to exercise the LAN handshake"
        );
        return;
    }

    let bind =
        std::env::var("AEGIS_FLEET_LAN_BIND").unwrap_or_else(|_| "192.168.1.71:0".to_owned());

    let child = Command::new(identity_binary())
        .args([
            "serve",
            "--bind",
            &bind,
            "--host-uuid",
            HOST_UUID,
            "--callsign",
            CALLSIGN,
            "--accepts",
            "2",
        ])
        .env("AEGIS_REMOTE_SECRET", SECRET)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn identity serve");

    let mut guard = ChildGuard(child);
    let stdout = guard.0.stdout.take().expect("serve stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .expect("read serve listening line");
    let listening: serde_json::Value =
        serde_json::from_str(line.trim()).expect("serve listening json");
    let address = listening["listening"]
        .as_str()
        .expect("listening address")
        .to_owned();
    eprintln!("live identity serve listening on {address}");

    let valid = run_verify(&address, SECRET);
    assert!(
        valid.status.success(),
        "valid secret should verify; stderr: {}",
        String::from_utf8_lossy(&valid.stderr)
    );
    let verified: serde_json::Value =
        serde_json::from_slice(&valid.stdout).expect("verify output json");
    assert_eq!(verified["verified"], serde_json::json!(true));
    assert_eq!(verified["host_uuid"], serde_json::json!(HOST_UUID));
    assert_eq!(verified["callsign"], serde_json::json!(CALLSIGN));
    eprintln!("live identity valid secret: {verified}");

    let wrong = run_verify(&address, WRONG_SECRET);
    assert!(
        !wrong.status.success(),
        "wrong secret must fail closed; stdout: {}",
        String::from_utf8_lossy(&wrong.stdout)
    );
    let stderr = String::from_utf8_lossy(&wrong.stderr);
    assert!(
        stderr.contains("proof is invalid"),
        "unexpected wrong-secret error: {stderr}"
    );
    eprintln!("live identity wrong secret rejected: {}", stderr.trim());

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match guard.0.try_wait() {
            Ok(Some(status)) => {
                assert!(status.success(), "serve exited with {status}");
                break;
            }
            Ok(None) => {
                assert!(
                    Instant::now() < deadline,
                    "serve did not exit after serving its accept budget"
                );
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => panic!("waiting for serve failed: {error}"),
        }
    }
}
