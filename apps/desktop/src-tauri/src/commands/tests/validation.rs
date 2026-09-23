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

#[test]
fn device_names_are_cleaned_before_desktop_validation() {
    use crate::commands::validation::valid_device_name;
    for (input, expected) in [
        ("Daniel’s MacBook Pro", "Daniel's MacBook Pro"),
        ("  René\u{a0}\tMac  ", "René Mac"),
        ("Mac–Pro", "Mac-Pro"),
    ] {
        assert_eq!(valid_device_name(input).unwrap(), expected);
    }
    assert_eq!(
        valid_device_name(&"Ａ".repeat(200)).unwrap(),
        "A".repeat(200)
    );
    for input in ["a__b", "Mac — Pro", "Mac.", "Mac 😀", "“Mac”", "a", ""] {
        assert!(valid_device_name(input).is_err(), "{input}");
    }
    assert_eq!(
        valid_device_name(&"é".repeat(200)).unwrap(),
        "é".repeat(200)
    );
    assert!(valid_device_name(&"é".repeat(201)).is_err());
    assert!(valid_device_name(&format!("ab{}", "\u{301}".repeat(2048))).is_err());
    assert!(valid_device_name(&"A".repeat(201)).is_err());
    for input in ["Mac\u{200b}", "Mac\0", "Mac\u{202e}"] {
        assert!(valid_device_name(input)
            .unwrap_err()
            .message
            .contains("invisible"));
    }
}

#[test]
fn recovery_phrase_accepts_wrapped_tokens_without_changing_the_secret() {
    use crate::commands::validation::{pairing_phrase, recovery_phrase};
    let phrase = foks_crypto::BackupKey::from_seed([0; 26])
        .unwrap()
        .phrase()
        .expose_joined();
    let wrapped = phrase.replace(' ', "\r\n");
    assert!(recovery_phrase(wrapped).is_ok());
    assert!(recovery_phrase(format!("{}\0", phrase.as_str())).is_err());
    assert!(recovery_phrase("x".repeat(4097)).is_err());
    let phrase = foks_crypto::KexSecret::generate()
        .unwrap()
        .phrase()
        .expose_joined();
    assert!(pairing_phrase(phrase.replace(' ', "\n")).is_ok());
}
