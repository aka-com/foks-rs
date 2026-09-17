use crate::commands::validation::{
    bounded_local_name, main_window_allowed, positive_recovery_serial_with,
};

#[test]
fn value_commands_only_accept_the_declared_main_window_label() {
    assert!(main_window_allowed("main"));
    assert!(!main_window_allowed("settings"));
    assert!(!main_window_allowed(""));
}

#[test]
fn recovery_serial_is_positive_and_not_renderer_controlled() {
    let mut fills = vec![[0u8; 8], 7u64.to_le_bytes()].into_iter();
    let serial = positive_recovery_serial_with(|bytes| {
        bytes.copy_from_slice(&fills.next().unwrap());
        Ok(())
    })
    .unwrap();
    assert_eq!(serial, 7);

    let error = positive_recovery_serial_with(|bytes| {
        bytes.fill(0);
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.code, "randomness-unavailable");
}

#[test]
fn phase_five_local_names_match_the_client_boundary() {
    let maximum = "x".repeat(64);
    for valid in ["a", "work_profile-2", maximum.as_str()] {
        assert_eq!(bounded_local_name(valid, "bad").unwrap(), valid);
    }
    let too_long = "x".repeat(65);
    for invalid in ["", "has space", "has/slash", "dot.name", too_long.as_str()] {
        assert_eq!(
            bounded_local_name(invalid, "bad").unwrap_err().code,
            "invalid-request"
        );
    }
}
