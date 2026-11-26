use aegis_fleet::{
    load_frame_time_report, parse_frame_times_csv, parse_nvidia_smi_line, render_summary_csv,
    render_summary_report, summarize_telemetry, TelemetrySeries, WorkloadClass,
    PERFORMANCE_SUMMARY_SCHEMA,
};

#[test]
fn nvidia_smi_rows_feed_the_summary_and_csv() {
    let samples = [
        parse_nvidia_smi_line("NVIDIA GeForce RTX 3080 Ti, 4, 1970, 12288, 60").expect("row 1"),
        parse_nvidia_smi_line("NVIDIA GeForce RTX 3080 Ti, 2, 1910, 12288, 58").expect("row 2"),
    ];
    let summary = summarize_telemetry(&samples);
    assert_eq!(summary.schema, PERFORMANCE_SUMMARY_SCHEMA);
    assert_eq!(summary.sample_count, 2);
    assert_eq!(summary.utilization_max_percent, Some(4));
    assert_eq!(summary.free_vram_max_mb, Some(10378));
    let csv = render_summary_csv(&summary);
    assert_eq!(csv.lines().count(), 2);

    let series = TelemetrySeries::new(samples.to_vec());
    let classification = series.classification();
    assert_eq!(classification.class, WorkloadClass::Idle);
    assert!(render_summary_report(&summary, &classification).contains("classification: idle"));
}

#[test]
fn csv_capture_is_summarized_only_when_present() {
    let csv = "timestamp_ms,frame_time_ms\n0,16\n16,16\n32,40\n";
    let report = parse_frame_times_csv(csv).expect("report");
    assert!(report.available);
    assert_eq!(report.sample_count, 3);
    assert_eq!(report.max_frame_time_ms, Some(40.0));

    let missing = load_frame_time_report(None);
    assert!(!missing.available);
    assert!(missing.avg_fps.is_none());
}

#[test]
fn live_gpu_snapshot_when_enabled() {
    if std::env::var("AEGIS_GPU_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping live GPU snapshot: set AEGIS_GPU_LIVE=1");
        return;
    }
    let sample = aegis_fleet::GpuProbe::probe(&aegis_fleet::NvidiaSmiProbe::new());
    if !sample.available {
        eprintln!("GPU snapshot unavailable: {:?}", sample.error);
        return;
    }
    let summary = summarize_telemetry(&[sample.clone()]);
    eprintln!(
        "live GPU snapshot (not a benchmark):\n{}",
        render_summary_report(
            &summary,
            &TelemetrySeries::new(vec![sample]).classification()
        )
    );
    assert_eq!(summary.sample_count, 1);
}
