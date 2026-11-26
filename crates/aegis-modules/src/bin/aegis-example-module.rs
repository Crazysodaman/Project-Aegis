//! Minimal conforming Aegis module process.
//!
//! Contract implemented (documented for the module SDK):
//!
//! * Read `--module-id <id>` and `--control <directory>`.
//! * While running, write `<control>/<id>.heartbeat` at least once per second.
//! * Exit cleanly when `<control>/<id>.stop` appears.
//! * Write diagnostics to `<control>/<id>.log`.
//!
//! Test-only switches:
//! * `--fail-after-ms <n>` exits nonzero after the given delay.
//! * `--ignore-stop` ignores the stop sentinel so the supervisor's forced-kill
//!   path can be exercised.
//! * `--fail-once-ms <n>` exits nonzero once after the given delay, leaving a
//!   marker so a recovered run keeps going. Used to test module recovery.
//! * `--spawn-child` starts a child worker that ignores the stop sentinel, so
//!   the forced process-tree termination path can be exercised. Pair it with
//!   `--ignore-stop`; the child is only expected to die with the process tree.
//! * `--child-worker` is the internal flag for the spawned child.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("aegis-example-module: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut module_id: Option<String> = None;
    let mut control: Option<PathBuf> = None;
    let mut fail_after: Option<Duration> = None;
    let mut fail_once: Option<Duration> = None;
    let mut ignore_stop = false;
    let mut spawn_child = false;
    let mut child_worker = false;
    let mut tool: Option<String> = None;
    let mut tool_input: Option<PathBuf> = None;
    let mut tool_output: Option<PathBuf> = None;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--module-id" => {
                index += 1;
                module_id = args.get(index).cloned();
            }
            "--control" => {
                index += 1;
                control = args.get(index).map(PathBuf::from);
            }
            "--fail-after-ms" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--fail-after-ms needs a value".to_owned())?;
                let millis = value
                    .parse::<u64>()
                    .map_err(|_| "--fail-after-ms must be an integer".to_owned())?;
                fail_after = Some(Duration::from_millis(millis));
            }
            "--fail-once-ms" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--fail-once-ms needs a value".to_owned())?;
                let millis = value
                    .parse::<u64>()
                    .map_err(|_| "--fail-once-ms must be an integer".to_owned())?;
                fail_once = Some(Duration::from_millis(millis));
            }
            "--ignore-stop" => ignore_stop = true,
            "--spawn-child" => spawn_child = true,
            "--child-worker" => child_worker = true,
            "--aegis-tool" => {
                index += 1;
                tool = args.get(index).cloned();
            }
            "--input" => {
                index += 1;
                tool_input = args.get(index).map(PathBuf::from);
            }
            "--output" => {
                index += 1;
                tool_output = args.get(index).map(PathBuf::from);
            }
            _ => {}
        }
        index += 1;
    }

    if child_worker {
        // The worker never observes the stop sentinel and never exits on its
        // own. The supervisor's forced stop must terminate it through the
        // process tree.
        loop {
            thread::sleep(Duration::from_millis(250));
        }
    }

    if let Some(tool) = tool {
        let input = tool_input.ok_or_else(|| "missing --input".to_owned())?;
        let output = tool_output.ok_or_else(|| "missing --output".to_owned())?;
        return run_tool(&tool, &input, &output);
    }

    let module_id = module_id.ok_or_else(|| "missing --module-id".to_owned())?;
    let control = control.ok_or_else(|| "missing --control".to_owned())?;
    fs::create_dir_all(&control).map_err(|error| format!("control directory: {error}"))?;

    let heartbeat = control.join(format!("{module_id}.heartbeat"));
    let stop = control.join(format!("{module_id}.stop"));
    let log = control.join(format!("{module_id}.log"));
    let fail_once_marker = control.join(format!("{module_id}.failed-once"));
    let _ = fs::remove_file(&stop);
    append_log(&log, "module process started");

    if spawn_child {
        let executable = std::env::current_exe()
            .map_err(|error| format!("resolve example module executable: {error}"))?;
        let child = std::process::Command::new(executable)
            .arg("--child-worker")
            .arg("--module-id")
            .arg(&module_id)
            .arg("--control")
            .arg(&control)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| format!("spawn child worker: {error}"))?;
        fs::write(
            control.join(format!("{module_id}.child")),
            child.id().to_string(),
        )
        .map_err(|error| format!("write child pid: {error}"))?;
        // The worker is intentionally independent of this handle; the
        // supervisor terminates it via process-tree cleanup.
        drop(child);
    }

    let started = Instant::now();
    loop {
        if let Some(limit) = fail_after {
            if started.elapsed() >= limit {
                append_log(&log, "module process simulated failure");
                return Err("simulated module failure".to_owned());
            }
        }
        if let Some(limit) = fail_once {
            if started.elapsed() >= limit && !fail_once_marker.exists() {
                let _ = fs::write(&fail_once_marker, b"1");
                append_log(&log, "module process simulated one-time failure");
                return Err("simulated one-time module failure".to_owned());
            }
        }
        if !ignore_stop && stop.exists() {
            append_log(&log, "module observed stop sentinel");
            let _ = fs::remove_file(&stop);
            return Ok(());
        }

        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let payload = serde_json::json!({
            "module_id": module_id,
            "pid": std::process::id(),
            "timestamp_ms": timestamp_ms,
        });
        let temporary = heartbeat.with_extension("heartbeat.tmp");
        if fs::write(&temporary, payload.to_string()).is_ok() {
            let _ = fs::rename(&temporary, &heartbeat);
        }

        thread::sleep(Duration::from_millis(200));
    }
}

fn run_tool(tool: &str, input: &std::path::Path, output: &std::path::Path) -> Result<(), String> {
    let input_value: serde_json::Value = serde_json::from_slice(
        &fs::read(input).map_err(|error| format!("read tool input: {error}"))?,
    )
    .map_err(|error| format!("invalid tool input: {error}"))?;
    let response = match tool {
        "echo" => serde_json::json!({"ok": true, "output": input_value}),
        "fail" => serde_json::json!({"ok": false, "error": "requested module tool failure"}),
        "sleep" => {
            thread::sleep(Duration::from_secs(5));
            serde_json::json!({"ok": true, "output": {"slept": true}})
        }
        other => serde_json::json!({"ok": false, "error": format!("unknown tool: {other}")}),
    };
    fs::write(output, response.to_string())
        .map_err(|error| format!("write tool output: {error}"))?;
    Ok(())
}

fn append_log(path: &std::path::Path, message: &str) {
    use std::io::Write;
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{message}");
    }
}
