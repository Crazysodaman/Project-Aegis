use std::time::Duration;

use aegis_service::supervision::{
    plan_restart, Backoff, RestartCondition, RestartDecision, RestartPolicy, StopReason,
    SupervisionError, MAX_RESTART_ATTEMPTS,
};

fn policy(condition: RestartCondition, delay: Duration, max_attempts: u32) -> RestartPolicy {
    RestartPolicy::new(condition, delay, max_attempts).expect("valid policy")
}

#[test]
fn fixed_backoff_is_constant_and_capped() {
    let fixed = policy(RestartCondition::Always, Duration::from_secs(2), 0);
    assert_eq!(fixed.delay_for_attempt(0).unwrap(), Duration::from_secs(2));
    assert_eq!(fixed.delay_for_attempt(7).unwrap(), Duration::from_secs(2));

    let capped = fixed.with_max_delay(Duration::from_secs(2));
    assert_eq!(
        capped.delay_for_attempt(99).unwrap(),
        Duration::from_secs(2)
    );
}

#[test]
fn linear_backoff_grows_and_is_capped() {
    let linear = policy(RestartCondition::Always, Duration::from_millis(100), 0)
        .with_backoff(Backoff::Linear {
            step: Duration::from_millis(50),
        })
        .with_max_delay(Duration::from_millis(300));

    assert_eq!(
        linear.delay_for_attempt(0).unwrap(),
        Duration::from_millis(100)
    );
    assert_eq!(
        linear.delay_for_attempt(2).unwrap(),
        Duration::from_millis(200)
    );
    assert_eq!(
        linear.delay_for_attempt(10).unwrap(),
        Duration::from_millis(300)
    );
}

#[test]
fn exponential_backoff_grows_and_is_capped() {
    let exponential = policy(RestartCondition::Always, Duration::from_millis(100), 0)
        .with_backoff(Backoff::Exponential { multiplier: 2 })
        .with_max_delay(Duration::from_millis(1000));

    assert_eq!(
        exponential.delay_for_attempt(0).unwrap(),
        Duration::from_millis(100)
    );
    assert_eq!(
        exponential.delay_for_attempt(1).unwrap(),
        Duration::from_millis(200)
    );
    assert_eq!(
        exponential.delay_for_attempt(2).unwrap(),
        Duration::from_millis(400)
    );
    assert_eq!(
        exponential.delay_for_attempt(4).unwrap(),
        Duration::from_millis(1000)
    );
    assert_eq!(
        exponential.delay_for_attempt(40).unwrap(),
        Duration::from_millis(1000)
    );
}

#[test]
fn on_failure_restarts_failure_and_stops_on_success() {
    let on_failure = policy(RestartCondition::OnFailure, Duration::from_secs(1), 5);

    let failure = on_failure
        .plan_restart(Some(3), 0, Duration::from_secs(2))
        .expect("plan");
    assert_eq!(
        failure,
        RestartDecision::Restart {
            attempt: 1,
            delay: Duration::from_secs(1),
        }
    );

    let signal = on_failure
        .plan_restart(None, 0, Duration::from_secs(2))
        .expect("plan");
    assert!(matches!(
        signal,
        RestartDecision::Restart { attempt: 1, .. }
    ));

    let success = on_failure
        .plan_restart(Some(0), 2, Duration::from_secs(2))
        .expect("plan");
    assert_eq!(
        success,
        RestartDecision::Stop {
            reason: StopReason::SuccessfulExit,
            attempts_reset: true,
        }
    );
}

#[test]
fn always_restarts_even_on_success_and_never_stops() {
    let always = policy(RestartCondition::Always, Duration::from_secs(1), 5);
    let decision = always
        .plan_restart(Some(0), 0, Duration::from_secs(1))
        .expect("plan");
    assert_eq!(
        decision,
        RestartDecision::Restart {
            attempt: 1,
            delay: Duration::from_secs(1),
        }
    );

    let never = policy(RestartCondition::Never, Duration::from_secs(1), 5);
    let decision = never
        .plan_restart(Some(9), 0, Duration::from_secs(1))
        .expect("plan");
    assert_eq!(
        decision,
        RestartDecision::Stop {
            reason: StopReason::PolicyNever,
            attempts_reset: false,
        }
    );
}

#[test]
fn max_attempts_exhausts_the_budget() {
    let policy = policy(RestartCondition::Always, Duration::from_secs(1), 3);
    let zero_window = Duration::ZERO;

    assert_eq!(
        policy.plan_restart(Some(1), 2, zero_window).unwrap(),
        RestartDecision::Restart {
            attempt: 3,
            delay: Duration::from_secs(1),
        }
    );
    assert_eq!(
        policy.plan_restart(Some(1), 3, zero_window).unwrap(),
        RestartDecision::Exhausted { attempts: 3 }
    );
}

#[test]
fn successful_exit_resets_the_attempt_counter() {
    let policy = policy(RestartCondition::Always, Duration::from_secs(1), 10);
    let decision = policy
        .plan_restart(Some(0), 6, Duration::from_secs(1))
        .expect("plan");
    assert_eq!(
        decision,
        RestartDecision::Restart {
            attempt: 1,
            delay: Duration::from_secs(1),
        }
    );
}

#[test]
fn long_runtime_resets_the_attempt_counter() {
    let policy = policy(RestartCondition::OnFailure, Duration::from_secs(1), 10)
        .with_reset_after(Duration::from_secs(30));
    let decision = policy
        .plan_restart(Some(1), 6, Duration::from_secs(30))
        .expect("plan");
    assert_eq!(
        decision,
        RestartDecision::Restart {
            attempt: 1,
            delay: Duration::from_secs(1),
        }
    );
}

#[test]
fn free_function_matches_method() {
    let policy = policy(RestartCondition::OnFailure, Duration::from_millis(250), 4);
    assert_eq!(
        plan_restart(&policy, Some(2), 1, Duration::ZERO).unwrap(),
        policy.plan_restart(Some(2), 1, Duration::ZERO).unwrap()
    );
}

#[test]
fn invalid_policies_are_rejected() {
    assert_eq!(
        RestartPolicy::new(RestartCondition::Always, Duration::ZERO, 1),
        Err(SupervisionError::ZeroDelay)
    );
    assert_eq!(
        RestartPolicy::new(RestartCondition::Always, Duration::from_secs(1), 1)
            .unwrap()
            .with_max_delay(Duration::ZERO)
            .validate(),
        Err(SupervisionError::ZeroMaxDelay)
    );
    assert_eq!(
        RestartPolicy::new(RestartCondition::Always, Duration::from_secs(10), 1)
            .unwrap()
            .with_max_delay(Duration::from_secs(1))
            .validate(),
        Err(SupervisionError::DelayExceedsMax {
            delay: Duration::from_secs(10),
            max_delay: Duration::from_secs(1),
        })
    );
    assert_eq!(
        RestartPolicy::new(
            RestartCondition::Always,
            Duration::from_secs(1),
            MAX_RESTART_ATTEMPTS + 1,
        ),
        Err(SupervisionError::TooManyAttempts(MAX_RESTART_ATTEMPTS + 1))
    );
    assert_eq!(
        RestartPolicy::new(RestartCondition::Always, Duration::from_secs(1), 1)
            .unwrap()
            .with_backoff(Backoff::Linear {
                step: Duration::ZERO,
            })
            .validate(),
        Err(SupervisionError::ZeroBackoffStep)
    );
    assert_eq!(
        RestartPolicy::new(RestartCondition::Always, Duration::from_secs(1), 1)
            .unwrap()
            .with_backoff(Backoff::Exponential { multiplier: 1 })
            .validate(),
        Err(SupervisionError::InvalidBackoffMultiplier(1))
    );
    assert_eq!(
        RestartPolicy::new(RestartCondition::Always, Duration::from_secs(1), 1)
            .unwrap()
            .with_reset_after(Duration::ZERO)
            .validate(),
        Err(SupervisionError::ZeroResetAfter)
    );
}

#[test]
fn invalid_policy_blocks_planning() {
    let mut policy = policy(RestartCondition::Always, Duration::from_secs(1), 2);
    policy.delay = Duration::ZERO;
    assert_eq!(
        policy.plan_restart(Some(1), 0, Duration::ZERO),
        Err(SupervisionError::ZeroDelay)
    );
    assert_eq!(
        policy.delay_for_attempt(0),
        Err(SupervisionError::ZeroDelay)
    );
}
