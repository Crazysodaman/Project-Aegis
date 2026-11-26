use aegis_fleet::gaming::GamingSignal;
use aegis_fleet::gpu::{
    evaluate_admission_with_gaming, GpuPolicy, GpuTelemetry, GPU_TELEMETRY_SCHEMA,
};
use aegis_fleet::supervisor::{
    placement_gate, InferenceQueue, PlacementGate, QueueDecision, QueueRejection,
    INFERENCE_QUEUE_SCHEMA, MAX_INFERENCE_QUEUE_DEPTH, MAX_INFERENCE_REQUEST_ID_LEN,
};

fn telemetry(utilization: u8, used: u64, total: u64, temperature: u8) -> GpuTelemetry {
    GpuTelemetry {
        schema: GPU_TELEMETRY_SCHEMA.to_owned(),
        available: true,
        name: Some("Synthetic GPU".to_owned()),
        utilization_percent: Some(utilization),
        memory_used_mb: Some(used),
        memory_total_mb: Some(total),
        temperature_c: Some(temperature),
        source: "synthetic".to_owned(),
        checked_at: "2026-01-01T00:00:00Z".to_owned(),
        error: None,
    }
}

#[test]
fn placement_gate_reflects_admission_and_gaming_via_public_module_paths() {
    let policy = GpuPolicy::default();
    let idle = telemetry(3, 2000, 12288, 50);
    let busy = telemetry(90, 2000, 12288, 50);
    let gaming = GamingSignal::detected("test", vec!["gears5".to_owned()]);
    let none = GamingSignal::none("test");
    let unavailable = GamingSignal::unavailable("test", "no process list");

    let idle_admission = evaluate_admission_with_gaming(&policy, &idle, None);
    assert!(idle_admission.admit);
    assert_eq!(placement_gate(&idle_admission, None), PlacementGate::Open);
    assert_eq!(
        placement_gate(&idle_admission, Some(&none)),
        PlacementGate::Open
    );

    let busy_admission = evaluate_admission_with_gaming(&policy, &busy, None);
    assert!(!busy_admission.admit);
    assert_eq!(
        placement_gate(&busy_admission, None),
        PlacementGate::AdmissionDenied
    );
    assert_eq!(
        placement_gate(&busy_admission, Some(&gaming)),
        PlacementGate::GamingActive,
        "gaming wins over admission for a stable reason"
    );
    assert_eq!(
        placement_gate(&idle_admission, Some(&gaming)),
        PlacementGate::GamingActive
    );
    assert_eq!(
        placement_gate(&idle_admission, Some(&unavailable)),
        PlacementGate::Open,
        "an unavailable probe is not a denial"
    );
}

#[test]
fn bounded_queue_flows_through_defer_and_dequeue() {
    assert_eq!(INFERENCE_QUEUE_SCHEMA, "aegis.fleet.inference_queue.v1");
    let mut queue = InferenceQueue::new(2).expect("bounded queue");
    assert_eq!(queue.max_depth(), 2);
    assert!(queue.is_empty());

    assert_eq!(
        queue.defer("embed-a", 4, PlacementGate::GamingActive),
        QueueDecision::Deferred {
            id: "embed-a".to_owned(),
            priority: 4,
            reason: PlacementGate::GamingActive,
            depth: 1,
        }
    );
    assert_eq!(queue.deferred_len(), 1);
    assert_eq!(
        queue.dequeue(PlacementGate::GamingActive),
        QueueDecision::Held {
            reason: PlacementGate::GamingActive,
            pending: 1,
        }
    );
    assert_eq!(queue.len(), 1, "held dequeue must not mutate the queue");

    assert_eq!(
        queue.enqueue("embed-b", 8),
        QueueDecision::Enqueued {
            id: "embed-b".to_owned(),
            priority: 8,
            depth: 2,
        }
    );
    assert_eq!(queue.ready_len(), 1);
    assert_eq!(
        queue.enqueue("embed-c", 9),
        QueueDecision::Rejected {
            reason: QueueRejection::QueueFull
        }
    );

    assert_eq!(
        queue.dequeue(PlacementGate::Open),
        QueueDecision::Dequeued {
            id: "embed-b".to_owned(),
            priority: 8,
            remaining: 1,
        }
    );
    assert_eq!(
        queue.dequeue(PlacementGate::Open),
        QueueDecision::Dequeued {
            id: "embed-a".to_owned(),
            priority: 4,
            remaining: 0,
        }
    );
    assert_eq!(queue.dequeue(PlacementGate::Open), QueueDecision::Empty);
}

#[test]
fn queue_is_deterministic_and_serializable() {
    fn run() -> Vec<QueueDecision> {
        let mut queue = InferenceQueue::new(8).expect("queue");
        queue.enqueue("low", 1);
        queue.enqueue("high", 9);
        queue.defer("blocked", 7, PlacementGate::AdmissionDenied);
        queue.enqueue("mid", 5);
        vec![
            queue.dequeue(PlacementGate::Open),
            queue.dequeue(PlacementGate::Open),
            queue.dequeue(PlacementGate::Open),
            queue.dequeue(PlacementGate::Open),
        ]
    }

    let first = run();
    let second = run();
    assert_eq!(first, second);

    let encoded = serde_json::to_string(&first).expect("serialize");
    let decoded: Vec<QueueDecision> = serde_json::from_str(&encoded).expect("deserialize");
    assert_eq!(first, decoded);
    assert_eq!(
        first
            .iter()
            .filter_map(|decision| match decision {
                QueueDecision::Dequeued { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec!["high", "blocked", "mid", "low"]
    );
}

#[test]
fn invalid_ids_and_duplicates_are_rejected_without_mutation() {
    let mut queue = InferenceQueue::new(MAX_INFERENCE_QUEUE_DEPTH).expect("queue");
    assert!(matches!(
        queue.enqueue("dupe", 3),
        QueueDecision::Enqueued { .. }
    ));
    assert_eq!(
        queue.enqueue("dupe", 3),
        QueueDecision::Duplicate {
            id: "dupe".to_owned()
        }
    );
    assert_eq!(queue.len(), 1);

    for invalid in [
        "",
        "   ",
        "bad\tid",
        &"z".repeat(MAX_INFERENCE_REQUEST_ID_LEN + 1),
    ] {
        assert_eq!(
            queue.enqueue(invalid, 0),
            QueueDecision::Rejected {
                reason: QueueRejection::InvalidId
            }
        );
    }
    assert_eq!(queue.len(), 1);

    assert!(InferenceQueue::new(0).is_err());
    assert!(InferenceQueue::new(MAX_INFERENCE_QUEUE_DEPTH + 1).is_err());
    assert_eq!(
        InferenceQueue::default().max_depth(),
        MAX_INFERENCE_QUEUE_DEPTH
    );
}
