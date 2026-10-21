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

/// The status for a team view bearer token that did not resolve to current
/// authority, given whether this host still holds a row for it. Matching
/// go-foks, a token with no row (never issued, evicted or reclaimed after
/// expiry) is `TEAM_VO_BEARER_TOKEN_NOT_FOUND_ERROR`, and a recorded token that
/// expired or whose membership was superseded is
/// `TEAM_BEARER_TOKEN_STALE_ERROR`. A client cures either by activating a new
/// view; v0.1.9 Go clients recognise only the stale status, so it stays the
/// answer wherever a row exists.
pub(crate) fn view_token_refusal(recorded: foks_server_db::Result<bool>) -> foks_rpc::RpcStatus {
    match recorded {
        Ok(false) => foks_rpc::RpcStatus::TeamVOBearerTokenNotFound,
        Ok(true) => foks_rpc::RpcStatus::TeamBearerTokenStale(
            "team-view bearer token is not active".to_owned(),
        ),
        Err(_) => foks_rpc::RpcStatus::TransactionRetry,
    }
}

pub(crate) fn challenge_hash(exact: &[u8]) -> [u8; 32] {
    const TYPE_ID: u64 = 0x6d10_7e4b_464f_4b53;
    foks_crypto::prefixed_hash(TYPE_ID, exact)
}

pub(crate) fn admin_token_hash(token: &[u8; 16]) -> [u8; 32] {
    const TYPE_ID: u64 = 0x6d10_7e4f_464f_4b53;
    foks_crypto::prefixed_hash(TYPE_ID, token)
}
