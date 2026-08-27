use foks_proto::Role;

pub(crate) fn role_parts(role: Role) -> (u64, i64) {
    (
        role.protocol_value(),
        role.visibility().map(i64::from).unwrap_or_default(),
    )
}

pub(crate) fn stored_role(kind: u64, visibility: i64) -> Option<Role> {
    match kind {
        1 => i16::try_from(visibility).ok().map(Role::member),
        2 if visibility == 0 => Some(Role::ADMIN),
        3 if visibility == 0 => Some(Role::OWNER),
        _ => None,
    }
}

pub(crate) fn token_hash(token: &[u8; 16]) -> [u8; 32] {
    const TYPE_ID: u64 = 0x6d10_7e4a_464f_4b53;
    foks_crypto::prefixed_hash(TYPE_ID, token)
}

pub(crate) fn challenge_hash(exact: &[u8]) -> [u8; 32] {
    const TYPE_ID: u64 = 0x6d10_7e4b_464f_4b53;
    foks_crypto::prefixed_hash(TYPE_ID, exact)
}

pub(crate) fn admin_token_hash(token: &[u8; 16]) -> [u8; 32] {
    const TYPE_ID: u64 = 0x6d10_7e4f_464f_4b53;
    foks_crypto::prefixed_hash(TYPE_ID, token)
}
