//! Read-only measurement harness for multi-condition GPU comparison.
//!
//! Three subcommands:
//!
//! * `capture --samples N --interval-ms M` samples `nvidia-smi` N times and
//!   prints a summary. It never loads a model, starts a game, or changes policy.
//! * `compare --baseline <label> --idle <csv> --inference <csv> --gaming <csv>`
//!   ingests owner-captured `nvidia-smi` snapshot files plus optional frame-time
//!   CSVs and prints a deterministic comparison report.
//! * `pressure --idle <csv> --during <csv> --after <csv> --unloaded <csv>`
//!   ingests the four phases of an inference-pressure episode and prints the
//!   VRAM growth/retention/recovery ledger. It does not start inference itself.
//!
//! This binary is a thin wrapper over the pure library API. It is not a
//! benchmark and produces no data without an actual capture file or probe.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use aegis_fleet::measurement::{
    capture_telemetry_series, compare_conditions, render_comparison_report, render_pressure_report,
    summarize_pressure, CaptureConfig, ConditionLabel, LabeledMeasurement,
};
use aegis_fleet::performance::load_frame_time_report;
use aegis_fleet::{parse_nvidia_smi_line, render_summary_report, NvidiaSmiProbe, TelemetrySeries};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("capture") => capture(&args),
        Some("compare") => compare(&args),
        Some("pressure") => pressure(&args),
        _ => Err(
            "usage:\n  aegis-fleet-measure capture --samples N --interval-ms M [--json]\n  \
             aegis-fleet-measure compare --baseline idle|inference|gaming \
             [--idle <csv>] [--inference <csv>] [--gaming <csv>] \
             [--idle-frames <csv>] [--inference-frames <csv>] [--gaming-frames <csv>] [--json]\n  \
             aegis-fleet-measure pressure --idle <csv> --during <csv> --after <csv> \
             --unloaded <csv> [--json]"
                .to_owned(),
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aegis-fleet-measure: {error}");
            ExitCode::FAILURE
        }
    }
}

fn capture(args: &[String]) -> Result<(), String> {
    let samples = value(args, "--samples")
        .ok_or_else(|| "missing --samples".to_owned())?
        .parse::<u32>()
        .map_err(|_| "invalid --samples".to_owned())?;
    let interval_ms = value(args, "--interval-ms")
        .ok_or_else(|| "missing --interval-ms".to_owned())?
        .parse::<u64>()
        .map_err(|_| "invalid --interval-ms".to_owned())?;
    let config = CaptureConfig::new(samples, Duration::from_millis(interval_ms));
    let probe = NvidiaSmiProbe::new();
    let series = capture_telemetry_series(&probe, config, std::thread::sleep)
        .map_err(|error| error.to_string())?;

    if has_flag(args, "--json") {
        let encoded = serde_json::to_string_pretty(&series)
            .map_err(|error| format!("serialize failed: {error}"))?;
        println!("{encoded}");
        return Ok(());
    }

    let summary = series.summary();
    let classification = series.classification();
    println!("Aegis live GPU capture (read-only snapshot, not a benchmark)");
    println!("this tool loaded no model and launched no game");
    println!("{}", render_summary_report(&summary, &classification));
    Ok(())
}

fn compare(args: &[String]) -> Result<(), String> {
    let baseline_label =
        value(args, "--baseline").ok_or_else(|| "missing --baseline".to_owned())?;
    let baseline = ConditionLabel::parse(&baseline_label)
        .ok_or_else(|| format!("invalid --baseline {baseline_label:?}"))?;

    let mut measurements = Vec::new();
    for (label, series_flag, frames_flag) in [
        (ConditionLabel::Idle, "--idle", "--idle-frames"),
        (
            ConditionLabel::Inference,
            "--inference",
            "--inference-frames",
        ),
        (ConditionLabel::Gaming, "--gaming", "--gaming-frames"),
    ] {
        let Some(path) = value(args, series_flag) else {
            if value(args, frames_flag).is_some() {
                return Err(format!("{frames_flag} requires {series_flag}"));
            }
            continue;
        };
        let series = read_nvidia_smi_series(Path::new(&path))?;
        let mut measurement = LabeledMeasurement::new(label, series);
        if let Some(frames_path) = value(args, frames_flag) {
            let report = load_frame_time_report(Some(Path::new(&frames_path)));
            measurement = measurement.with_frame_times(report);
        }
        measurements.push(measurement);
    }

    let report = compare_conditions(baseline, &measurements);
    if has_flag(args, "--json") {
        let encoded = serde_json::to_string_pretty(&report)
            .map_err(|error| format!("serialize failed: {error}"))?;
        println!("{encoded}");
    } else {
        print!("{}", render_comparison_report(&report));
    }
    Ok(())
}

fn pressure(args: &[String]) -> Result<(), String> {
    let idle_path = value(args, "--idle").ok_or_else(|| "missing --idle".to_owned())?;
    let during_path = value(args, "--during").ok_or_else(|| "missing --during".to_owned())?;
    let after_path = value(args, "--after").ok_or_else(|| "missing --after".to_owned())?;
    let unloaded_path = value(args, "--unloaded").ok_or_else(|| "missing --unloaded".to_owned())?;

    let idle = read_nvidia_smi_series(Path::new(&idle_path))?;
    let during = read_nvidia_smi_series(Path::new(&during_path))?;
    let after = read_nvidia_smi_series(Path::new(&after_path))?;
    let unloaded = read_nvidia_smi_series(Path::new(&unloaded_path))?;

    let report = summarize_pressure(&idle, &during, &after, &unloaded);
    if has_flag(args, "--json") {
        let encoded = serde_json::to_string_pretty(&report)
            .map_err(|error| format!("serialize failed: {error}"))?;
        println!("{encoded}");
    } else {
        print!("{}", render_pressure_report(&report));
    }
    Ok(())
}

fn read_nvidia_smi_series(path: &Path) -> Result<TelemetrySeries, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("{}: failed to read: {error}", path.display()))?;
    let mut samples = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let sample = parse_nvidia_smi_line(line)
            .map_err(|error| format!("{}:{}: {error}", path.display(), index + 1))?;
        samples.push(sample);
    }
    if samples.is_empty() {
        return Err(format!("{}: no nvidia-smi rows", path.display()));
    }
    Ok(TelemetrySeries::new(samples))
}

fn value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|argument| argument == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|argument| argument == flag)
}
