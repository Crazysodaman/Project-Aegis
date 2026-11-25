use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use aegis_ipc::IpcServer;
use aegis_platform::PlatformService;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aegis-service-host: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let service = PlatformService::start(&args.root).map_err(|error| error.to_string())?;
    let server = IpcServer::new(Arc::new(service), args.socket);
    server.serve(0).map_err(|error| error.to_string())
}

struct Args {
    root: PathBuf,
    socket: String,
}

fn parse_args() -> Result<Args, String> {
    let mut root = None;
    let mut socket = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--root" => {
                root = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--root requires a value".to_owned())?,
                ));
            }
            "--socket" => {
                socket = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "--socket requires a value".to_owned())?,
                );
            }
            "--help" | "-h" => {
                println!("Usage: aegis-service-host --root <storage-root> --socket <name>");
                std::process::exit(0);
            }
            other => return Err(format!("unexpected argument: {other}")),
        }
    }
    Ok(Args {
        root: root.ok_or_else(|| "--root is required".to_owned())?,
        socket: socket.ok_or_else(|| "--socket is required".to_owned())?,
    })
}
