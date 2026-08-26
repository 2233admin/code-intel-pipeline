use super::*;

#[test]
fn staged_output_path_differs_even_when_the_clock_repeats() {
    // #352 regression: the previous temp-file name was pid + a raw clock
    // read, nothing else. `clock` is stubbed to return the exact same
    // reading twice in a row -- the condition #352 already confirmed
    // causes silent data corruption in `audit_report::TempReport` -- so
    // this collision case is exercised deterministically instead of
    // waiting for a real one. `create_new(true)` at the call site turns a
    // collision into a loud `AlreadyExists` error rather than silent
    // overwrite, but a spurious error under legitimate concurrent use is
    // still a real bug worth fixing.
    let fixed_clock = || Ok(0xC352_u128);
    let parent = Path::new("/tmp");
    let first = staged_output_path(parent, "artifact.json", 4242, fixed_clock).expect("allocate");
    let second = staged_output_path(parent, "artifact.json", 4242, fixed_clock).expect("allocate");
    assert_ne!(
        first, second,
        "staged output path must stay unique even when the clock reading repeats"
    );
}

#[test]
fn staged_output_path_propagates_a_clock_failure() {
    // The allocator's contract with its caller: a clock error must still
    // surface as `AdapterError::Io`, not be swallowed. Pins that widening
    // the allocator to take an injectable clock did not change this.
    let parent = Path::new("/tmp");
    let error = staged_output_path(parent, "artifact.json", 4242, || {
        Err(AdapterError::Io("clock unavailable".to_string()))
    })
    .expect_err("clock failure must propagate");
    assert!(matches!(error, AdapterError::Io(message) if message == "clock unavailable"));
}
