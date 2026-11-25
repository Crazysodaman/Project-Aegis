use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use aegis_contracts::core::validate_core_id;
use aegis_core_runtime::{
    heartbeat_path, prepare_run_dir, stop_sentinel_path, write_heartbeat, Heartbeat,
    HEARTBEAT_INTERVAL_MS,
};
use aegis_core_storage::{CoreDatabase, CoreStorageLayout};
use aegis_safe::SafeStore;
use aegis_storage::StoragePaths;
use chrono::Utc;

const STOP_POLL_MS: u64 = 25;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aegis-core-host: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args =
        parse_args().map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
    let paths = StoragePaths::new(&args.root)?;
    let layout = CoreStorageLayout::new(paths.root())?;
    let core_uuid = validate_core_id(&args.core_id)
        .map(|uuid| uuid.hyphenated().to_string())
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "--core-id must be a valid non-nil UUID",
            )
        })?;
    CoreDatabase::read_snapshot(&layout, &core_uuid)?;

    prepare_run_dir(paths.root())?;
    let heartbeat = heartbeat_path(paths.root(), &core_uuid);
    let sentinel = stop_sentinel_path(paths.root(), &core_uuid);
    let pid = std::process::id();
    let interval = Duration::from_millis(HEARTBEAT_INTERVAL_MS);
    let safe = SafeStore::new(paths.clone());

    loop {
        if safe.stop_status()?.active {
            let _ = fs::remove_file(&heartbeat);
            return Ok(());
        }
        if sentinel.exists() {
            let _ = fs::remove_file(&heartbeat);
            return Ok(());
        }
        let beat = Heartbeat {
            core_uuid: core_uuid.clone(),
            pid,
            timestamp_ms: Utc::now().timestamp_millis(),
        };
        write_heartbeat(&heartbeat, &beat)?;

        let wake = Instant::now() + interval;
        while Instant::now() < wake {
            if safe.stop_status()?.active || sentinel.exists() {
                let _ = fs::remove_file(&heartbeat);
                return Ok(());
            }
            thread::sleep(Duration::from_millis(STOP_POLL_MS));
        }
    }
}

struct Args {
    root: PathBuf,
    core_id: String,
}

fn parse_args() -> Result<Args, String> {
    let mut root = None;
    let mut core_id = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--root" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--root requires a value".to_owned())?;
                root = Some(PathBuf::from(value));
            }
            "--core-id" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--core-id requires a value".to_owned())?;
                core_id = Some(value);
            }
            "--help" | "-h" => {
                println!("Usage: aegis-core-host --root <storage-root> --core-id <core-uuid>");
                std::process::exit(0);
            }
            other => return Err(format!("unexpected argument: {other}")),
        }
    }
    Ok(Args {
        root: root.ok_or_else(|| "--root is required".to_owned())?,
        core_id: core_id.ok_or_else(|| "--core-id is required".to_owned())?,
    })
}
