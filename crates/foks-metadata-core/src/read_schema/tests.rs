use super::*;

fn profile(id: QualificationProfileId) -> ReadSchemaProfile {
    ReadSchemaProfile {
        id,
        hashes: std::array::from_fn(|i| [i as u8 + 1; 32]),
    }
}

#[test]
fn exact_private_bytes_are_fixed_and_ordered() {
    let value = profile(QualificationProfileId::NativeSchema59);
    // Independent fixed byte oracle for all fixed-width fields. Hash bytes are
    // distinct per component so swapping any of the seven fields is visible.
    let prefix: [u8; 44] = [
        0x46, 0x4b, 0x53, 0x50, 1, 0, 0, 0, 1, 0, 0, 0x80, 59, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1,
        0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0,
    ];
    let suffix: [u8; 28] = [
        0, 4, 0, 0, 0, 0, 0x20, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0x10, 0, 0x40, 0, 0, 0, 0, 4, 0, 0,
    ];
    let actual = value.encoded();
    assert_eq!(actual.len(), 296);
    assert_eq!(&actual[..44], &prefix);
    for (index, hash) in actual[44..268].chunks_exact(32).enumerate() {
        assert_eq!(hash, [index as u8 + 1; 32]);
    }
    assert_eq!(&actual[268..], &suffix);
    assert_eq!(value.id(), QualificationProfileId::NativeSchema59);
    // Independent parent-review Python struct.pack + hashlib calculation over
    // the specified296-byte fixture and a single-NUL domain (2026-10-16).
    assert_eq!(
        value.composite_hash(),
        [
            0x34, 0x05, 0x26, 0xd3, 0x83, 0x08, 0xea, 0xeb, 0xc4, 0xbb, 0x02, 0x6b, 0xf3, 0x5d,
            0xfd, 0x5a, 0x5a, 0xc6, 0xef, 0x89, 0xbe, 0xaa, 0xf5, 0x17, 0x53, 0x90, 0x2f, 0x4c,
            0xce, 0xb8, 0xd4, 0x30
        ]
    );
}

#[test]
fn qualification_ids_and_every_contract_component_bind_the_digest() {
    let native = profile(QualificationProfileId::NativeSchema59);
    let celld = profile(QualificationProfileId::Celld062Schema59);
    assert_eq!(native.id().encoded(), 0x8000_0001);
    assert_eq!(celld.id().encoded(), 0x8000_0002);
    assert_ne!(native.encoded(), celld.encoded());
    assert_ne!(native.composite_hash(), celld.composite_hash());
    for component in 0..7 {
        let mut changed = profile(QualificationProfileId::NativeSchema59);
        changed.hashes[component][31] ^= 0x80;
        assert_ne!(native.composite_hash(), changed.composite_hash());
    }
    let mut wrong_domain = Sha256::new();
    wrong_domain.update(b"foks-read-dependency-schema-v1\0");
    wrong_domain.update(native.encoded());
    let wrong: [u8; 32] = wrong_domain.finalize().into();
    assert_ne!(native.composite_hash(), wrong);
    // The separator is a single NUL, not the textual escape sequence.
    let mut textual_escape = Sha256::new();
    textual_escape.update(b"foks-hosted-read-schema-profile-v1\\0");
    textual_escape.update(native.encoded());
    let wrong: [u8; 32] = textual_escape.finalize().into();
    assert_ne!(native.composite_hash(), wrong);
}

#[cfg(feature = "native")]
mod generator;
