mod common;
use foks_server_db::*;
use rusqlite::{params, Connection};
const HOST: [u8; 33] = [2; 33];
const UID: [u8; 33] = [1; 33];
fn now() -> AdminMoment {
    AdminMoment {
        epoch: [1; 16],
        utc_us: 2_000_000,
        elapsed_us: 100,
    }
}
fn credential() -> WebCredential {
    WebCredential {
        host: HOST,
        uid: UID,
        credential: [4; 33],
        certificate_expires_at_us: 1_000_000_000,
    }
}
fn fixture() -> common::TestDatabase {
    let mut f = common::TestDatabase::new();
    f.reserve(1_000_000);
    f.commit(None).unwrap();
    Connection::open(&f.path)
        .unwrap()
        .execute(
            "INSERT INTO host_metadata VALUES(1,?1,'host.test',X'01',X'01')",
            [HOST],
        )
        .unwrap();
    f
}
fn auth() -> WebMutationAuth {
    WebMutationAuth {
        session_hash: [4; 32],
        csrf_hash: [5; 32],
    }
}
fn session(f: &mut common::TestDatabase, operator: bool) {
    let d = &mut f.database;
    if operator {
        d.admin_set_grant(&HOST, &UID, true, "operator", now().utc_us)
            .unwrap();
    }
    d.web_issue_ticket(&credential(), &[1; 32], &[1; 16], now())
        .unwrap();
    d.web_stage(&[1; 32], &[2; 32], &[3; 32], None, now())
        .unwrap();
    d.web_redeem(
        foks_server_db::WebRedemption {
            binding: &[2; 32],
            csrf: &[3; 32],
            session: &[4; 32],
            session_csrf: &[5; 32],
            id: &[4; 16],
            existing: None,
        },
        now(),
    )
    .unwrap();
}
#[test]
fn ticket_confirmation_is_single_use_owner_checked_and_epoch_bound() {
    let mut f = fixture();
    f.database
        .web_issue_ticket(&credential(), &[1; 32], &[1; 16], now())
        .unwrap();
    let read = ReadDatabase::open(&f.path, Default::default()).unwrap();
    assert!(matches!(
        read.snapshot()
            .unwrap()
            .web_check_ticket(&[1; 32], &[9; 33], now()),
        Err(Error::WebWrongUser)
    ));
    f.database
        .web_stage(&[1; 32], &[2; 32], &[3; 32], None, now())
        .unwrap();
    f.database
        .web_stage(&[1; 32], &[6; 32], &[7; 32], None, now())
        .unwrap();
    assert!(f
        .database
        .web_redeem(
            foks_server_db::WebRedemption {
                binding: &[2; 32],
                csrf: &[8; 32],
                session: &[4; 32],
                session_csrf: &[5; 32],
                id: &[4; 16],
                existing: None
            },
            now()
        )
        .is_err());
    f.database
        .web_redeem(
            foks_server_db::WebRedemption {
                binding: &[2; 32],
                csrf: &[3; 32],
                session: &[4; 32],
                session_csrf: &[5; 32],
                id: &[4; 16],
                existing: None,
            },
            now(),
        )
        .unwrap();
    assert!(f
        .database
        .web_redeem(
            foks_server_db::WebRedemption {
                binding: &[6; 32],
                csrf: &[7; 32],
                session: &[8; 32],
                session_csrf: &[9; 32],
                id: &[8; 16],
                existing: None
            },
            now()
        )
        .is_err());
    assert!(read
        .snapshot()
        .unwrap()
        .web_check_ticket(&[1; 32], &UID, now())
        .is_err());
    let ctx = read
        .snapshot()
        .unwrap()
        .web_context(&[4; 32], now())
        .unwrap();
    assert!(!ctx.operator);
    assert_eq!(ctx.credential.uid, UID);
    let mut later = now();
    later.epoch = [2; 16];
    assert!(read
        .snapshot()
        .unwrap()
        .web_context(&[4; 32], later)
        .is_err());
    later = now();
    later.elapsed_us += WEB_SESSION_US;
    assert!(read
        .snapshot()
        .unwrap()
        .web_context(&[4; 32], later)
        .is_err());
}
#[test]
fn operator_grant_and_credential_are_reloaded_inside_mutations() {
    let mut f = fixture();
    session(&mut f, true);
    f.database
        .web_set_policy(&auth(), InviteRegime::Required, 1, now())
        .unwrap();
    f.database
        .admin_set_grant(&HOST, &UID, false, "revoked", now().utc_us)
        .unwrap();
    assert!(f
        .database
        .web_set_policy(&auth(), InviteRegime::Optional, 2, now())
        .is_err());
    f.database
        .admin_set_grant(&HOST, &UID, true, "restored", now().utc_us)
        .unwrap();
    Connection::open(&f.path)
        .unwrap()
        .execute("UPDATE devices SET active=0", [])
        .unwrap();
    assert!(f
        .database
        .web_set_policy(&auth(), InviteRegime::Optional, 2, now())
        .is_err());
    assert_eq!(
        f.database.invite_policy().unwrap().regime,
        InviteRegime::Required
    );
}
#[test]
fn invite_receipt_survives_form_deadline_and_configuration_cas_ignores_redemption() {
    let mut f = fixture();
    session(&mut f, true);
    f.database
        .web_prepare_invite(&auth(), &[10; 32], now())
        .unwrap();
    let invite = WebInvite {
        nonce_hash: [10; 32],
        id: [10; 16],
        code_hash: [11; 32],
    };
    assert_eq!(
        f.database.web_issue_invite(&auth(), invite, now()).unwrap(),
        WebInviteResult::Created([10; 16])
    );
    let mut later = now();
    later.utc_us += WEB_TICKET_US + 1;
    later.elapsed_us += WEB_TICKET_US + 1;
    f.database.web_cleanup(later).unwrap();
    assert_eq!(
        f.database.web_issue_invite(&auth(), invite, later).unwrap(),
        WebInviteResult::AlreadyCreated([10; 16])
    );
    let sql = Connection::open(&f.path).unwrap();
    sql.execute(
        "UPDATE signup_invites SET use_count=1 WHERE invite_id=?1",
        [[10u8; 16]],
    )
    .unwrap();
    f.database
        .web_disable_invite(&auth(), &[10; 16], 1, later)
        .unwrap();
    assert!(f
        .database
        .web_disable_invite(&auth(), &[10; 16], 1, later)
        .is_err());
    assert_eq!(f.database.invites().unwrap()[0].configuration_revision, 2);
}
#[test]
fn audit_failure_rolls_back_the_invite_and_its_nonce() {
    let mut f = fixture();
    session(&mut f, true);
    f.database
        .web_prepare_invite(&auth(), &[10; 32], now())
        .unwrap();
    let sql = Connection::open(&f.path).unwrap();
    sql.execute_batch("CREATE TRIGGER fail_audit BEFORE INSERT ON admin_audit WHEN NEW.action=7 BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
    let invite = WebInvite {
        nonce_hash: [10; 32],
        id: [10; 16],
        code_hash: [11; 32],
    };
    assert!(f.database.web_issue_invite(&auth(), invite, now()).is_err());
    assert!(f.database.invites().unwrap().is_empty());
    sql.execute_batch("DROP TRIGGER fail_audit;").unwrap();
    assert_eq!(
        f.database.web_issue_invite(&auth(), invite, now()).unwrap(),
        WebInviteResult::Created([10; 16])
    );
}
#[test]
fn self_sessions_cannot_mutate_host_policy_and_revoke_all_invalidates_tickets() {
    let mut f = fixture();
    session(&mut f, false);
    assert!(f
        .database
        .web_set_policy(&auth(), InviteRegime::Required, 1, now())
        .is_err());
    f.database
        .web_issue_ticket(&credential(), &[8; 32], &[8; 16], now())
        .unwrap();
    f.database
        .web_stage(&[8; 32], &[9; 32], &[10; 32], None, now())
        .unwrap();
    f.database.web_revoke(&auth(), &UID, None, now()).unwrap();
    let read = ReadDatabase::open(&f.path, Default::default()).unwrap();
    assert!(read
        .snapshot()
        .unwrap()
        .web_context(&[4; 32], now())
        .is_err());
    assert!(read
        .snapshot()
        .unwrap()
        .web_check_ticket(&[8; 32], &UID, now())
        .is_err());
    assert!(read
        .snapshot()
        .unwrap()
        .web_confirmation(&[9; 32], now())
        .is_err());
}
#[test]
fn web_session_authorization_binding_allows_refresh_revision_but_rejects_new_binding_generation() {
    let mut f = fixture();
    f.database
        .sso_activate(
            &HOST,
            &[1; 16],
            &[2; 32],
            "https://issuer.test",
            SsoRolloutMode::Migration,
        )
        .unwrap();
    assert!(f
        .database
        .web_issue_ticket(&credential(), &[1; 32], &[1; 16], now())
        .is_err());
    let sql = Connection::open(&f.path).unwrap();
    sql.execute("INSERT INTO sso_access(host,uid,issuer,subject,config_hash,revision,authorization_epoch,authorization_generation,interrupted,state,expires_at_ms,ciphertext) VALUES(?1,?2,'https://issuer.test','subject',?3,1,1,1,0,0,1000000,?4)",params![HOST,UID,[2u8;32],vec![0u8;57]]).unwrap();
    session(&mut f, true);
    let read = ReadDatabase::open(&f.path, Default::default()).unwrap();
    sql.execute("UPDATE sso_access SET revision=revision+1", [])
        .unwrap();
    assert!(read
        .snapshot()
        .unwrap()
        .web_context(&[4; 32], now())
        .is_ok());
    sql.execute("UPDATE sso_access SET authorization_generation=2", [])
        .unwrap();
    assert!(read
        .snapshot()
        .unwrap()
        .web_context(&[4; 32], now())
        .is_err());
}
#[test]
fn ticket_and_nonce_caps_reject_without_evicting_live_records() {
    let mut f = fixture();
    for i in 1..=3 {
        f.database
            .web_issue_ticket(&credential(), &[i; 32], &[i; 16], now())
            .unwrap();
    }
    assert!(matches!(
        f.database
            .web_issue_ticket(&credential(), &[4; 32], &[4; 16], now()),
        Err(Error::Capacity(_))
    ));
    let mut f = fixture();
    session(&mut f, true);
    for i in 1..=8 {
        f.database
            .web_prepare_invite(&auth(), &[i; 32], now())
            .unwrap();
    }
    assert!(matches!(
        f.database.web_prepare_invite(&auth(), &[9; 32], now()),
        Err(Error::Capacity(_))
    ));
}
#[test]
fn foreign_host_bot_backup_and_invalid_csrf_are_denied() {
    let mut f = fixture();
    let mut v = credential();
    v.host = [3; 33];
    assert!(f
        .database
        .web_issue_ticket(&v, &[1; 32], &[1; 16], now())
        .is_err());
    for kind in [
        foks_proto::ENTITY_BOT_TOKEN_KEY,
        foks_proto::ENTITY_BACKUP_KEY,
    ] {
        v = credential();
        v.credential[0] = kind;
        assert!(f
            .database
            .web_issue_ticket(&v, &[1; 32], &[1; 16], now())
            .is_err());
    }
    session(&mut f, true);
    let mut bad = auth();
    bad.csrf_hash = [8; 32];
    assert!(f
        .database
        .web_set_policy(&bad, InviteRegime::Required, 1, now())
        .is_err());
}

#[test]
fn delegated_browser_credential_is_invalidated_by_parent_revocation() {
    let mut f = fixture();
    let sql = Connection::open(&f.path).unwrap();
    let subkey = [13u8; 33];
    sql.execute("UPDATE devices SET subkey_id=?1", [subkey])
        .unwrap();
    let mut v = credential();
    v.credential = subkey;
    f.database
        .web_issue_ticket(&v, &[1; 32], &[1; 16], now())
        .unwrap();
    f.database
        .web_stage(&[1; 32], &[2; 32], &[3; 32], None, now())
        .unwrap();
    f.database
        .web_redeem(
            foks_server_db::WebRedemption {
                binding: &[2; 32],
                csrf: &[3; 32],
                session: &[4; 32],
                session_csrf: &[5; 32],
                id: &[4; 16],
                existing: None,
            },
            now(),
        )
        .unwrap();
    let read = ReadDatabase::open(&f.path, Default::default()).unwrap();
    assert!(read
        .snapshot()
        .unwrap()
        .web_context(&[4; 32], now())
        .is_ok());
    sql.execute("UPDATE devices SET active=0", []).unwrap();
    assert!(read
        .snapshot()
        .unwrap()
        .web_context(&[4; 32], now())
        .is_err());
}

#[test]
fn general_expiry_maintenance_is_bounded_without_losing_remaining_work() {
    let mut f = fixture();
    let mut sql = Connection::open(&f.path).unwrap();
    let tx = sql.transaction().unwrap();
    for id in 0u32..300 {
        let mut token = [0u8; 17];
        token[..4].copy_from_slice(&id.to_be_bytes());
        tx.execute("INSERT INTO names(normalized_name,reservation_token,reservation_sequence,expires_at) VALUES(?1,?2,1,1)",params![format!("pending{id}").into_bytes(),token]).unwrap();
    }
    tx.commit().unwrap();
    assert_eq!(
        f.database
            .run_maintenance(now().utc_us, 0)
            .unwrap()
            .reservations,
        128
    );
    assert_eq!(
        f.database
            .run_maintenance(now().utc_us, 0)
            .unwrap()
            .reservations,
        128
    );
    assert_eq!(
        f.database
            .run_maintenance(now().utc_us, 0)
            .unwrap()
            .reservations,
        44
    );
}
