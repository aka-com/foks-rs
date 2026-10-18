use super::*;
use crate::{
    authority::{Activation, Identity},
    Limits,
};
fn entropy() -> Zeroizing<[u8; 48]> {
    let mut bytes = Zeroizing::new([0x11; 48]);
    bytes[32..].fill(0x22);
    bytes
}
fn authority() -> ReadTokenAuthority {
    ReadTokenAuthority::from_host_entropy(entropy()).unwrap()
}
fn binding() -> ListBinding {
    ListBinding::new(
        Fence {
            identity: Identity {
                installation: [1; 16],
                tenant: [2; 16],
                host: vec![3; 33],
                cell: [4; 32],
            },
            activation: Activation {
                version: Number(1),
                schema: Number(58),
                schema_hash: [5; 32],
                kernel_hash: [6; 32],
                catalog_hash: [7; 32],
                key_generation: Number(8),
                key_digest: [9; 32],
                storage_namespace: [10; 16],
                storage_placement: [11; 32],
                limits: Limits {
                    blob_bytes: 16777216,
                    dirent_bytes: 4096,
                    namespace_bytes: 9007199254740993,
                    namespace_objects: 10000,
                    directories: 1000,
                    dirents: 9000,
                },
            },
            owner: [12; 16],
            epoch: Number(2),
        },
        Number(3),
        ReadDependency::new([13; 32], 9007199254740993).unwrap(),
        PeerCertificate::from_frontend_parts(vec![48, 1, 2], vec![7], [8; 32], 20000000).unwrap(),
        [1; 33],
        [9; 16],
        InitialCursor::Time {
            value: Number(9007199254740993),
        },
        4097,
        true,
        Number(15000),
        [10; 32],
    )
    .unwrap()
}
fn key() -> RoleAndGeneration {
    RoleAndGeneration {
        role: Role::member(-7),
        generation: 9007199254740993,
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn token(authority: &ReadTokenAuthority, binding: &ListBinding) -> Token {
    authority
        .issue(binding, key(), [0x33; 16], Number(1))
        .unwrap()
}
fn remac(authority: &ReadTokenAuthority, bytes: &mut [u8; TOKEN_BYTES]) {
    let mut mac = HmacSha256::new_from_slice(authority.key.as_ref()).unwrap();
    mac.update(MAC_DOMAIN);
    mac.update(&bytes[..PAYLOAD_BYTES]);
    bytes[PAYLOAD_BYTES..].copy_from_slice(&mac.finalize().into_bytes());
}
#[test]
fn golden_canonical_binding_and_fixed_token_match_independent_fixture() {
    let b = binding();
    let a = authority();
    let t = token(&a, &b);
    assert_eq!(
        b.canonical_bytes().unwrap(),
        include_bytes!("binding.json").strip_suffix(b"\n").unwrap()
    );
    assert_eq!(
        hex(&b.digest().unwrap()),
        include_str!("binding.sha256").trim()
    );
    assert_eq!(hex(t.as_bytes()), include_str!("token.hex").trim());
    assert_eq!(t.as_bytes().len(), 180);
    let verified = a.verify(t.as_bytes(), &b, Number(14999)).unwrap();
    assert_eq!(verified.key(), key());
    assert_eq!(verified.remaining(), 4096);
    assert_eq!(verified.ordinal(), 0);
}
#[test]
fn every_token_byte_and_truncation_is_authenticated() {
    let b = binding();
    let a = authority();
    let t = token(&a, &b);
    for end in 0..TOKEN_BYTES {
        assert!(a.verify(&t.as_bytes()[..end], &b, Number(1)).is_err());
    }
    let mut extra = t.as_bytes().to_vec();
    extra.push(0);
    assert!(a.verify(&extra, &b, Number(1)).is_err());
    for index in 0..TOKEN_BYTES {
        let mut bad = *t.as_bytes();
        bad[index] ^= 1;
        assert!(a.verify(&bad, &b, Number(1)).is_err(), "byte{index}");
    }
    for offset in [4, 5, 6, 7, 72, 92, 94, 96, 98, 99, 100, 132] {
        let mut bad = *t.as_bytes();
        bad[offset] = 255;
        remac(&a, &mut bad);
        assert!(
            a.verify(&bad, &b, Number(1)).is_err(),
            "authenticated invalid field{offset}"
        );
    }
    let mut invalid_visibility = *t.as_bytes();
    invalid_visibility[80..84].copy_from_slice(&32768_i32.to_le_bytes());
    remac(&a, &mut invalid_visibility);
    assert!(a.verify(&invalid_visibility, &b, Number(1)).is_err());
}
#[test]
fn full_binding_identity_context_arguments_and_clock_are_pinned() {
    let a = authority();
    let b = binding();
    let t = token(&a, &b);
    type Change = Box<dyn Fn(&mut ListBinding)>;
    let changes: Vec<Change> = vec![
        Box::new(|b| b.fence.identity.installation[0] ^= 1),
        Box::new(|b| b.fence.identity.tenant[0] ^= 1),
        Box::new(|b| b.fence.identity.host[0] ^= 1),
        Box::new(|b| b.fence.identity.cell[0] ^= 1),
        Box::new(|b| b.fence.owner[0] ^= 1),
        Box::new(|b| b.fence.epoch.0 += 1),
        Box::new(|b| b.revision.0 += 1),
        Box::new(|b| b.dependency.incarnation[0] ^= 1),
        Box::new(|b| b.dependency.revision.0 += 1),
        Box::new(|b| b.fence.activation.version.0 += 1),
        Box::new(|b| b.fence.activation.schema.0 += 1),
        Box::new(|b| b.fence.activation.schema_hash[0] ^= 1),
        Box::new(|b| b.fence.activation.kernel_hash[0] ^= 1),
        Box::new(|b| b.fence.activation.catalog_hash[0] ^= 1),
        Box::new(|b| b.fence.activation.key_generation.0 += 1),
        Box::new(|b| b.fence.activation.key_digest[0] ^= 1),
        Box::new(|b| b.fence.activation.storage_namespace[0] ^= 1),
        Box::new(|b| b.fence.activation.storage_placement[0] ^= 1),
        Box::new(|b| b.fence.activation.limits.blob_bytes += 1),
        Box::new(|b| b.fence.activation.limits.dirent_bytes += 1),
        Box::new(|b| b.fence.activation.limits.namespace_bytes += 1),
        Box::new(|b| b.fence.activation.limits.namespace_objects += 1),
        Box::new(|b| b.fence.activation.limits.directories += 1),
        Box::new(|b| b.fence.activation.limits.dirents += 1),
        Box::new(|b| {
            b.peer =
                PeerCertificate::from_frontend_parts(vec![48, 1, 3], vec![7], [8; 32], 20000000)
                    .unwrap()
        }),
        Box::new(|b| {
            b.peer =
                PeerCertificate::from_frontend_parts(vec![48, 1, 2], vec![9], [8; 32], 20000000)
                    .unwrap()
        }),
        Box::new(|b| {
            b.peer =
                PeerCertificate::from_frontend_parts(vec![48, 1, 2], vec![7], [9; 32], 20000000)
                    .unwrap()
        }),
        Box::new(|b| {
            b.peer =
                PeerCertificate::from_frontend_parts(vec![48, 1, 2], vec![7], [8; 32], 20000001)
                    .unwrap()
        }),
        Box::new(|b| b.namespace[0] ^= 1),
        Box::new(|b| b.parent[0] ^= 1),
        Box::new(|b| b.cursor = InitialCursor::None {}),
        Box::new(|b| b.count.0 -= 1),
        Box::new(|b| b.load_small = false),
        Box::new(|b| b.deadline.0 += 1),
        Box::new(|b| b.clock_domain[0] ^= 1),
        Box::new(|b| b.route = "different_route"),
    ];
    for change in changes {
        let mut altered = b.clone();
        change(&mut altered);
        assert!(a.verify(t.as_bytes(), &altered, Number(1)).is_err());
    }
    assert!(matches!(
        a.verify(t.as_bytes(), &b, Number(15000)),
        Err(Error::Expired)
    ));
}
#[test]
fn stateless_retries_ties_completion_and_expected_frontend_position() {
    let a = authority();
    let mut b = binding();
    b.count = Number(16);
    let original = token(&a, &b);
    let initial = a.verify(original.as_bytes(), &b, Number(1)).unwrap();
    let last = Position::Time(9007199254740993, [1; 16]);
    let first = a.advance(&initial, &b, 15, 15, last, Number(2)).unwrap();
    assert_eq!(
        first.as_bytes(),
        a.advance(&initial, &b, 15, 15, last, Number(2))
            .unwrap()
            .as_bytes()
    );
    let verified = a.verify(first.as_bytes(), &b, Number(2)).unwrap();
    assert_eq!(verified.returned(), 15);
    assert_eq!(verified.remaining(), 1);
    assert!(!verified.complete());
    verified.check_expected([0x33; 16], 1).unwrap();
    assert!(verified.check_expected([0x33; 16], 0).is_err());
    assert!(verified.check_expected([0x44; 16], 1).is_err());
    assert!(a.advance(&verified, &b, 1, 1, last, Number(3)).is_err());
    let last = Position::Time(9007199254740993, [2; 16]);
    let full = a.advance(&verified, &b, 1, 1, last, Number(3)).unwrap();
    let full = a.verify(full.as_bytes(), &b, Number(3)).unwrap();
    assert!(full.complete());
    assert!(!full.final_page());
    assert_eq!(full.returned(), 16);
    assert!(a.advance(&full, &b, 0, 1, last, Number(4)).is_err());
    let short = a
        .advance(
            &initial,
            &b,
            1,
            15,
            Position::Time(9007199254740993, [1; 16]),
            Number(3),
        )
        .unwrap();
    let short = a.verify(short.as_bytes(), &b, Number(3)).unwrap();
    assert!(short.complete() && short.final_page());
    let empty = a
        .advance(&initial, &b, 0, 15, Position::None, Number(3))
        .unwrap();
    let empty = a.verify(empty.as_bytes(), &b, Number(3)).unwrap();
    assert!(empty.complete() && empty.final_page());
    assert_eq!(empty.returned(), 0);
}
#[test]
fn key_loss_rotation_and_different_authority_cannot_resign_previously_verified_state() {
    let mut a = authority();
    let b = binding();
    let t = token(&a, &b);
    let verified = a.verify(t.as_bytes(), &b, Number(1)).unwrap();
    let mut changed = entropy();
    changed[0] ^= 1;
    let another = ReadTokenAuthority::from_host_entropy(changed).unwrap();
    assert!(another.verify(t.as_bytes(), &b, Number(1)).is_err());
    assert!(another
        .advance(
            &verified,
            &b,
            1,
            15,
            Position::Time(9007199254740993, [1; 16]),
            Number(2)
        )
        .is_err());
    a.invalidate();
    assert_eq!(&*a.key, &[0; 32]);
    assert_eq!(a.nonce, [0; 16]);
    assert!(matches!(
        a.verify(t.as_bytes(), &b, Number(1)),
        Err(Error::BundleLost)
    ));
    assert!(a.issue(&b, key(), [0x33; 16], Number(1)).is_err());
    assert!(ReadTokenAuthority::from_host_entropy(Zeroizing::new([0; 48])).is_err());
    let mut zero_nonce = entropy();
    zero_nonce[32..].fill(0);
    assert!(ReadTokenAuthority::from_host_entropy(zero_nonce).is_err());
}

#[test]
fn dependency_coordinate_is_mandatory_bounded_and_independent_of_intent_revision() {
    assert!(ReadDependency::new([0; 32], 1).is_err());
    assert!(ReadDependency::new([1; 32], 0).is_err());
    assert!(ReadDependency::new([1; 32], i64::MAX as u64 + 1).is_err());
    assert_eq!(
        ReadDependency::new([1; 32], i64::MAX as u64)
            .unwrap()
            .revision(),
        i64::MAX as u64
    );
    let a = authority();
    let b = binding();
    let t = token(&a, &b);
    let mut changed = b.clone();
    changed.dependency = ReadDependency::new([14; 32], b.dependency.revision()).unwrap();
    assert!(a.verify(t.as_bytes(), &changed, Number(1)).is_err());
    changed = b.clone();
    changed.dependency =
        ReadDependency::new(*b.dependency.incarnation(), b.dependency.revision() + 1).unwrap();
    assert!(a.verify(t.as_bytes(), &changed, Number(1)).is_err());
    assert_eq!(b.revision, changed.revision);
}
