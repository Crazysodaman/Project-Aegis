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
    let mut ignore_stop = false;
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
            "--ignore-stop" => ignore_stop = true,
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
    let _ = fs::remove_file(&stop);
    append_log(&log, "module process started");

    let started = Instant::now();
    loop {
        if let Some(limit) = fail_after {
            if started.elapsed() >= limit {
                append_log(&log, "module process simulated failure");
                return Err("simulated module failure".to_owned());
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
