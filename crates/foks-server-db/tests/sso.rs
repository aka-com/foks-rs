mod common;
use foks_server_db::{Error, SsoSession, SsoSessionState as State};
fn row(id: u8) -> SsoSession {
    SsoSession {
        authorization_epoch: 1,
        interrupted: false,
        host: [1; 33],
        session_hash: [id; 32],
        config_hash: [2; 32],
        source_hash: [3; 32],
        uid: None,
        state: State::Waiting,
        revision: 1,
        expires_at_ms: 600_100,
        ciphertext: vec![4; 57],
    }
}
#[test]
fn code_exchange_claims_are_single_owner_durable_and_config_blocked() {
    let mut fixture = common::TestDatabase::new();
    let db = &mut fixture.database;
    db.sso_activate(
        &[1; 33],
        &[1; 16],
        &[2; 32],
        "https://issuer.test",
        foks_server_db::SsoRolloutMode::Migration,
    )
    .unwrap();
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
    assert_eq!(abandoned.state, State::Exchanging);
    assert!(abandoned.interrupted);
    assert!(db
        .sso_transition(&abandoned, State::Waiting, &[9; 57], 102)
        .is_err());
    let r = row(2);
    db.sso_insert_session(&r, 100).unwrap();
    let expected = db.sso_policy(&[1; 33]).unwrap().unwrap();
    db.sso_transition_policy(
        &expected,
        &foks_server_db::SsoPolicyTransition::ReplaceProvider {
            config_hash: [5; 32],
            issuer: "https://issuer.test".into(),
        },
    )
    .unwrap();
    assert!(matches!(
        db.sso_transition(&r, State::Exchanging, &[9; 57], 101),
        Err(Error::AuthorizationChanged)
    ));
    assert!(db.sso_insert_session(&row(3), 100).is_err());
    db.sso_disable_policy().unwrap();
    assert!(db
        .sso_transition(&r, State::Exchanging, &[9; 57], 101)
        .is_err());
}
#[test]
fn admission_is_bounded_even_for_terminal_flows_and_releases_on_expiry() {
    let mut fixture = common::TestDatabase::new();
    let db = &mut fixture.database;
    db.sso_activate(
        &[1; 33],
        &[1; 16],
        &[2; 32],
        "https://issuer.test",
        foks_server_db::SsoRolloutMode::Migration,
    )
    .unwrap();
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
    r.source_hash = [7; 32];
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
    db.sso_activate(
        &[1; 33],
        &[1; 16],
        &[2; 32],
        "https://issuer.test",
        foks_server_db::SsoRolloutMode::Migration,
    )
    .unwrap();
    for id in 0u32..1000 {
        let mut r = row(1);
        r.session_hash[..4].copy_from_slice(&id.to_be_bytes());
        r.source_hash = r.session_hash;
        db.sso_insert_session(&r, 100).unwrap();
    }
    let mut r = row(255);
    r.source_hash = [255; 32];
    assert!(matches!(
        db.sso_insert_session(&r, 100),
        Err(Error::Capacity(_))
    ));
}

#[test]
fn activation_snapshots_existing_accounts_and_enforcement_is_counted_cas() {
    use foks_server_db::{SsoAccessDecision as D, SsoPolicyTransition as T, SsoRolloutMode as M};
    let mut fixture = common::TestDatabase::new();
    fixture.reserve(1_000_000);
    fixture.commit(None).unwrap();
    let db = &mut fixture.database;
    assert_eq!(db.sso_access_decision(&[1; 33], 100).unwrap(), D::NoPolicy);
    assert!(db
        .sso_activate(
            &[2; 33],
            &[1; 16],
            &[2; 32],
            "https://issuer.test",
            M::Enforced
        )
        .is_err());
    assert!(db.sso_policy(&[2; 33]).unwrap().is_none());
    let p = db
        .sso_activate(
            &[2; 33],
            &[1; 16],
            &[2; 32],
            "https://issuer.test",
            M::Migration,
        )
        .unwrap();
    let status = db.sso_rollout_status(&p.host).unwrap().unwrap();
    assert_eq!((status.cohort, status.linked, status.unlinked), (1, 0, 1));
    assert_eq!(
        db.sso_access_decision(&[1; 33], 100).unwrap(),
        D::MigrationEligible
    );
    assert!(!D::MigrationEligible.permits_administration());
    assert_eq!(db.sso_access_decision(&[9; 33], 100).unwrap(), D::Denied);
    for (count, accept) in [(0, false), (0, true), (1, false)] {
        assert!(db
            .sso_transition_policy(
                &p,
                &T::Enforce {
                    expected_unlinked: count,
                    accept_lockout: accept
                }
            )
            .is_err());
    }
    let enforced = db
        .sso_transition_policy(
            &p,
            &T::Enforce {
                expected_unlinked: 1,
                accept_lockout: true,
            },
        )
        .unwrap();
    assert_eq!(enforced.authorization_epoch, p.authorization_epoch);
    assert_eq!(db.sso_access_decision(&[1; 33], 100).unwrap(), D::LinkOnly);
    assert!(db
        .sso_transition_policy(
            &p,
            &T::Enforce {
                expected_unlinked: 1,
                accept_lockout: true
            }
        )
        .is_err());
    assert!(db
        .sso_activate(&p.host, &[3; 16], &p.config_hash, &p.issuer, M::Migration)
        .is_err());
}

#[test]
fn block_reason_is_durable_idempotent_and_reenable_never_resurrects_old_sessions() {
    use foks_server_db::{
        SsoPolicyTransition as T, SsoProviderBlockReason as F, SsoRolloutMode as M,
    };
    let mut fixture = common::TestDatabase::new();
    let db = &mut fixture.database;
    let p = db
        .sso_activate(
            &[1; 33],
            &[1; 16],
            &[2; 32],
            "https://issuer.test",
            M::Migration,
        )
        .unwrap();
    let r = row(1);
    db.sso_insert_session(&r, 100).unwrap();
    db.sso_block_policy(&p.host, F::Operator).unwrap();
    let blocked = db.sso_policy(&p.host).unwrap().unwrap();
    db.sso_block_policy(&p.host, F::Operator).unwrap();
    assert_eq!(db.sso_policy(&p.host).unwrap().unwrap(), blocked);
    assert_eq!(blocked.authorization_epoch, p.authorization_epoch + 1);
    assert!(db.sso_transition_policy(&p, &T::Reenable).is_err());
    let enabled = db.sso_transition_policy(&blocked, &T::Reenable).unwrap();
    assert_eq!(enabled.authorization_epoch, p.authorization_epoch + 2);
    assert_eq!(enabled.mode, p.mode);
    assert!(db
        .sso_transition(&r, State::Exchanging, &[9; 57], 101)
        .is_err());
    assert!(db.sso_insert_session(&row(2), 101).is_err());
    let mut fresh = row(2);
    fresh.authorization_epoch = enabled.authorization_epoch;
    db.sso_insert_session(&fresh, 101).unwrap();
    assert!(db
        .sso_transition_policy(
            &enabled,
            &T::ReplaceProvider {
                config_hash: [3; 32],
                issuer: "https://different.test".into()
            }
        )
        .is_err());
}

fn ready_login(db: &mut foks_server_db::Database, id: u8) -> SsoSession {
    let mut flow = row(id);
    flow.uid = Some([1; 33]);
    db.sso_insert_session(&flow, 100).unwrap();
    db.sso_transition(&flow, State::Exchanging, &[5; 57], 101)
        .unwrap();
    flow = db
        .sso_session(&flow.host, &flow.session_hash)
        .unwrap()
        .unwrap();
    db.sso_transition(&flow, State::Ready, &[6; 57], 102)
        .unwrap();
    db.sso_session(&flow.host, &flow.session_hash)
        .unwrap()
        .unwrap()
}
fn first_binding(flow: SsoSession, subject: &str) -> foks_server_db::SsoAccountBinding {
    foks_server_db::SsoAccountBinding {
        purpose: foks_proto::SsoPurpose::LinkExisting,
        commitment: flow.session_hash,
        access: foks_server_db::SsoAccess {
            host: flow.host,
            uid: [1; 33],
            issuer: "https://issuer.test".into(),
            subject: subject.into(),
            config_hash: flow.config_hash,
            revision: 1,
            authorization_epoch: flow.authorization_epoch,
            authorization_generation: 1,
            interrupted: false,
            state: foks_server_db::SsoAccessState::Active,
            expires_at_ms: 500,
            ciphertext: vec![8; 57],
        },
        flow,
        completed_ciphertext: vec![7; 57],
        device: vec![4; 33],
        expected_user_sequence: Some(1),
    }
}
#[test]
fn first_link_is_create_only_atomic_and_available_after_enforcement() {
    use foks_server_db::{SsoAccessDecision as D, SsoPolicyTransition as T, SsoRolloutMode as M};
    let mut fixture = common::TestDatabase::new();
    fixture.reserve(1_000_000);
    fixture.commit(None).unwrap();
    let db = &mut fixture.database;
    let p = db
        .sso_activate(
            &[1; 33],
            &[1; 16],
            &[2; 32],
            "https://issuer.test",
            M::Migration,
        )
        .unwrap();
    assert!(db.sso_authorization_binding(&[1; 33], 100).is_err());
    let flow = ready_login(db, 1);
    let competing = ready_login(db, 2);
    db.sso_transition_policy(
        &p,
        &T::Enforce {
            expected_unlinked: 1,
            accept_lockout: true,
        },
    )
    .unwrap();
    let binding = first_binding(flow, "subject-a");
    db.sso_login(&binding, 103).unwrap();
    db.sso_login(&binding, 104).unwrap();
    assert!(db
        .sso_login(&first_binding(competing, "subject-b"), 104)
        .is_err());
    assert_eq!(
        db.sso_access(&[1; 33]).unwrap().unwrap().subject,
        "subject-a"
    );
    assert_eq!(db.sso_rollout_status(&p.host).unwrap().unwrap().unlinked, 0);
    assert_eq!(
        db.sso_access_decision(&[1; 33], 104).unwrap(),
        D::LinkedActive
    );
    assert_eq!(
        db.sso_authorization_binding(&[1; 33], 104).unwrap(),
        foks_server_db::AuthorizationBinding::Linked {
            host: [1; 33],
            config_hash: [2; 32],
            policy_epoch: 1,
            account_generation: 1
        }
    );
    assert!(db.sso_authorization_binding(&[1; 33], 500).is_err());
    assert_eq!(db.sso_access_decision(&[1; 33], 500).unwrap(), D::Denied);
    assert_eq!(
        db.sso_session(&[1; 33], &[2; 32]).unwrap().unwrap().state,
        State::Ready
    );
}
#[test]
fn revoked_or_nonowner_first_link_does_not_consume_flow_or_receipt_capacity() {
    let mut fixture = common::TestDatabase::new();
    fixture.reserve(1_000_000);
    fixture.commit(None).unwrap();
    fixture
        .database
        .sso_activate(
            &[1; 33],
            &[1; 16],
            &[2; 32],
            "https://issuer.test",
            foks_server_db::SsoRolloutMode::Migration,
        )
        .unwrap();
    let binding = first_binding(ready_login(&mut fixture.database, 1), "subject-a");
    let sql = rusqlite::Connection::open(&fixture.path).unwrap();
    for update in [
        "UPDATE devices SET role_type=1",
        "UPDATE devices SET role_type=3,active=0",
    ] {
        sql.execute(update, []).unwrap();
        assert!(fixture.database.sso_login(&binding, 103).is_err());
        assert!(fixture.database.sso_access(&[1; 33]).unwrap().is_none());
        assert_eq!(
            fixture
                .database
                .sso_session(&[1; 33], &[1; 32])
                .unwrap()
                .unwrap()
                .state,
            State::Ready
        );
        assert_eq!(
            sql.query_row("SELECT count(*) FROM sso_binding_receipts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
#[test]
fn receipt_capacity_is_committed_only_and_exact_repeat_succeeds_when_full() {
    let mut fixture = common::TestDatabase::new();
    fixture.reserve(1_000_000);
    fixture.commit(None).unwrap();
    fixture
        .database
        .sso_activate(
            &[1; 33],
            &[1; 16],
            &[2; 32],
            "https://issuer.test",
            foks_server_db::SsoRolloutMode::Migration,
        )
        .unwrap();
    let binding = first_binding(ready_login(&mut fixture.database, 1), "subject-a");
    fixture.database.sso_login(&binding, 103).unwrap();
    let mut sql = rusqlite::Connection::open(&fixture.path).unwrap();
    let tx = sql.transaction().unwrap();
    for n in 0u32..4095 {
        let mut hash = [0; 32];
        hash[..4].copy_from_slice(&n.to_be_bytes());
        tx.execute(
            "INSERT INTO sso_binding_receipts VALUES(?1,?2,?3,2,1,1,10000)",
            rusqlite::params![[1u8; 33], [1u8; 33], hash],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    fixture.database.sso_login(&binding, 104).unwrap();
    let mut reauth = first_binding(ready_login(&mut fixture.database, 2), "subject-a");
    reauth.purpose = foks_proto::SsoPurpose::Reauthenticate;
    reauth.access.revision = 2;
    reauth.access.authorization_generation = 2;
    assert!(matches!(
        fixture.database.sso_login(&reauth, 104),
        Err(Error::Capacity(_))
    ));
    assert_eq!(
        fixture
            .database
            .sso_access(&[1; 33])
            .unwrap()
            .unwrap()
            .authorization_generation,
        1
    );
    assert_eq!(
        fixture
            .database
            .sso_session(&[1; 33], &[2; 32])
            .unwrap()
            .unwrap()
            .state,
        State::Ready
    );
}

#[test]
fn identity_proof_is_one_use_host_bound_and_survives_provider_blocking() {
    let mut fixture = common::TestDatabase::new();
    fixture.reserve(1_000_000);
    fixture.commit(None).unwrap();
    let seed = foks_proto::SecretSeed::new([19; 32]);
    let signer = foks_crypto::derive_device_public(&seed).unwrap().id;
    let sql = rusqlite::Connection::open(&fixture.path).unwrap();
    sql.execute("INSERT INTO devices(device_id,uid,active,role_type,visibility,hepk_fingerprint,self_token,exact_hepk,exact_name) SELECT ?1,uid,1,3,visibility,hepk_fingerprint,?2,exact_hepk,exact_name FROM devices LIMIT 1",rusqlite::params![signer.as_bytes(),[9u8;17]]).unwrap();
    let db = &mut fixture.database;
    db.sso_activate(
        &[2; 33],
        &[1; 16],
        &[2; 32],
        "https://issuer.test",
        foks_server_db::SsoRolloutMode::Migration,
    )
    .unwrap();
    let claim = foks_proto::IdentityClaim {
        host: foks_proto::EntityId::from_bytes(vec![2; 33]).unwrap(),
        uid: foks_proto::EntityId::from_bytes(vec![1; 33]).unwrap(),
        signer,
        receipt_commitment: None,
    };
    let ch = db
        .sso_issue_identity_challenge(&claim, [1; 32], [1; 32], 100)
        .unwrap();
    let proof = foks_crypto::sign_identity_proof(&seed, ch).unwrap();
    assert!(db.sso_prove_identity(&[3; 33], &proof, 101).is_err());
    let mut tampered = proof.clone();
    tampered.challenge.claim.receipt_commitment = Some([1; 32]);
    assert!(db.sso_prove_identity(&[2; 33], &tampered, 101).is_err());
    db.sso_disable_policy().unwrap();
    let status = db.sso_prove_identity(&[2; 33], &proof, 101).unwrap();
    assert_eq!(
        status.account_state,
        foks_proto::SsoAccountState::MigrationEligible
    );
    assert!(!status.access_available);
    assert_ne!(status.provider_blocked_reason, 0);
    assert!(db.sso_prove_identity(&[2; 33], &proof, 102).is_err());
    let ch = db
        .sso_issue_identity_challenge(&claim, [2; 32], [1; 32], 100)
        .unwrap();
    let proof = foks_crypto::sign_identity_proof(&seed, ch).unwrap();
    sql.execute("UPDATE devices SET active=0", []).unwrap();
    assert!(db.sso_prove_identity(&[2; 33], &proof, 101).is_err());
}

#[test]
fn identity_challenge_quota_depends_on_source_not_claimed_account() {
    let mut fixture = common::TestDatabase::new();
    let mut claim = foks_proto::IdentityClaim {
        host: foks_proto::EntityId::from_bytes(vec![2; 33]).unwrap(),
        uid: foks_proto::EntityId::from_bytes(vec![1; 33]).unwrap(),
        signer: foks_proto::EntityId::from_bytes(vec![4; 33]).unwrap(),
        receipt_commitment: None,
    };
    for n in 1..=8 {
        let mut uid = vec![n; 33];
        uid[0] = foks_proto::ENTITY_USER;
        claim.uid = foks_proto::EntityId::from_bytes(uid).unwrap();
        fixture
            .database
            .sso_issue_identity_challenge(&claim, [n; 32], [1; 32], 100)
            .unwrap();
    }
    assert!(matches!(
        fixture
            .database
            .sso_issue_identity_challenge(&claim, [9; 32], [1; 32], 100),
        Err(Error::Capacity(_))
    ));
    fixture
        .database
        .sso_issue_identity_challenge(&claim, [9; 32], [2; 32], 100)
        .unwrap();
    fixture
        .database
        .sso_issue_identity_challenge(&claim, [10; 32], [1; 32], 60_100)
        .unwrap();
}

#[test]
fn web_session_authorization_binding_never_treats_migration_eligibility_as_linked_access() {
    let mut fixture = common::TestDatabase::new();
    let db = &mut fixture.database;
    assert_eq!(
        db.sso_authorization_binding(&[2; 33], 100).unwrap(),
        foks_server_db::AuthorizationBinding::Unconfigured
    );
    db.sso_activate(
        &[1; 33],
        &[1; 16],
        &[2; 32],
        "https://issuer.test",
        foks_server_db::SsoRolloutMode::Migration,
    )
    .unwrap();
    assert!(db.sso_authorization_binding(&[2; 33], 100).is_err());
}
