mod common;
use foks_server_db::{Error, SsoSession, SsoSessionState as State};
fn row(id: u8) -> SsoSession {
    SsoSession {
        host: [1; 33],
        session_hash: [id; 32],
        config_hash: [2; 32],
        admission_hash: [3; 32],
        uid: None,
        state: State::Waiting,
        revision: 1,
        expires_at_ms: 600_100,
        ciphertext: vec![4; 57],
    }
}
#[test]
fn code_exchange_claims_are_single_owner_durable_and_config_fenced() {
    let mut fixture = common::TestDatabase::new();
    let db = &mut fixture.database;
    db.sso_set_policy(&[1; 33], &[2; 32], true).unwrap();
    let r = row(1);
    db.sso_insert_session(&r, 100).unwrap();
    db.sso_transition(&r, State::Exchanging, &[9; 57], 101)
        .unwrap();
    assert!(matches!(
        db.sso_transition(&r, State::Exchanging, &[9; 57], 101),
        Err(Error::AuthorizationChanged)
    ));
    assert_eq!(db.sso_abandon_exchanges(&[1; 33]).unwrap(), 1);
    let abandoned = db.sso_session(&[1; 33], &[1; 32]).unwrap().unwrap();
    assert_eq!(abandoned.state, State::ExchangeUnknown);
    assert!(db
        .sso_transition(&abandoned, State::Waiting, &[9; 57], 102)
        .is_err());
    let r = row(2);
    db.sso_insert_session(&r, 100).unwrap();
    db.sso_set_policy(&[1; 33], &[5; 32], true).unwrap();
    assert!(matches!(
        db.sso_transition(&r, State::Exchanging, &[9; 57], 101),
        Err(Error::AuthorizationChanged)
    ));
    assert!(db.sso_insert_session(&row(3), 100).is_err());
    db.sso_set_policy(&[1; 33], &[2; 32], false).unwrap();
    assert!(db
        .sso_transition(&r, State::Exchanging, &[9; 57], 101)
        .is_err());
}
#[test]
fn admission_is_bounded_even_for_terminal_flows_and_releases_on_expiry() {
    let mut fixture = common::TestDatabase::new();
    let db = &mut fixture.database;
    db.sso_set_policy(&[1; 33], &[2; 32], true).unwrap();
    for id in 1..=4 {
        let r = row(id);
        db.sso_insert_session(&r, 100).unwrap();
        db.sso_transition(&r, State::Denied, &[9; 57], 100).unwrap();
    }
    assert!(matches!(
        db.sso_insert_session(&row(5), 100),
        Err(Error::Capacity(_))
    ));
    let mut r = row(5);
    r.admission_hash = [7; 32];
    db.sso_insert_session(&r, 100).unwrap();
    assert!(db
        .sso_transition(&r, State::Exchanging, &[9; 57], 600_100)
        .is_err());
    let mut renewed = row(6);
    renewed.expires_at_ms = 1_200_100;
    db.sso_insert_session(&renewed, 600_100).unwrap();
    assert!(db.sso_session(&r.host, &r.session_hash).unwrap().is_none());
    assert!(!State::Ready.permits(State::Exchanging));
    assert!(!State::Completed.permits(State::Ready));
}
#[test]
fn host_admission_cannot_grow_without_bound() {
    let mut fixture = common::TestDatabase::new();
    let db = &mut fixture.database;
    db.sso_set_policy(&[1; 33], &[2; 32], true).unwrap();
    for id in 0u32..1000 {
        let mut r = row(1);
        r.session_hash[..4].copy_from_slice(&id.to_be_bytes());
        r.admission_hash = r.session_hash;
        db.sso_insert_session(&r, 100).unwrap();
    }
    let mut r = row(255);
    r.admission_hash = [255; 32];
    assert!(matches!(
        db.sso_insert_session(&r, 100),
        Err(Error::Capacity(_))
    ));
}
