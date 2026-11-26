use std::cell::Cell;
use std::time::Duration;

use aegis_fleet::{
    capture_telemetry_series, compare_conditions, parse_frame_times_csv, parse_nvidia_smi_line,
    render_comparison_report, render_pressure_report, summarize_frame_times, summarize_pressure,
    CaptureConfig, ConditionLabel, DeltaStatus, GpuProbe, GpuTelemetry, LabeledMeasurement,
    MetricKind, TelemetrySeries, COMPARISON_REPORT_SCHEMA, PRESSURE_REPORT_SCHEMA,
};

fn series(rows: &[&str]) -> TelemetrySeries {
    TelemetrySeries::new(
        rows.iter()
            .map(|row| parse_nvidia_smi_line(row).expect("nvidia-smi row"))
            .collect(),
    )
}

#[test]
fn multi_condition_comparison_via_public_api() {
    let idle = LabeledMeasurement::new(
        ConditionLabel::Idle,
        series(&[
            "NVIDIA GeForce RTX 3080 Ti, 4, 1970, 12288, 60",
            "NVIDIA GeForce RTX 3080 Ti, 2, 1910, 12288, 58",
        ]),
    );
    let inference = LabeledMeasurement::new(
        ConditionLabel::Inference,
        series(&[
            "NVIDIA GeForce RTX 3080 Ti, 22, 7000, 12288, 66",
            "NVIDIA GeForce RTX 3080 Ti, 18, 6900, 12288, 67",
        ]),
    )
    .with_frame_times(summarize_frame_times(&[16.0, 17.0, 20.0], "test").expect("frames"));
    let gaming = LabeledMeasurement::new(
        ConditionLabel::Gaming,
        series(&[
            "NVIDIA GeForce RTX 3080 Ti, 70, 10000, 12288, 78",
            "NVIDIA GeForce RTX 3080 Ti, 92, 10500, 12288, 80",
        ]),
    );

    let report = compare_conditions(ConditionLabel::Idle, &[idle, inference, gaming]);
    assert_eq!(report.schema, COMPARISON_REPORT_SCHEMA);
    assert!(report.baseline_present);
    assert_eq!(report.conditions.len(), 3);

    let gaming_report = &report.conditions[2];
    assert_eq!(gaming_report.label, ConditionLabel::Gaming);
    let used_delta = gaming_report
        .deltas
        .iter()
        .find(|delta| delta.metric == MetricKind::VramUsedAvgMb)
        .expect("used vram delta");
    assert_eq!(used_delta.status, DeltaStatus::Available);
    assert!((used_delta.absolute_delta.expect("delta") - 8310.0).abs() < 1e-9);

    let gaming_fps = gaming_report
        .deltas
        .iter()
        .find(|delta| delta.metric == MetricKind::AvgFps)
        .expect("fps delta");
    assert_eq!(gaming_fps.status, DeltaStatus::ConditionUnavailable);

    let inference_fps = report.conditions[1]
        .deltas
        .iter()
        .find(|delta| delta.metric == MetricKind::AvgFps)
        .expect("fps delta");
    assert_eq!(inference_fps.status, DeltaStatus::BaselineUnavailable);

    let text = render_comparison_report(&report);
    assert!(text.contains("baseline: idle"));
    assert!(text.contains("not ground truth"));
    assert!(text.contains("delta vram_used_avg_mb: +8310"));
}

#[test]
fn frame_time_fixture_produces_lows_and_percentiles() {
    let csv = "timestamp_ms,frame_time_ms\n0,16\n16,16\n32,16\n48,16\n64,100\n";
    let report = parse_frame_times_csv(csv).expect("frame report");
    assert!(report.available);
    assert_eq!(report.sample_count, 5);
    assert_eq!(report.max_frame_time_ms, Some(100.0));
    assert_eq!(report.p95_frame_time_ms, Some(100.0));
    assert_eq!(report.p99_frame_time_ms, Some(100.0));
    assert_eq!(report.one_percent_low_fps, Some(10.0));
    assert_eq!(report.point_one_percent_low_fps, Some(10.0));
}

#[derive(Debug)]
struct SyntheticProbe {
    utilizations: Vec<u8>,
    index: Cell<usize>,
}

impl GpuProbe for SyntheticProbe {
    fn probe(&self) -> GpuTelemetry {
        let index = self.index.get();
        let utilization = self.utilizations[index.min(self.utilizations.len() - 1)];
        self.index.set(index + 1);
        let mut sample =
            parse_nvidia_smi_line("Synthetic GPU, 0, 1000, 12288, 50").expect("base row");
        sample.utilization_percent = Some(utilization);
        sample
    }
}

#[test]
fn pressure_episode_reports_growth_retention_and_recovery_via_public_api() {
    let idle = series(&[
        "NVIDIA GeForce RTX 3080 Ti, 4, 1970, 12288, 60",
        "NVIDIA GeForce RTX 3080 Ti, 5, 1990, 12288, 60",
    ]);
    let during = series(&[
        "NVIDIA GeForce RTX 3080 Ti, 38, 7600, 12288, 71",
        "NVIDIA GeForce RTX 3080 Ti, 44, 7700, 12288, 73",
    ]);
    let after = series(&["NVIDIA GeForce RTX 3080 Ti, 6, 6300, 12288, 62"]);
    let unloaded = series(&["NVIDIA GeForce RTX 3080 Ti, 4, 2010, 12288, 58"]);

    let report = summarize_pressure(&idle, &during, &after, &unloaded);
    assert_eq!(report.schema, PRESSURE_REPORT_SCHEMA);
    assert!((report.inference_vram_growth_mb.expect("growth") - 5670.0).abs() < 1e-9);
    assert!((report.retained_vram_mb.expect("retained") - 4320.0).abs() < 1e-9);
    assert!((report.recovered_vram_mb.expect("recovered") - 4290.0).abs() < 1e-9);

    let text = render_pressure_report(&report);
    assert!(text.contains("not gameplay"));
    assert!(text.contains("inference VRAM growth: 5670.00 MB"));
}

#[test]
fn live_capture_helper_is_pure_and_uses_supplied_wait() {
    let probe = SyntheticProbe {
        utilizations: vec![5, 25, 75],
        index: Cell::new(0),
    };
    let mut waits = Vec::new();
    let series = capture_telemetry_series(
        &probe,
        CaptureConfig::new(3, Duration::from_millis(250)),
        |duration| waits.push(duration),
    )
    .expect("capture");

    assert_eq!(series.samples.len(), 3);
    assert_eq!(
        series
            .samples
            .iter()
            .map(|sample| sample.utilization_percent.expect("utilization"))
            .collect::<Vec<_>>(),
        vec![5, 25, 75]
    );
    assert_eq!(waits, vec![Duration::from_millis(250); 2]);
    assert_eq!(series.summary().utilization_avg_percent, Some(35.0));
}
