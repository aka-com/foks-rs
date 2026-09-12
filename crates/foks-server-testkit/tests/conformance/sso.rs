use foks_client::{
    CreatedSoftwareAccount, FederationCredential, SoftwareAccountRequest, SoftwareAccountSecrets,
    SsoIntent, SsoSigningKey,
};
use foks_client_db::SsoFlowState;
use foks_oidc::{NetworkPolicy, ProviderHttp};
use foks_proto::{EntityId, InviteCode, SecretSeed, ENTITY_PUK_VERIFY, ENTITY_USER};
use foks_server_testkit::{oidc::TestOidcProvider, TestClient, TestEnvironment};

fn signup(
    environment: &TestEnvironment,
    client: &TestClient,
    provider: &TestOidcProvider,
) -> CreatedSoftwareAccount {
    let host = client.probe_and_pin().unwrap().pinned;
    let foks = client.foks();
    let http = ProviderHttp::new(NetworkPolicy::loopback_test()).unwrap();
    let mut protected = client.open_protected_store().unwrap();
    let device = SecretSeed::new([42; 32]);
    let puk = SecretSeed::new([43; 32]);
    let device_id = foks_crypto::derive_device_public(&device).unwrap().id;
    let mut uid = foks_crypto::derive_shared_verify_key(&puk, ENTITY_PUK_VERIFY)
        .unwrap()
        .as_bytes()
        .to_vec();
    uid[0] = ENTITY_USER;
    let uid = EntityId::from_bytes(uid).unwrap();
    let flow = foks
        .begin_sso(
            &host,
            SsoIntent {
                uid: uid.clone(),
                device: device_id,
                for_login: false,
            },
            &http,
            &mut protected,
        )
        .unwrap();
    provider.complete(flow.browser_url.as_ref().unwrap());
    assert_eq!(
        foks.poll_sso(&host, flow.operation_id, 1000, &http, &mut protected)
            .unwrap()
            .state,
        SsoFlowState::Ready
    );
    let authorization = foks
        .authorize_sso_signup(
            &host,
            flow.operation_id,
            SsoSigningKey::Software(&device),
            &http,
            &mut protected,
        )
        .unwrap();
    let created = foks
        .create_software_account_with_sso(
            &host,
            SoftwareAccountRequest {
                username_utf8: "ssoalice".into(),
                device_name: "sso desktop".into(),
                invite_code: InviteCode::Empty,
                email: "ssoalice@example.test".into(),
                passphrase: None,
            },
            SoftwareAccountSecrets::new(device, puk, [33; 17]),
            client.soft_state_path(),
            &authorization,
            &mut protected,
        )
        .unwrap();
    assert_eq!(foks.ping(&host, &created.credential).unwrap(), uid);
    let database = environment.read_database().unwrap();
    let access = database.sso_access(uid.as_bytes()).unwrap().unwrap();
    assert_eq!(access.subject, "alice-subject");
    assert_eq!(access.state, foks_server_db::SsoAccessState::Active);
    created
}
#[test]
pub(crate) fn sso_signup_login_and_expiry_enforcement() {
    let provider = TestOidcProvider::start();
    let environment = TestEnvironment::new().unwrap();
    let config = provider.config(environment.client_path("oidc", "secret").unwrap());
    let server = environment.start_oidc_server(config.clone()).unwrap();
    let client = TestClient::new(&environment, "oidc-account").unwrap();
    let created = signup(&environment, &client, &provider);
    let host = client.pinned_host().unwrap();
    let foks = client.foks();
    let http = ProviderHttp::new(NetworkPolicy::loopback_test()).unwrap();
    let mut protected = client.open_protected_store().unwrap();
    let reauth = foks
        .begin_sso(
            &host,
            SsoIntent {
                uid: created.credential.uid.clone(),
                device: foks_crypto::derive_device_public(&created.credential.seed)
                    .unwrap()
                    .id,
                for_login: true,
            },
            &http,
            &mut protected,
        )
        .unwrap();
    provider.complete(reauth.browser_url.as_ref().unwrap());
    foks.poll_sso(&host, reauth.operation_id, 1000, &http, &mut protected)
        .unwrap();
    assert_eq!(
        foks.finish_sso_login(
            &host,
            reauth.operation_id,
            FederationCredential::Software(&created.credential),
            &http,
            &mut protected
        )
        .unwrap()
        .state,
        SsoFlowState::Complete
    );
    // Reuse the client's authenticated connection after expiry. A certificate is insufficient.
    environment.advance_clock(301_000_000);
    assert_eq!(
        foks.ping(&host, &created.credential).unwrap(),
        created.credential.uid
    );
    assert_eq!(provider.refreshes(), 1);
    provider.set_invalid_grant(true);
    environment.advance_clock(301_000_000);
    assert!(matches!(
        foks.ping(&host, &created.credential),
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 1069,
            ..
        }))
    ));
    assert_eq!(provider.refreshes(), 1); // Invalid grant is rejected before the fixture consumes it.
    server.shutdown().unwrap();
    let reopened = environment.start_oidc_server(config).unwrap();
    assert!(foks.ping(&host, &created.credential).is_err());
    reopened.shutdown().unwrap();
    // Removing provider config retains its policy/linkage and does not unlock old accounts.
    let disabled = environment.start_server().unwrap();
    assert!(foks.ping(&host, &created.credential).is_err());
    disabled.shutdown().unwrap();
}
#[test]
fn provider_subject_cannot_retarget_a_foks_account() {
    let provider = TestOidcProvider::start();
    let environment = TestEnvironment::new().unwrap();
    let config = provider.config(environment.client_path("oidc", "secret").unwrap());
    let _server = environment.start_oidc_server(config).unwrap();
    let client = TestClient::new(&environment, "oidc-account").unwrap();
    let created = signup(&environment, &client, &provider);
    let host = client.pinned_host().unwrap();
    let foks = client.foks();
    let http = ProviderHttp::new(NetworkPolicy::loopback_test()).unwrap();
    let mut protected = client.open_protected_store().unwrap();
    provider.identity("ssoalice", "different-subject");
    let flow = foks
        .begin_sso(
            &host,
            SsoIntent {
                uid: created.credential.uid.clone(),
                device: foks_crypto::derive_device_public(&created.credential.seed)
                    .unwrap()
                    .id,
                for_login: true,
            },
            &http,
            &mut protected,
        )
        .unwrap();
    provider.complete(flow.browser_url.as_ref().unwrap());
    foks.poll_sso(&host, flow.operation_id, 1000, &http, &mut protected)
        .unwrap();
    assert!(matches!(
        foks.finish_sso_login(
            &host,
            flow.operation_id,
            FederationCredential::Software(&created.credential),
            &http,
            &mut protected
        ),
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 1067,
            ..
        }))
    ));
    assert_eq!(
        environment
            .read_database()
            .unwrap()
            .sso_access(created.credential.uid.as_bytes())
            .unwrap()
            .unwrap()
            .subject,
        "alice-subject"
    );
}

#[test]
fn every_authenticated_route_is_gated_and_public_reauthentication_remains_reachable() {
    use foks_proto::RealtimeWire as _;
    use std::io::Write as _;
    let provider = TestOidcProvider::start();
    let environment = TestEnvironment::new().unwrap();
    let config = provider.config(environment.client_path("oidc", "secret").unwrap());
    let server = environment.start_oidc_server(config).unwrap();
    let client = TestClient::new(&environment, "oidc-account").unwrap();
    let created = signup(&environment, &client, &provider);
    let probe = client.probe_and_pin().unwrap();
    let fixture = crate::support::Fixture {
        environment,
        server,
        client,
        probe,
    };
    let mut stream = crate::authorization::authenticated_stream(&fixture, &created.credential);
    stream
        .write_all(
            &foks_rpc::encode_call(
                foks_rpc::USER_PROTOCOL_ID,
                foks_rpc::USER_PING_METHOD_POSITION,
                &[0xc0],
                0,
            )
            .unwrap(),
        )
        .unwrap();
    foks_rpc::read_response(&mut stream, 4096, 0).unwrap();
    provider.set_invalid_grant(true);
    fixture.environment.advance_clock(301_000_000);
    for route in foks_server::rpc::ROUTES
        .iter()
        .filter(|r| r.supported && r.listeners.contains(&"authenticated"))
    {
        let argument = if route.id == foks_server::rpc::RouteId::RealTimeRtPollInbox {
            foks_proto::RtPollInboxArgument {
                poll: foks_proto::RtPollInbox {
                    app: foks_proto::RtAppId::Chat,
                    since: 0,
                    timeout_milliseconds: 1,
                },
            }
            .encoded()
            .unwrap()
        } else {
            vec![0xc0]
        };
        stream
            .write_all(
                &foks_rpc::encode_call(route.protocol_id, route.position, &argument, 0).unwrap(),
            )
            .unwrap();
        let error = foks_rpc::read_response(&mut stream, 4096, 0).unwrap_err();
        assert!(
            matches!(error, foks_rpc::Error::RemoteStatus { code: 1069, .. }),
            "{}::{} bypassed SSO: {error:?}",
            route.protocol,
            route.method
        );
    }
    let foks = fixture.client.foks();
    assert!(foks
        .registration_server_config(fixture.host())
        .unwrap()
        .sso
        .is_some());
    assert!(foks
        .fetch_device_certificate_chain(
            fixture.host(),
            &created.credential.uid,
            &created.credential.seed
        )
        .is_err());
    let mut protected = fixture.client.open_protected_store().unwrap();
    let http = ProviderHttp::new(NetworkPolicy::loopback_test()).unwrap();
    assert!(foks
        .begin_sso(
            fixture.host(),
            SsoIntent {
                uid: created.credential.uid.clone(),
                device: foks_crypto::derive_device_public(&created.credential.seed)
                    .unwrap()
                    .id,
                for_login: true
            },
            &http,
            &mut protected
        )
        .is_ok());
}

#[test]
fn queued_mutation_rechecks_policy_before_changing_account_state() {
    let provider = TestOidcProvider::start();
    let environment = TestEnvironment::new().unwrap();
    let config = provider.config(environment.client_path("oidc", "secret").unwrap());
    let server = environment.start_oidc_server(config).unwrap();
    let client = TestClient::new(&environment, "oidc-account").unwrap();
    let created = signup(&environment, &client, &provider);
    let host = client.pinned_host().unwrap();
    let pressure = server.saturate_writer_queue().unwrap();
    let writer = server.writer_handle();
    let disable_writer = writer.clone();
    let disable = std::thread::spawn(move || {
        disable_writer.call(|db| {
            db.sso_disable_policy()?;
            Ok(())
        })
    });
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while writer.metrics().pending < 3 {
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
    std::thread::scope(|scope| {
        let mutation = scope.spawn(|| {
            client
                .foks()
                .clear_device_nag(&host, &created.credential, true)
        });
        let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while writer.metrics().pending < 4 {
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now();
        }
        pressure.drain().unwrap();
        disable.join().unwrap().unwrap();
        assert!(mutation.join().unwrap().is_err());
    });
    let database = rusqlite::Connection::open(environment.database_path()).unwrap();
    let cleared: bool = database
        .query_row(
            "SELECT device_nag_cleared FROM users WHERE uid=?1",
            [created.credential.uid.as_bytes()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!cleared);
}
#[test]
fn an_open_realtime_poll_stops_when_sso_expires() {
    let provider = TestOidcProvider::start();
    let environment = TestEnvironment::new().unwrap();
    let config = provider.config(environment.client_path("oidc", "secret").unwrap());
    let server = environment.start_oidc_server(config).unwrap();
    let client = TestClient::new(&environment, "oidc-account").unwrap();
    let created = signup(&environment, &client, &provider);
    let host = client.pinned_host().unwrap();
    let mut connection = client
        .foks()
        .realtime_connection(&host, &created.credential)
        .unwrap();
    let poll = std::thread::spawn(move || {
        connection.call(&foks_rpc::RealtimeRequest::PollInbox(
            foks_proto::RtPollInboxArgument {
                poll: foks_proto::RtPollInbox {
                    app: foks_proto::RtAppId::Chat,
                    since: 0,
                    timeout_milliseconds: 5000,
                },
            },
        ))
    });
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while server.metrics().active_realtime_polls == 0 {
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
    environment.advance_clock(301_000_000);
    assert!(poll.join().unwrap().is_err());
}
#[test]
fn hardware_signup_and_reauthentication_use_the_same_provider_binding() {
    use foks_yubi::{MockYubiProvider, Pin, PivPolicy, SlotId, YubiProvider as _};
    let idp = TestOidcProvider::start();
    idp.identity("ssoyubi", "yubi-subject");
    let environment = TestEnvironment::new().unwrap();
    let config = idp.config(environment.client_path("oidc", "secret").unwrap());
    let _server = environment.start_oidc_server(config).unwrap();
    let test = TestClient::new(&environment, "oidc-yubi").unwrap();
    let host = test.probe_and_pin().unwrap().pinned;
    let client = test.foks();
    let http = ProviderHttp::new(NetworkPolicy::loopback_test()).unwrap();
    let mut protected = test.open_protected_store().unwrap();
    let pin = Pin::new("654321").unwrap();
    let provider = MockYubiProvider::with_card("sso-yubi", 73001, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);
    let hardware = provider
        .prepare(
            &card,
            SlotId::new(0x82).unwrap(),
            SlotId::new(0x83).unwrap(),
            &pin,
            None,
            PivPolicy::Once,
            PivPolicy::Never,
        )
        .unwrap();
    let parent = hardware.device.as_ref();
    let puk = SecretSeed::new([49; 32]);
    let mut uid = foks_crypto::derive_shared_verify_key(&puk, ENTITY_PUK_VERIFY)
        .unwrap()
        .as_bytes()
        .to_vec();
    uid[0] = ENTITY_USER;
    let uid = EntityId::from_bytes(uid).unwrap();
    let flow = client
        .begin_sso(
            &host,
            SsoIntent {
                uid: uid.clone(),
                device: parent.entity_id().clone(),
                for_login: false,
            },
            &http,
            &mut protected,
        )
        .unwrap();
    idp.complete(flow.browser_url.as_ref().unwrap());
    client
        .poll_sso(&host, flow.operation_id, 1000, &http, &mut protected)
        .unwrap();
    let auth = client
        .authorize_sso_signup(
            &host,
            flow.operation_id,
            SsoSigningKey::Yubi(parent),
            &http,
            &mut protected,
        )
        .unwrap();
    let created = client
        .create_yubi_account_with_sso(
            &host,
            parent,
            foks_client::YubiAccountRequest {
                username_utf8: "ssoyubi".into(),
                device_name: "sso key".into(),
                invite_code: InviteCode::Empty,
                email: "ssoyubi@example.test".into(),
                passphrase: None,
                pq_hint: foks_proto::YubiSlotAndPqKeyId {
                    slot: 0x83,
                    id: hardware.locator.pq_key_id,
                },
            },
            foks_client::YubiAccountSecrets::new(SecretSeed::new([50; 32]), puk, [36; 17]),
            test.soft_state_path(),
            &auth,
            &mut protected,
        )
        .unwrap();
    let login = client
        .begin_sso(
            &host,
            SsoIntent {
                uid,
                device: parent.entity_id().clone(),
                for_login: true,
            },
            &http,
            &mut protected,
        )
        .unwrap();
    idp.complete(login.browser_url.as_ref().unwrap());
    client
        .poll_sso(&host, login.operation_id, 1000, &http, &mut protected)
        .unwrap();
    assert_eq!(
        client
            .finish_sso_login(
                &host,
                login.operation_id,
                FederationCredential::Yubi(&created.credential),
                &http,
                &mut protected
            )
            .unwrap()
            .state,
        SsoFlowState::Complete
    );
}

#[test]
fn a_provider_subject_cannot_create_two_accounts_and_failed_signup_is_atomic() {
    let provider = TestOidcProvider::start();
    let environment = TestEnvironment::new().unwrap();
    let config = provider.config(environment.client_path("oidc", "secret").unwrap());
    let _server = environment.start_oidc_server(config).unwrap();
    let first = TestClient::new(&environment, "first").unwrap();
    let original = signup(&environment, &first, &provider);
    provider.identity("otheralice", "alice-subject");
    let second = TestClient::new(&environment, "second").unwrap();
    let client = &second;
    let host = client.probe_and_pin().unwrap().pinned;
    let foks = client.foks();
    let http = ProviderHttp::new(NetworkPolicy::loopback_test()).unwrap();
    let mut protected = client.open_protected_store().unwrap();
    let device = SecretSeed::new([52; 32]);
    let puk = SecretSeed::new([53; 32]);
    let device_id = foks_crypto::derive_device_public(&device).unwrap().id;
    let mut uid = foks_crypto::derive_shared_verify_key(&puk, ENTITY_PUK_VERIFY)
        .unwrap()
        .as_bytes()
        .to_vec();
    uid[0] = ENTITY_USER;
    let uid = EntityId::from_bytes(uid).unwrap();
    let flow = foks
        .begin_sso(
            &host,
            SsoIntent {
                uid: uid.clone(),
                device: device_id,
                for_login: false,
            },
            &http,
            &mut protected,
        )
        .unwrap();
    provider.complete(flow.browser_url.as_ref().unwrap());
    assert_eq!(
        foks.poll_sso(&host, flow.operation_id, 1000, &http, &mut protected)
            .unwrap()
            .state,
        SsoFlowState::Ready
    );
    let authorization = foks
        .authorize_sso_signup(
            &host,
            flow.operation_id,
            SsoSigningKey::Software(&device),
            &http,
            &mut protected,
        )
        .unwrap();
    let result = foks.create_software_account_with_sso(
        &host,
        SoftwareAccountRequest {
            username_utf8: "otheralice".into(),
            device_name: "sso desktop".into(),
            invite_code: InviteCode::Empty,
            email: "otheralice@example.test".into(),
            passphrase: None,
        },
        SoftwareAccountSecrets::new(device, puk, [33; 17]),
        client.soft_state_path(),
        &authorization,
        &mut protected,
    );

    assert!(result.is_err());
    let db = environment.read_database().unwrap();
    assert!(db.user_authority(uid.as_bytes()).unwrap().is_none());
    assert!(db.sso_access(uid.as_bytes()).unwrap().is_none());
    assert_eq!(
        db.sso_access(original.credential.uid.as_bytes())
            .unwrap()
            .unwrap()
            .subject,
        "alice-subject"
    );
    let connection = rusqlite::Connection::open(environment.database_path()).unwrap();
    let devices: i64 = connection
        .query_row(
            "SELECT count(*) FROM devices WHERE uid=?1",
            [uid.as_bytes()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(devices, 0);
}

#[test]
fn refresh_claims_fence_races_revocation_policy_changes_and_crashes() {
    use foks_server_db::SsoAccessState;
    let provider = TestOidcProvider::start();
    let environment = TestEnvironment::new().unwrap();
    let config = provider.config(environment.client_path("oidc", "secret").unwrap());
    let server = environment.start_oidc_server(config.clone()).unwrap();
    let client = TestClient::new(&environment, "account").unwrap();
    let created = signup(&environment, &client, &provider);
    let uid = created.credential.uid.as_bytes().to_vec();
    let key = foks_crypto::derive_device_public(&created.credential.seed)
        .unwrap()
        .id
        .as_bytes()
        .to_vec();
    let writer = server.writer_handle();
    writer
        .call(move |db| {
            let active = db.sso_access(&uid)?.unwrap();
            let mut claim = active.clone();
            claim.state = SsoAccessState::Refreshing;
            claim.revision += 1;
            let now = active.expires_at_ms - 1;
            db.sso_transition_access(&active, &claim, &key, 1, now)?;
            assert!(db
                .sso_transition_access(&active, &claim, &key, 1, now)
                .is_err());
            let mut completed = claim.clone();
            completed.state = SsoAccessState::Active;
            completed.revision += 1;
            // The provider result cannot commit against a changed credential/chain snapshot.
            assert!(db
                .sso_transition_access(&claim, &completed, &key, 2, now)
                .is_err());
            assert!(db
                .sso_transition_access(&claim, &completed, &[0; 33], 1, now)
                .is_err());
            db.sso_disable_policy()?;
            assert!(db
                .sso_transition_access(&claim, &completed, &key, 1, now)
                .is_err());
            assert_eq!(
                db.sso_access(&uid)?.unwrap().state,
                SsoAccessState::Refreshing
            );
            Ok(())
        })
        .unwrap();
    server.shutdown().unwrap();
    let _reopened = environment.start_oidc_server(config).unwrap();
    let row = environment
        .read_database()
        .unwrap()
        .sso_access(created.credential.uid.as_bytes())
        .unwrap()
        .unwrap();
    assert_eq!(row.state, SsoAccessState::ReauthenticationRequired);
    assert_eq!(provider.refreshes(), 0);
    assert!(client
        .foks()
        .ping(&client.pinned_host().unwrap(), &created.credential)
        .is_err());
}

#[test]
fn bot_enrollment_after_lost_reply_stays_bound_across_sso_reauthentication() {
    use foks_client_db::{HardStateStore, MutationState};
    let provider = TestOidcProvider::start();
    let environment = TestEnvironment::new().unwrap();
    let config = provider.config(environment.client_path("oidc", "bot-secret").unwrap());
    let _server = environment.start_oidc_server(config).unwrap();
    let client = TestClient::new(&environment, "oidc-bot").unwrap();
    let created = signup(&environment, &client, &provider);
    let host = client.pinned_host().unwrap();
    let foks = client.foks();
    let mut protected = client.open_protected_store().unwrap();
    let token = foks_crypto::BotToken::generate().unwrap();
    let owner = FederationCredential::Software(&created.credential);
    let op = foks
        .prepare_bot_enrollment(
            &host,
            owner,
            foks_proto::Role::OWNER,
            &token,
            &mut protected,
        )
        .unwrap();
    environment.arm_fault(foks_server_testkit::TestFault::ProvisionAfterCommitBeforeResponse);
    assert!(foks
        .bot_enrollment_progress(&host, owner, op.operation_id, true, &mut protected)
        .is_err());
    // Mint the bot certificate before advancing the fake clock beyond TLS skew.
    // Subsequent assertions use its existing authenticated connection.
    let bot = foks.load_bot_token(&host, &token).unwrap();
    provider.set_invalid_grant(true);
    environment.advance_clock(301_000_000);
    assert!(foks
        .bot_enrollment_progress(&host, owner, op.operation_id, true, &mut protected)
        .is_err());
    assert_eq!(
        HardStateStore::open(client.hard_state_path())
            .unwrap()
            .mutation(&op.operation_id)
            .unwrap()
            .unwrap()
            .attempt_count,
        1
    );
    provider.set_invalid_grant(false);
    let http = ProviderHttp::new(NetworkPolicy::loopback_test()).unwrap();
    let flow = foks
        .begin_sso(
            &host,
            SsoIntent {
                uid: created.credential.uid.clone(),
                device: created.credential.public_material().unwrap().id,
                for_login: true,
            },
            &http,
            &mut protected,
        )
        .unwrap();
    provider.complete(flow.browser_url.as_ref().unwrap());
    foks.poll_sso(&host, flow.operation_id, 1000, &http, &mut protected)
        .unwrap();
    foks.finish_sso_login(&host, flow.operation_id, owner, &http, &mut protected)
        .unwrap();
    let op = foks
        .bot_enrollment_progress(&host, owner, op.operation_id, true, &mut protected)
        .unwrap();
    assert_eq!(op.attempt_count, 1);
    assert_eq!(op.state, MutationState::RemoteVerified);
    assert_eq!(foks.ping(&host, &bot).unwrap(), created.credential.uid);
    provider.set_invalid_grant(true);
    environment.advance_clock(301_000_000);
    assert!(foks.ping(&host, &bot).is_err());
}
