use super::*;

#[test]
fn budget_default_values() {
    let budget = Budget::with_defaults();
    assert_eq!(budget.wall_clock_limit, DEFAULT_WALL_CLOCK_SECONDS);
    assert_eq!(budget.bytes_limit, DEFAULT_BYTES_BUDGET);
    assert!(budget.estimable);
    assert_eq!(budget.wall_clock_consumed, 0);
    assert_eq!(budget.bytes_consumed, 0);
}

#[test]
fn budget_custom_constructor() {
    let budget = Budget::new(60, 50_000_000, true);
    assert_eq!(budget.wall_clock_limit, 60);
    assert_eq!(budget.bytes_limit, 50_000_000);
    assert!(budget.estimable);
}

#[test]
fn estimate_ok_empty_budget() {
    let budget = Budget::with_defaults();
    assert!(budget.estimate_ok(100, 1_000_000));
}

#[test]
fn estimate_ok_both_dimensions_pass() {
    let budget = Budget::new(100, 1_000_000, true);
    assert!(budget.estimate_ok(50, 500_000));
}

#[test]
fn estimate_ok_wall_clock_at_limit() {
    let budget = Budget::new(100, 1_000_000, true);
    // Exactly at limit should pass
    assert!(budget.estimate_ok(100, 500_000));
}

#[test]
fn estimate_ok_wall_clock_exceeds_limit() {
    let budget = Budget::new(100, 1_000_000, true);
    // Over limit by 1 second
    assert!(!budget.estimate_ok(101, 500_000));
}

#[test]
fn estimate_ok_bytes_at_limit() {
    let budget = Budget::new(100, 1_000_000, true);
    // Exactly at limit should pass
    assert!(budget.estimate_ok(50, 1_000_000));
}

#[test]
fn estimate_ok_bytes_exceeds_limit() {
    let budget = Budget::new(100, 1_000_000, true);
    // Over limit by 1 byte
    assert!(!budget.estimate_ok(50, 1_000_001));
}

#[test]
fn estimate_ok_after_consume() {
    let mut budget = Budget::new(100, 1_000_000, true);
    // Consume 50 seconds and 500k bytes
    budget.consume(50, 500_000);
    // Check remaining capacity: 50 seconds and 500k bytes
    assert!(budget.estimate_ok(50, 500_000));
    // Over either dimension should fail
    assert!(!budget.estimate_ok(51, 500_000));
    assert!(!budget.estimate_ok(50, 500_001));
}

#[test]
fn estimate_ok_non_estimable_always_false() {
    let budget = Budget::new(1000, 10_000_000, false).non_estimable();
    // Even with huge available budget, should return false
    assert!(!budget.estimate_ok(1, 1));
    assert!(!budget.estimate_ok(100, 1_000_000));
}

#[test]
fn consume_accumulates() {
    let mut budget = Budget::new(100, 1_000_000, true);
    budget.consume(10, 100_000);
    assert_eq!(budget.wall_clock_consumed, 10);
    assert_eq!(budget.bytes_consumed, 100_000);

    budget.consume(20, 200_000);
    assert_eq!(budget.wall_clock_consumed, 30);
    assert_eq!(budget.bytes_consumed, 300_000);
}

#[test]
fn consume_does_not_clamp_to_limit() {
    let mut budget = Budget::new(100, 1_000_000, true);
    // Consume more than the limit
    budget.consume(150, 2_000_000);
    assert_eq!(budget.wall_clock_consumed, 150);
    assert_eq!(budget.bytes_consumed, 2_000_000);
}

#[test]
fn exceeded_false_when_within_limits() {
    let mut budget = Budget::new(100, 1_000_000, true);
    budget.consume(50, 500_000);
    assert!(!budget.exceeded());
}

#[test]
fn exceeded_false_at_exact_limits() {
    let mut budget = Budget::new(100, 1_000_000, true);
    budget.consume(100, 1_000_000);
    assert!(!budget.exceeded());
}

#[test]
fn exceeded_true_wall_clock_by_1_second() {
    let mut budget = Budget::new(100, 1_000_000, true);
    budget.consume(101, 500_000);
    assert!(budget.exceeded());
}

#[test]
fn exceeded_true_bytes_by_1_byte() {
    let mut budget = Budget::new(100, 1_000_000, true);
    budget.consume(50, 1_000_001);
    assert!(budget.exceeded());
}

#[test]
fn exceeded_true_both_dimensions() {
    let mut budget = Budget::new(100, 1_000_000, true);
    budget.consume(150, 2_000_000);
    assert!(budget.exceeded());
}

#[test]
fn wall_clock_override() {
    let budget = Budget::with_defaults().with_wall_clock(60);
    assert_eq!(budget.wall_clock_limit, 60);
    // bytes should still be default
    assert_eq!(budget.bytes_limit, DEFAULT_BYTES_BUDGET);
}

#[test]
fn bytes_override() {
    let budget = Budget::with_defaults().with_bytes(50_000_000);
    assert_eq!(budget.bytes_limit, 50_000_000);
    // wall_clock should still be default
    assert_eq!(budget.wall_clock_limit, DEFAULT_WALL_CLOCK_SECONDS);
}

#[test]
fn both_overrides() {
    let budget = Budget::with_defaults()
        .with_wall_clock(120)
        .with_bytes(200_000_000);
    assert_eq!(budget.wall_clock_limit, 120);
    assert_eq!(budget.bytes_limit, 200_000_000);
}

#[test]
fn non_estimable_flag() {
    let budget = Budget::with_defaults().non_estimable();
    assert!(!budget.estimable);
}

#[test]
fn precedence_chain_simulation() {
    // Built-in default
    let default_budget = Budget::with_defaults();
    assert_eq!(default_budget.wall_clock_limit, DEFAULT_WALL_CLOCK_SECONDS);

    // Project config override
    let with_config = Budget::with_defaults().with_wall_clock(180);
    assert_eq!(with_config.wall_clock_limit, 180);

    // CLI flag override (simulated by using custom new())
    let with_cli = Budget::new(90, DEFAULT_BYTES_BUDGET, true);
    assert_eq!(with_cli.wall_clock_limit, 90);

    // CLI should win over config
    assert!(with_cli.wall_clock_limit < with_config.wall_clock_limit);
}

#[test]
fn independent_dimensions() {
    let mut budget = Budget::new(100, 1_000_000, true);
    // Exhaust only the wall-clock dimension
    budget.consume(150, 100_000);
    assert!(budget.exceeded()); // exceeded wall-clock

    // bytes should still be well within limit
    let bytes_estimate = budget.estimate_ok(0, 500_000);
    assert!(!bytes_estimate); // estimate_ok fails because wall-clock already exceeded
}

#[test]
fn bytes_exceed_independently() {
    let mut budget = Budget::new(100, 1_000_000, true);
    // Exhaust only the bytes dimension
    budget.consume(10, 1_500_000);
    assert!(budget.exceeded()); // exceeded bytes

    // wall-clock should still be well within limit
    let wall_estimate = budget.estimate_ok(50, 0);
    assert!(!wall_estimate); // estimate_ok fails because bytes already exceeded
}

#[test]
fn clone_independence() {
    let mut budget1 = Budget::with_defaults();
    budget1.consume(50, 500_000);

    let mut budget2 = budget1.clone();
    budget2.consume(30, 300_000);

    // budget1 should not be affected by budget2's consume
    assert_eq!(budget1.wall_clock_consumed, 50);
    assert_eq!(budget1.bytes_consumed, 500_000);

    assert_eq!(budget2.wall_clock_consumed, 80);
    assert_eq!(budget2.bytes_consumed, 800_000);
}
