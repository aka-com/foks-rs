//! Opt-in gate against the pinned Go fleet, including both mixed-host directions.
use foks_client::{
    DeviceCredential, EncryptedFileMutationStore, FederationCredential, FoksClient,
    InvitationIntent, NamedTeamSecrets, PinnedHost, ProbeTarget, SoftwareAccountRequest,
    SoftwareAccountSecrets,
};
use foks_proto::{EntityId, InviteCode, RawInboxRequest, Role, SecretSeed};
use foks_server_testkit::TestEnvironment;
use std::path::Path;
use zeroize::Zeroizing;

struct Actor {
    client: FoksClient,
    host: PinnedHost,
    credential: DeviceCredential,
    protected: EncryptedFileMutationStore,
}
impl Actor {
    fn new(roots: rustls::RootCertStore, probe: &str, dir: &Path, name: &str, n: u8) -> Self {
        let client = FoksClient::with_roots(roots);
        let target = ProbeTarget::parse(probe).unwrap();
        let hard = dir.join(format!("{name}-hard.sqlite"));
        client.probe_and_pin(&target, &hard).unwrap();
        let host = client.pinned_host(target.hostname(), &hard).unwrap();
        let mut protected = EncryptedFileMutationStore::open(
            dir.join(format!("{name}-protected")),
            Zeroizing::new([n; 32]),
        )
        .unwrap();
        let account = client
            .create_software_account(
                &host,
                SoftwareAccountRequest {
                    username_utf8: name.into(),
                    device_name: "invitation gate".into(),
                    invite_code: InviteCode::Empty,
                    email: format!("{name}@example.test"),
                    passphrase: None,
                },
                SoftwareAccountSecrets::new(
                    SecretSeed::new([n; 32]),
                    SecretSeed::new([n + 1; 32]),
                    [n + 2; 17],
                ),
                &dir.join(format!("{name}-soft.sqlite")),
                &mut protected,
            )
            .unwrap();
        Self {
            client,
            host,
            credential: account.credential,
            protected,
        }
    }
    fn team(&self, name: &str, n: u8) -> EntityId {
        self.client
            .create_single_owner_named_team(
                &self.host,
                &self.credential,
                name,
                &NamedTeamSecrets {
                    member_min: SecretSeed::new([n; 32]),
                    member: SecretSeed::new([n + 1; 32]),
                    admin: SecretSeed::new([n + 2; 32]),
                    owner: SecretSeed::new([n + 3; 32]),
                    removal_key: SecretSeed::new([n + 4; 32]),
                    team_name_commitment_key: [n + 5; 16],
                },
            )
            .unwrap()
            .team
    }
}
fn local(owner: &mut Actor, joiner: &Actor, team: &EntityId) {
    let a = FederationCredential::Software(&owner.credential);
    let j = FederationCredential::Software(&joiner.credential);
    let cert = owner
        .client
        .prepare_team_invitation(&owner.host, a, team)
        .unwrap();
    owner
        .client
        .upload_team_invitation(&owner.host, a, &cert)
        .unwrap();
    let prepared = joiner
        .client
        .prepare_local_invitation_acceptance(&joiner.host, j, &cert.invite)
        .unwrap();
    let rsvp = joiner
        .client
        .submit_local_invitation_acceptance(&joiner.host, j, &prepared)
        .unwrap();
    assert!(!rsvp.is_remote());
    assert!(joiner
        .client
        .submit_local_invitation_acceptance(&joiner.host, j, &prepared)
        .is_err());
    let rows = owner
        .client
        .team_invitation_inbox(&owner.host, a, team, None)
        .unwrap();
    assert_eq!(rows.len(), 1);
    let target = owner
        .client
        .load_local_invitation_joiner(&owner.host, a, team, &rows[0])
        .unwrap();
    let user = owner
        .client
        .authenticate_credential_and_pin(&owner.host, a)
        .unwrap();
    let loaded = owner
        .client
        .load_and_pin_team_with_credential(&owner.host, a, &user.verified, &user.puks, team)
        .unwrap();
    let removal = SecretSeed::new([201; 32]);
    let request = foks_client::AddLocalTeamMemberRequest {
        target_user: &target,
        destination_role: Role::member(0),
        removal_key: &removal,
    };
    let plan = owner
        .client
        .local_team_member_addition_plan(a.uid(), team, &loaded, &request)
        .unwrap();
    owner
        .client
        .add_invited_local_user_durable(&owner.host, a, team, &plan, &request, &mut owner.protected)
        .unwrap();
    assert!(owner
        .client
        .team_invitation_inbox(&owner.host, a, team, None)
        .unwrap()
        .is_empty());
    let user = joiner
        .client
        .authenticate_credential_and_pin(&joiner.host, j)
        .unwrap();
    let loaded = joiner
        .client
        .load_and_pin_team_with_credential(&joiner.host, j, &user.verified, &user.puks, team)
        .unwrap();
    assert_eq!(loaded.ptks.len(), 2);
    eprintln!("Go local invitation, approval and PTK load passed");
}
fn remote(home: &mut Actor, dest: &mut Actor, team: &EntityId, source: Option<&EntityId>) {
    let h = FederationCredential::Software(&home.credential);
    let d = FederationCredential::Software(&dest.credential);
    let cert = dest
        .client
        .prepare_team_invitation(&dest.host, d, team)
        .unwrap();
    dest.client
        .upload_team_invitation(&dest.host, d, &cert)
        .unwrap();
    let prepared = match source {
        Some(source) => home.client.prepare_remote_team_invitation(
            &home.host,
            &dest.host,
            h,
            source,
            Role::ADMIN,
            &cert.invite,
        ),
        None => home
            .client
            .prepare_remote_user_invitation(&home.host, &dest.host, h, &cert.invite),
    }
    .unwrap();
    let operation = home
        .client
        .prepare_invitation_operation(
            &home.host,
            h,
            InvitationIntent::RemoteAcceptance(Box::new(prepared)),
            &mut home.protected,
        )
        .unwrap()
        .operation;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let rsvp = loop {
        let progress = home
            .client
            .remote_invitation_progress(
                &home.host,
                &dest.host,
                h,
                operation.operation_id,
                true,
                &mut home.protected,
            )
            .unwrap();
        if let Some(rsvp) = progress.rsvp {
            break rsvp;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "home membership proof not published"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let rows = dest
        .client
        .team_invitation_inbox(&dest.host, d, team, None)
        .unwrap();
    let row = rows.iter().find(|r| r.rsvp == rsvp).unwrap();
    let RawInboxRequest::Remote(request) = &row.request else {
        panic!("remote request")
    };
    let payload = dest
        .client
        .open_remote_invitation(&dest.host, d, team, request)
        .unwrap();
    let user = dest
        .client
        .authenticate_credential_and_pin(&dest.host, d)
        .unwrap();
    let loaded = dest
        .client
        .load_and_pin_team_with_credential(&dest.host, d, &user.verified, &user.puks, team)
        .unwrap();
    let removal = SecretSeed::new([if source.is_some() { 204 } else { 202 }; 32]);
    if let Some(source) = source {
        let expanded = dest
            .client
            .verify_remote_invitation_team(&home.host, &payload)
            .unwrap();
        let plan = dest
            .client
            .invited_team_addition_plan(
                d.uid(),
                &loaded,
                &expanded.verified,
                Role::ADMIN,
                Role::member(0),
                &removal,
            )
            .unwrap();
        dest.client
            .add_invited_team_durable(
                &dest.host,
                d,
                team,
                &expanded.verified,
                Some(&payload.permission),
                &rsvp,
                &plan,
                &removal,
                &mut dest.protected,
            )
            .unwrap();
        let member = home
            .client
            .load_invited_remote_team_as_team(&home.host, &dest.host, h, source, Role::ADMIN, team)
            .unwrap();
        assert_eq!(member.ptks.len(), 2);
    } else {
        let expanded = dest
            .client
            .verify_remote_invitation_user(&home.host, payload)
            .unwrap();
        let plan = dest
            .client
            .invited_remote_user_addition_plan(
                d.uid(),
                team,
                &loaded,
                &expanded,
                Role::member(0),
                &removal,
            )
            .unwrap();
        dest.client
            .add_invited_remote_user_durable(
                &dest.host,
                d,
                team,
                &expanded,
                &rsvp,
                &plan,
                &removal,
                &mut dest.protected,
            )
            .unwrap();
        let member = home
            .client
            .load_invited_remote_team(&home.host, &dest.host, h, team)
            .unwrap();
        assert_eq!(member.ptks.len(), 2);
    }
    assert!(dest
        .client
        .team_invitation_inbox(&dest.host, d, team, None)
        .unwrap()
        .is_empty());
    eprintln!(
        "mixed-host invitation, approval and PTK load passed (team actor: {})",
        source.is_some()
    );
}
#[test]
fn invitations_against_go_and_mixed_hosts() {
    let Ok(probe) = std::env::var("FOKS_TEST_INVITATION_PROBE") else {
        return;
    };
    let ca = std::fs::read(std::env::var("FOKS_TEST_INVITATION_CA").unwrap()).unwrap();
    let dir = std::path::PathBuf::from(std::env::var("FOKS_TEST_INVITATION_STATE").unwrap());
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(ca))
        .unwrap();
    let mut go = Actor::new(roots.clone(), &probe, &dir, "invitegoadmin", 21);
    let guest = Actor::new(roots, &probe, &dir, "invitegoguest", 31);
    let local_team = go.team("invitegolocal", 41);
    local(&mut go, &guest, &local_team);
    let env = TestEnvironment::new().unwrap();
    env.set_invite_regime(foks_server_db::InviteRegime::Optional)
        .unwrap();
    let server = env.start_server().unwrap();
    let rust_probe = format!("localhost:{}", server.addresses().probe.port());
    let mut rust = Actor::new(env.probe_roots(), &rust_probe, &dir, "inviterustadmin", 51);
    let go_target = go.team("invitegoparent", 61);
    let rust_target = rust.team("inviterustparent", 71);
    remote(&mut rust, &mut go, &go_target, None);
    remote(&mut go, &mut rust, &rust_target, None);
    let go_source = go.team("invitegochild", 81);
    let rust_source = rust.team("inviterustchild", 91);
    go.client
        .lower_team_index_range_durable(&go.host, &go.credential, &go_source, &mut go.protected)
        .unwrap();
    rust.client
        .lower_team_index_range_durable(
            &rust.host,
            &rust.credential,
            &rust_source,
            &mut rust.protected,
        )
        .unwrap();
    go.client
        .raise_team_index_range_durable(&go.host, &go.credential, &go_target, &mut go.protected)
        .unwrap();
    rust.client
        .raise_team_index_range_durable(
            &rust.host,
            &rust.credential,
            &rust_target,
            &mut rust.protected,
        )
        .unwrap();
    remote(&mut rust, &mut go, &go_target, Some(&rust_source));
    remote(&mut go, &mut rust, &rust_target, Some(&go_source));
}

#[test]
fn official_go_invitation_client_against_rust() {
    let Some(oracle) = std::env::var_os("FOKS_GO_ORACLE_DIR") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let env = TestEnvironment::new().unwrap();
    env.set_invite_regime(foks_server_db::InviteRegime::Optional)
        .unwrap();
    let server = env.start_server().unwrap();
    let probe = format!("localhost:{}", server.addresses().probe.port());
    let mut owner = Actor::new(
        env.probe_roots(),
        &probe,
        dir.path(),
        "gosdkinviteadmin",
        151,
    );
    let joiner = Actor::new(
        env.probe_roots(),
        &probe,
        dir.path(),
        "gosdkinvitejoiner",
        161,
    );
    let team = owner.team("gosdkinvitation", 171);
    let a = FederationCredential::Software(&owner.credential);
    let j = FederationCredential::Software(&joiner.credential);
    let cert = owner
        .client
        .prepare_team_invitation(&owner.host, a, &team)
        .unwrap();
    owner
        .client
        .upload_team_invitation(&owner.host, a, &cert)
        .unwrap();
    let write = |n: &str, b: &[u8]| std::fs::write(dir.path().join(n), b).unwrap();
    write("invite.snowp", &cert.invite.encoded().unwrap());
    write("uid.raw", joiner.credential.uid.as_bytes());
    write("seed.raw", joiner.credential.seed.as_slice());
    write(
        "key.der",
        &foks_crypto::device_signing_key_pkcs8(&joiner.credential.seed).unwrap(),
    );
    for (i, cert) in joiner.credential.certificate_chain.iter().enumerate() {
        write(&format!("certificate-{i}.der"), cert);
    }
    write("ca.der", &joiner.host.tls_ca_certificates()[0]);
    let user = joiner
        .client
        .authenticate_credential_and_pin(&joiner.host, j)
        .unwrap();
    let puk = user
        .puks
        .iter()
        .find(|k| k.role == Role::OWNER && k.generation == 1)
        .unwrap();
    write("puk.raw", puk.seed.as_slice());
    let run = |phase: &str| {
        let output = std::process::Command::new("go")
            .args(["test", "-C"])
            .arg(&oracle)
            .args([
                "-mod=readonly",
                "-run",
                "^TestGoInvitationsAgainstRustServer$",
                "-count=1",
                "-timeout=40s",
                "-v",
            ])
            .env("FOKS_INVITATION_LIVE_DIR", dir.path())
            .env("FOKS_INVITATION_LIVE_PHASE", phase)
            .env(
                "FOKS_INVITATION_LIVE_AUTH",
                server.addresses().authenticated.to_string(),
            )
            .env(
                "FOKS_INVITATION_LIVE_PUBLIC",
                server.addresses().public_services.to_string(),
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Go invitation client failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run("request");
    let rows = owner
        .client
        .team_invitation_inbox(&owner.host, a, &team, None)
        .unwrap();
    assert_eq!(rows.len(), 1);
    let target = owner
        .client
        .load_local_invitation_joiner(&owner.host, a, &team, &rows[0])
        .unwrap();
    let user = owner
        .client
        .authenticate_credential_and_pin(&owner.host, a)
        .unwrap();
    let loaded = owner
        .client
        .load_and_pin_team_with_credential(&owner.host, a, &user.verified, &user.puks, &team)
        .unwrap();
    let removal = SecretSeed::new([203; 32]);
    let request = foks_client::AddLocalTeamMemberRequest {
        target_user: &target,
        destination_role: Role::member(0),
        removal_key: &removal,
    };
    let plan = owner
        .client
        .local_team_member_addition_plan(a.uid(), &team, &loaded, &request)
        .unwrap();
    owner
        .client
        .add_invited_local_user_durable(
            &owner.host,
            a,
            &team,
            &plan,
            &request,
            &mut owner.protected,
        )
        .unwrap();
    run("member");
    assert!(owner
        .client
        .team_invitation_inbox(&owner.host, a, &team, None)
        .unwrap()
        .is_empty());
}
