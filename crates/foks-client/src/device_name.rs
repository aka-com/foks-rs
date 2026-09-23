/// User-facing rules for names that remain invalid after presentation cleanup.
pub const DEVICE_NAME_RULES: &str = "Device names must normalize to 2 to 200 letters, numbers, spaces, or . _ + ' -, and start with a letter or number. Use single punctuation marks; words cannot end in . _ or ', and + or - cannot stand alone. Remove unsupported symbols.";

/// Cleans presentation characters before protocol validation. This does not
/// make arbitrary names valid, transliterate letters, or truncate input. Never
/// apply this to signed names being verified.
pub fn fix_device_name(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| match c {
            '‘' | '’' | '‚' | '‛' => '\'',
            '‐' | '‑' | '–' | '—' | '−' => '-',
            'Ａ'..='Ｚ' | 'ａ'..='ｚ' | '０'..='９' | '．' | '＿' | '＋' | '＇' | '－' => {
                char::from_u32(c as u32 - 0xfee0).expect("fullwidth ASCII")
            }
            other => other,
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Prepares user input with a common display-byte bound and protocol checks.
/// Use only for new names, never for verifying existing signed labels.
pub fn prepare_device_name(name: &str) -> crate::Result<String> {
    if name.len() > foks_proto::MAXIMUM_DEVICE_NAME_BYTES {
        return Err(crate::Error::AccountRequest(
            "device name input exceeds 4096 bytes",
        ));
    }
    let fixed = fix_device_name(name);
    foks_verify::normalize_device_name(fixed.as_bytes()).ok_or(crate::Error::AccountRequest(
        "device name is not valid after normalization",
    ))?;
    Ok(fixed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_name_display_bytes_and_normalized_length_have_distinct_bounds() {
        assert_eq!(
            prepare_device_name(&"é".repeat(200)).unwrap(),
            "é".repeat(200)
        );
        assert!(prepare_device_name(&"é".repeat(201)).is_err());
        let excessive = format!("ab{}", "\u{301}".repeat(2048));
        assert!(prepare_device_name(&excessive).is_err());
    }

    #[test]
    fn device_name_cleanup_preserves_meaning_and_validates_after_substitution() {
        for (input, expected) in [
            ("Daniel’s MacBook Pro", "Daniel's MacBook Pro"),
            ("a‘b’c‚d‛e", "a'b'c'd'e"),
            ("a‐b‑c–d—e−f", "a-b-c-d-e-f"),
            (" Ｍａｃ　１２．３＋ ", "Mac 12.3+"),
            ("  René\u{a0}\t\n Mac  ", "René Mac"),
            ("Élodie's Mac", "Élodie's Mac"),
        ] {
            let fixed = fix_device_name(input);
            assert_eq!(fixed, expected);
            assert_eq!(fix_device_name(&fixed), fixed);
            assert!(foks_verify::normalize_device_name(fixed.as_bytes()).is_some());
        }
        for input in [
            "Mac 😀",
            "Mac\u{200b}",
            "Mac\0",
            "“Mac”",
            "这是书",
            "a__b",
            "Mac — Pro",
            "Mac.",
            "a",
        ] {
            assert!(
                foks_verify::normalize_device_name(fix_device_name(input).as_bytes()).is_none(),
                "{input}"
            );
        }
        assert!(
            foks_verify::normalize_device_name(fix_device_name(&"Ａ".repeat(200)).as_bytes())
                .is_some()
        );
        assert!(
            foks_verify::normalize_device_name(fix_device_name(&"Ａ".repeat(201)).as_bytes())
                .is_none()
        );
    }
}
