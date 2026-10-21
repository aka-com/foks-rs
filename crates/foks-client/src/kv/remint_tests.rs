use std::cell::RefCell;

use foks_rpc::KvAuth;

use super::rpc::KvRequest;
use super::{call_with_remint, OwnedKvAuth, TeamViewRemint};
use crate::{Error, Result};

fn status(code: u64) -> Error {
    Error::Rpc(foks_rpc::Error::RemoteStatus {
        code,
        detail: Default::default(),
    })
}

/// A server that refuses every token in `refused` with `code` and records
/// the token each request presented.
fn server<'a>(
    refused: &'a [[u8; 16]],
    code: u64,
    seen: &'a RefCell<Vec<Option<[u8; 16]>>>,
) -> impl FnMut(KvAuth<'_>, &KvRequest) -> Result<Vec<u8>> + 'a {
    move |auth, _| {
        let token = match auth {
            KvAuth::User => None,
            KvAuth::Team(token) => Some(*token),
        };
        seen.borrow_mut().push(token);
        match token {
            Some(token) if refused.contains(&token) => Err(status(code)),
            _ => Ok(vec![0xc0]),
        }
    }
}

fn remint_to(token: [u8; 16], calls: &RefCell<u32>) -> TeamViewRemint<'_> {
    Box::new(move || {
        *calls.borrow_mut() += 1;
        Ok(token)
    })
}

#[test]
fn a_refused_view_token_is_reminted_and_the_request_replayed_once() {
    for code in [
        foks_rpc::STATUS_TEAM_BEARER_TOKEN_STALE_ERROR,
        foks_rpc::STATUS_TEAM_VO_BEARER_TOKEN_NOT_FOUND_ERROR,
    ] {
        let seen = RefCell::new(Vec::new());
        let calls = RefCell::new(0);
        let mut auth = OwnedKvAuth::Team([1; 16]);
        let mut remint = Some(remint_to([2; 16], &calls));
        let result = call_with_remint(
            server(&[[1; 16]], code, &seen),
            &mut auth,
            &mut remint,
            &KvRequest::Root,
        );
        assert_eq!(result.unwrap(), [0xc0]);
        assert_eq!(*seen.borrow(), [Some([1; 16]), Some([2; 16])]);
        assert_eq!(*calls.borrow(), 1);
        assert!(remint.is_none());
        assert!(matches!(auth, OwnedKvAuth::Team(token) if token == [2; 16]));

        // A later refusal in the same session is not re-minted again.
        let seen = RefCell::new(Vec::new());
        let error = call_with_remint(
            server(&[[2; 16]], code, &seen),
            &mut auth,
            &mut remint,
            &KvRequest::Root,
        )
        .unwrap_err();
        assert!(crate::rejects_team_view_token(&error));
        assert_eq!(seen.borrow().len(), 1);
    }
}

#[test]
fn a_replacement_that_is_refused_again_is_not_replayed_a_second_time() {
    let seen = RefCell::new(Vec::new());
    let calls = RefCell::new(0);
    let mut auth = OwnedKvAuth::Team([1; 16]);
    let mut remint = Some(remint_to([2; 16], &calls));
    let error = call_with_remint(
        server(
            &[[1; 16], [2; 16]],
            foks_rpc::STATUS_TEAM_VO_BEARER_TOKEN_NOT_FOUND_ERROR,
            &seen,
        ),
        &mut auth,
        &mut remint,
        &KvRequest::Root,
    )
    .unwrap_err();
    assert!(crate::rejects_team_view_token(&error));
    assert_eq!(seen.borrow().len(), 2);
    assert_eq!(*calls.borrow(), 1);
}

#[test]
fn other_refusals_and_personal_sessions_are_not_reminted() {
    let calls = RefCell::new(0);
    for (code, mut auth) in [
        (
            foks_rpc::STATUS_PERMISSION_ERROR,
            OwnedKvAuth::Team([1; 16]),
        ),
        (foks_rpc::STATUS_EXPIRED_ERROR, OwnedKvAuth::Team([1; 16])),
        (foks_rpc::STATUS_KV_PERM_ERROR, OwnedKvAuth::Team([1; 16])),
    ] {
        let seen = RefCell::new(Vec::new());
        let mut remint = Some(remint_to([2; 16], &calls));
        let result = call_with_remint(
            server(&[[1; 16]], code, &seen),
            &mut auth,
            &mut remint,
            &KvRequest::Root,
        );
        assert!(matches!(
            result,
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus { code: c, .. })) if c == code
        ));
        assert_eq!(seen.borrow().len(), 1);
        assert!(remint.is_some());
    }
    // A personal session presents no view token, so even a team-token status
    // (which a personal request cannot legitimately receive) is returned.
    let mut auth = OwnedKvAuth::User;
    let mut remint = Some(remint_to([2; 16], &calls));
    let user_server = |_: KvAuth<'_>, _: &KvRequest| -> Result<Vec<u8>> {
        Err(status(
            foks_rpc::STATUS_TEAM_VO_BEARER_TOKEN_NOT_FOUND_ERROR,
        ))
    };
    assert!(call_with_remint(user_server, &mut auth, &mut remint, &KvRequest::Root).is_err());
    assert_eq!(*calls.borrow(), 0);
}

#[test]
fn a_failed_remint_reports_the_original_refusal() {
    let seen = RefCell::new(Vec::new());
    let mut auth = OwnedKvAuth::Team([1; 16]);
    let mut remint: Option<TeamViewRemint<'_>> = Some(Box::new(|| {
        Err(Error::TeamBinding(
            "team keys changed before the team view could be replaced",
        ))
    }));
    let error = call_with_remint(
        server(
            &[[1; 16]],
            foks_rpc::STATUS_TEAM_BEARER_TOKEN_STALE_ERROR,
            &seen,
        ),
        &mut auth,
        &mut remint,
        &KvRequest::Root,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        Error::Rpc(foks_rpc::Error::RemoteStatus { code: 7003, .. })
    ));
    assert_eq!(seen.borrow().len(), 1);
    assert!(matches!(auth, OwnedKvAuth::Team(token) if token == [1; 16]));
}
