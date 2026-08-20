use super::*;

#[test]
fn evenly_divisible_budget_yields_exact_step_count() {
    assert_eq!(steps_from_wall_clock_budget(300, 30), 10);
}

#[test]
fn budget_floors_down_to_whole_steps() {
    assert_eq!(steps_from_wall_clock_budget(45, 30), 1);
    assert_eq!(steps_from_wall_clock_budget(59, 30), 1);
}

#[test]
fn budget_smaller_than_one_step_yields_zero() {
    assert_eq!(steps_from_wall_clock_budget(29, 30), 0);
    assert_eq!(steps_from_wall_clock_budget(0, 30), 0);
}

#[test]
fn zero_seconds_per_step_never_divides_by_zero() {
    assert_eq!(steps_from_wall_clock_budget(300, 0), 0);
}

#[test]
fn default_seconds_per_step_is_thirty() {
    assert_eq!(DEFAULT_SECONDS_PER_STEP, 30);
}
