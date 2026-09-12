//! Exact headerless team invitation calls; no request-status extension.
use crate::TEAM_GUEST_PROTOCOL_ID;
use crate::{
    encode_call, Result, TEAM_ADMIN_PROTOCOL_ID, TEAM_MEMBER_PROTOCOL_ID, USER_PROTOCOL_ID,
};
use foks_proto::{
    InboxPagination, LocalViewPermissionPayload, PostGenericLinkArgument, RemoteJoinRequest, Role,
    Signature, TeamCertificate, TeamInvite, TeamRsvp,
};
use foks_snowpack::{decode, encode, Value};
fn call(protocol: u64, method: u64, fields: Vec<Value>) -> Result<Vec<u8>> {
    encode_call(protocol, method, &encode(&Value::Array(fields))?, 0)
}
fn token(t: &[u8; 16]) -> Value {
    Value::Binary(t.to_vec())
}
pub fn encode_lookup_team_certificate_request(invite: &TeamInvite) -> Result<Vec<u8>> {
    call(TEAM_GUEST_PROTOCOL_ID, 0, vec![decode(&invite.encoded()?)?])
}
pub fn encode_put_team_certificate_request(
    tok: &[u8; 16],
    cert: &TeamCertificate,
) -> Result<Vec<u8>> {
    call(
        TEAM_ADMIN_PROTOCOL_ID,
        6,
        vec![token(tok), decode(&cert.encoded()?)?],
    )
}
pub fn encode_current_team_certificates_request(tok: &[u8; 16]) -> Result<Vec<u8>> {
    call(TEAM_ADMIN_PROTOCOL_ID, 7, vec![token(tok)])
}
pub fn encode_accept_invite_local_request(
    invite: &TeamInvite,
    source_role: Role,
    tok: Option<&[u8; 16]>,
    link: Option<&PostGenericLinkArgument>,
) -> Result<Vec<u8>> {
    call(
        TEAM_MEMBER_PROTOCOL_ID,
        0,
        vec![
            decode(&invite.encoded()?)?,
            source_role.to_value(),
            tok.map_or(Value::Null, token),
            match link {
                Some(l) => decode(&l.encoded()?)?,
                None => Value::Null,
            },
        ],
    )
}
pub fn encode_accept_invite_remote_request(
    invite: &TeamInvite,
    request: &RemoteJoinRequest,
) -> Result<Vec<u8>> {
    call(
        TEAM_GUEST_PROTOCOL_ID,
        1,
        vec![decode(&invite.encoded()?)?, decode(&request.encoded()?)?],
    )
}
pub fn encode_load_team_inbox_request(
    tok: &[u8; 16],
    pagination: Option<&InboxPagination>,
) -> Result<Vec<u8>> {
    call(
        TEAM_ADMIN_PROTOCOL_ID,
        12,
        vec![
            token(tok),
            match pagination {
                Some(p) => decode(&p.encoded()?)?,
                None => Value::Null,
            },
        ],
    )
}
pub fn encode_load_remote_join_request(tok: &[u8; 16], receipt: &TeamRsvp) -> Result<Vec<u8>> {
    if !receipt.is_remote() {
        return Err(foks_proto::Error::IntegerRange("remote receipt required").into());
    }
    call(
        TEAM_ADMIN_PROTOCOL_ID,
        8,
        vec![token(tok), decode(&receipt.encoded()?)?],
    )
}
pub fn encode_reject_join_request(tok: &[u8; 16], receipt: &TeamRsvp) -> Result<Vec<u8>> {
    call(
        TEAM_ADMIN_PROTOCOL_ID,
        13,
        vec![token(tok), decode(&receipt.encoded()?)?],
    )
}
pub fn encode_grant_local_user_view_request(
    payload: &LocalViewPermissionPayload,
) -> Result<Vec<u8>> {
    call(USER_PROTOCOL_ID, 10, vec![decode(&payload.encoded()?)?])
}
pub fn encode_grant_local_team_view_request(
    payload: &LocalViewPermissionPayload,
    signature: &Signature,
    generation: u64,
    role: Role,
) -> Result<Vec<u8>> {
    call(
        TEAM_MEMBER_PROTOCOL_ID,
        2,
        vec![
            decode(&payload.encoded()?)?,
            Value::Array(vec![
                signature.to_value(),
                Value::Unsigned(generation),
                role.to_value(),
            ]),
        ],
    )
}
pub fn encode_post_team_removal_request(
    tok: &[u8; 16],
    removal: &foks_proto::TeamRemovalAndCommitment,
) -> Result<Vec<u8>> {
    call(
        TEAM_ADMIN_PROTOCOL_ID,
        11,
        vec![token(tok), decode(&removal.encoded()?)?],
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_invitation_frames_keep_bare_team_and_wrapped_user_arguments() {
        let read = |n: &str| {
            std::fs::read(format!(
                "../foks-snowpack/tests/fixtures/foks-v0.1.9/invitations/{n}"
            ))
            .unwrap()
        };
        let invite = TeamInvite::decode(&read("initial.invite")).unwrap();
        let cert = TeamCertificate::decode(&read("initial.cert")).unwrap();
        let grant = LocalViewPermissionPayload::decode(&read("local-grant.payload")).unwrap();
        let mut tok = [0; 16];
        tok[0] = 91;
        assert_eq!(
            encode_lookup_team_certificate_request(&invite).unwrap(),
            read("lookup.rpc")
        );
        assert_eq!(
            encode_put_team_certificate_request(&tok, &cert).unwrap(),
            read("put.rpc")
        );
        assert_eq!(
            encode_accept_invite_local_request(&invite, Role::OWNER, None, None).unwrap(),
            read("accept-local.rpc")
        );
        assert_eq!(
            encode_grant_local_user_view_request(&grant).unwrap(),
            read("grant-user.rpc")
        );
    }
}
