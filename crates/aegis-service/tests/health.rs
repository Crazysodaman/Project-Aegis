use std::time::Duration;

use aegis_service::{evaluate_health, HealthInputs, HealthLevel, DEFAULT_HEARTBEAT_TIMEOUT};

#[test]
fn stopped_without_state_is_ok() {
    let inputs = HealthInputs::new(false);
    assert_eq!(inputs.heartbeat_timeout, DEFAULT_HEARTBEAT_TIMEOUT);

    let health = evaluate_health(&inputs);
    assert_eq!(health.level, HealthLevel::Ok);
    assert!(health.is_ok());
    assert!(health.is_healthy());
    assert!(!health.actual_running);
    assert_eq!(health.codes().collect::<Vec<_>>(), ["stopped"]);
}

#[test]
fn fully_running_service_is_ok() {
    let inputs = HealthInputs::new(true)
        .with_recorded_state(true)
        .with_process(4242, true)
        .with_heartbeat_age(Duration::from_secs(1));

    let health = inputs.evaluate();
    assert_eq!(health.level, HealthLevel::Ok);
    assert!(health.actual_running);
    assert_eq!(health.codes().collect::<Vec<_>>(), ["healthy"]);
}

#[test]
fn missing_state_is_an_error() {
    let inputs = HealthInputs::new(true)
        .with_process(4242, true)
        .with_heartbeat_age(Duration::from_secs(1));

    let health = evaluate_health(&inputs);
    assert_eq!(health.level, HealthLevel::Error);
    assert!(!health.is_healthy());
    assert!(!health.actual_running);
    assert!(health.codes().any(|code| code == "state-missing"));
}

#[test]
fn dead_process_is_an_error() {
    let inputs = HealthInputs::new(true)
        .with_recorded_state(true)
        .with_process(4242, false)
        .with_heartbeat_age(Duration::from_secs(1));

    let health = evaluate_health(&inputs);
    assert_eq!(health.level, HealthLevel::Error);
    assert!(health.codes().any(|code| code == "pid-dead"));
}

#[test]
fn missing_pid_is_an_error() {
    let inputs = HealthInputs::new(true)
        .with_recorded_state(true)
        .with_heartbeat_age(Duration::from_secs(1));

    let health = evaluate_health(&inputs);
    assert_eq!(health.level, HealthLevel::Error);
    assert!(health.codes().any(|code| code == "pid-missing"));
}

#[test]
fn missing_heartbeat_is_a_warning() {
    let inputs = HealthInputs::new(true)
        .with_recorded_state(true)
        .with_process(4242, true);

    let health = evaluate_health(&inputs);
    assert_eq!(health.level, HealthLevel::Warn);
    assert!(health.is_healthy());
    assert!(health.codes().any(|code| code == "heartbeat-missing"));
}

#[test]
fn stale_heartbeat_is_an_error() {
    let inputs = HealthInputs::new(true)
        .with_recorded_state(true)
        .with_process(4242, true)
        .with_heartbeat_timeout(Duration::from_secs(5))
        .with_heartbeat_age(Duration::from_secs(6));

    let health = evaluate_health(&inputs);
    assert_eq!(health.level, HealthLevel::Error);
    assert!(health.codes().any(|code| code == "heartbeat-stale"));
}

#[test]
fn stopping_desired_but_process_alive_is_a_warning() {
    let inputs = HealthInputs::new(false)
        .with_recorded_state(true)
        .with_process(4242, true)
        .with_heartbeat_age(Duration::from_secs(1));

    let health = evaluate_health(&inputs);
    assert_eq!(health.level, HealthLevel::Warn);
    assert!(health.actual_running);
    assert_eq!(health.codes().collect::<Vec<_>>(), ["unexpected-running"]);
}

#[test]
fn stopping_desired_with_stale_state_is_a_warning() {
    let inputs = HealthInputs::new(false)
        .with_recorded_state(true)
        .with_process(4242, false);

    let health = evaluate_health(&inputs);
    assert_eq!(health.level, HealthLevel::Warn);
    assert!(!health.actual_running);
    assert_eq!(health.codes().collect::<Vec<_>>(), ["stale-state"]);
}

#[test]
fn worst_reason_wins() {
    let inputs = HealthInputs::new(true)
        .with_recorded_state(false)
        .with_process(4242, false)
        .with_heartbeat_age(Duration::from_secs(120))
        .with_heartbeat_timeout(Duration::from_secs(5));

    let health = evaluate_health(&inputs);
    assert_eq!(health.level, HealthLevel::Error);
    let codes = health.codes().collect::<Vec<_>>();
    assert!(codes.contains(&"state-missing"));
    assert!(codes.contains(&"pid-dead"));
    assert!(codes.contains(&"heartbeat-stale"));
}

#[test]
fn zero_heartbeat_timeout_treats_any_age_as_stale() {
    let inputs = HealthInputs::new(true)
        .with_recorded_state(true)
        .with_process(4242, true)
        .with_heartbeat_age(Duration::from_millis(1))
        .with_heartbeat_timeout(Duration::ZERO);

    let health = evaluate_health(&inputs);
    assert_eq!(health.level, HealthLevel::Error);
    assert!(health.codes().any(|code| code == "heartbeat-stale"));
}
