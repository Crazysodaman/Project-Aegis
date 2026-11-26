//! Minimal Fleet remote-identity handshake agent.
//!
//! Two modes share one newline-delimited JSON protocol:
//!
//! * `serve --bind <addr> [--accepts N]` reads a [`RemoteChallenge`] line and
//!   answers with a [`RemoteResponse`] line.
//! * `verify --connect <addr> --expect-host <uuid>` issues a challenge, reads
//!   the response, and verifies it.
//!
//! The shared secret is read from `AEGIS_REMOTE_SECRET` (never from argv) and
//! is never printed. This binary exists to exercise the identity proof across
//! real processes and sockets; it is not a long-running service.

use std::env;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::ExitCode;
use std::time::Duration;

use aegis_fleet::remote::{verify_response, RemoteChallenge, RemoteIdentity, RemoteResponse};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aegis-fleet-identity: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let secret =
        env::var("AEGIS_REMOTE_SECRET").map_err(|_| "AEGIS_REMOTE_SECRET is not set".to_owned())?;
    let command = args.first().map(String::as_str).unwrap_or("help");
    match command {
        "serve" => serve(&args, secret.as_bytes()),
        "verify" => verify(&args, secret.as_bytes()),
        _ => Err("usage: aegis-fleet-identity serve|verify ...".to_owned()),
    }
}

fn serve(args: &[String], secret: &[u8]) -> Result<(), String> {
    let policy = parse_host(args, secret)?;
    let bind = optional(args, "--bind").ok_or_else(|| "missing --bind".to_owned())?;
    let accepts = optional(args, "--accepts")
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| "bad --accepts".to_owned())
        })
        .transpose()?
        .unwrap_or(4);
    let listener = TcpListener::bind(&bind).map_err(|error| format!("bind failed: {error}"))?;
    println!(
        "{{\"listening\":\"{}\",\"accepts\":{accepts}}}",
        listener
            .local_addr()
            .map(|address| address.to_string())
            .unwrap_or_else(|_| bind.clone())
    );
    for _ in 0..accepts {
        let (stream, _) = match listener.accept() {
            Ok(pair) => pair,
            Err(_) => continue,
        };
        let _ = handle_connection(stream, &policy);
    }
    Ok(())
}

fn handle_connection(stream: TcpStream, policy: &RemoteIdentity) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    let challenge: RemoteChallenge =
        serde_json::from_str(line.trim()).map_err(|error| format!("bad challenge: {error}"))?;
    let response = policy
        .respond(&challenge)
        .map_err(|error| error.to_string())?;
    let mut stream = stream;
    let encoded = serde_json::to_string(&response).map_err(|error| error.to_string())?;
    stream
        .write_all(encoded.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(|error| error.to_string())
}

fn verify(args: &[String], secret: &[u8]) -> Result<(), String> {
    let connect = optional(args, "--connect").ok_or_else(|| "missing --connect".to_owned())?;
    let expect_host =
        optional(args, "--expect-host").ok_or_else(|| "missing --expect-host".to_owned())?;
    let challenge = RemoteIdentity::challenge();
    let mut stream = TcpStream::connect(&connect).map_err(|error| format!("connect: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| error.to_string())?;
    let encoded = serde_json::to_string(&challenge).map_err(|error| error.to_string())?;
    stream
        .write_all(encoded.as_bytes())
        .and_then(|_| stream.write_all(b"\n"))
        .and_then(|_| stream.flush())
        .map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    let response: RemoteResponse =
        serde_json::from_str(line.trim()).map_err(|error| format!("bad response: {error}"))?;
    verify_response(secret, &expect_host, &challenge, &response)
        .map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::json!({
            "schema": "aegis.fleet.remote_verify.v1",
            "verified": true,
            "host_uuid": response.host_uuid,
            "callsign": response.callsign,
            "nonce": response.nonce,
        })
    );
    Ok(())
}

fn parse_host(args: &[String], secret: &[u8]) -> Result<RemoteIdentity, String> {
    let host_uuid =
        optional(args, "--host-uuid").ok_or_else(|| "missing --host-uuid".to_owned())?;
    let callsign = optional(args, "--callsign").ok_or_else(|| "missing --callsign".to_owned())?;
    RemoteIdentity::new(host_uuid, callsign, secret).map_err(|error| error.to_string())
}

fn optional(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|argument| argument == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}
