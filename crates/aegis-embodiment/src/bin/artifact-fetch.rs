//! Minimal client for the authenticated remote-artifact protocol.
//!
//! ```text
//! artifact-fetch <server-addr> <core-uuid> <asset-id> <destination>
//! ```
//!
//! This is a thin wrapper over [`aegis_embodiment::RemoteArtifactClient`]. It
//! verifies the server proof, the Core/asset/nonce binding, and the SHA-256 of
//! the received bytes before writing the destination atomically. It prints the
//! verified digest and byte count on success.
//!
//! The shared secret is read from the `AEGIS_ARTIFACT_SECRET` environment
//! variable only and is never accepted as an argument or logged.

use std::error::Error;

use aegis_embodiment::{RemoteArtifactClient, MIN_SECRET_BYTES};

/// Environment variable that carries the shared secret.
pub const SECRET_ENV: &str = "AEGIS_ARTIFACT_SECRET";

const USAGE: &str = "usage: artifact-fetch <server-addr> <core-uuid> <asset-id> <destination>";

fn main() {
    if let Err(error) = run() {
        eprintln!("artifact-fetch: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{USAGE}");
        println!("secret: set {SECRET_ENV} (at least {MIN_SECRET_BYTES} bytes); never passed as an argument");
        return Ok(());
    }
    if args.len() != 4 {
        return Err(format!("expected 4 arguments; {USAGE}").into());
    }

    let secret = match std::env::var(SECRET_ENV) {
        Ok(secret) => secret,
        Err(_) => {
            return Err(format!(
                "{SECRET_ENV} must be set; the secret is never passed as an argument"
            )
            .into())
        }
    };

    let client = RemoteArtifactClient::new(secret.as_bytes())?;
    let artifact = client.fetch_to_path(&args[0], &args[1], &args[2], &args[3])?;
    println!(
        "fetched {} bytes sha256={} core={} asset={}",
        artifact.bytes.len(),
        artifact.sha256,
        artifact.core_uuid,
        artifact.asset_id
    );
    Ok(())
}
