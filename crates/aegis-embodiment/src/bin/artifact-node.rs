//! Standalone artifact node: serves the authenticated remote-artifact protocol.
//!
//! This executable wraps [`aegis_embodiment::RemoteArtifactServer`] around one
//! Core's existing embodiment store. It answers read-only fetch requests for
//! live assets that the Core already owns. It performs no replication,
//! synchronization, write, or delete operations.
//!
//! ```text
//! artifact-node <bind-addr> <storage-root> <core-uuid>
//! ```
//!
//! The shared secret is read from the `AEGIS_ARTIFACT_SECRET` environment
//! variable only. It is never accepted as an argument and never logged. The
//! bound address is printed once on standard output so a supervising process can
//! discover an ephemeral port; the secret is not part of that banner.

use std::error::Error;
use std::io::{BufWriter, Write};
use std::net::TcpListener;

use aegis_embodiment::{EmbodimentStore, RemoteArtifactServer, MIN_SECRET_BYTES};

/// Environment variable that carries the shared secret.
pub const SECRET_ENV: &str = "AEGIS_ARTIFACT_SECRET";

const USAGE: &str = "usage: artifact-node <bind-addr> <storage-root> <core-uuid>";

fn main() {
    if let Err(error) = run() {
        eprintln!("artifact-node: {error}");
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
    if args.len() != 3 {
        return Err(format!("expected 3 arguments; {USAGE}").into());
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
    if secret.len() < MIN_SECRET_BYTES {
        return Err(format!("{SECRET_ENV} must be at least {MIN_SECRET_BYTES} bytes").into());
    }

    let store = EmbodimentStore::open(&args[1], &args[2])?;
    let server = RemoteArtifactServer::new(&store, secret.as_bytes())?;
    let listener = TcpListener::bind(&args[0])?;
    let address = listener.local_addr()?;

    {
        let mut banner = BufWriter::new(std::io::stdout());
        writeln!(
            banner,
            "artifact-node listening on {address} core {}",
            store.core_uuid()
        )?;
        banner.flush()?;
    }

    for accepted in listener.incoming() {
        match accepted {
            Ok(stream) => {
                if let Err(error) = server.serve_once(stream) {
                    eprintln!("artifact-node: request failed: {error}");
                }
            }
            Err(error) => eprintln!("artifact-node: accept failed: {error}"),
        }
    }
    Ok(())
}
