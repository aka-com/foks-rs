use crate::{authorization::authenticated_stream, support::Fixture};
use foks_client::{FederationCredential, NamedTeamSecrets};
use foks_proto::{Role, SecretSeed};
use foks_server_testkit::{TestAccountSpec, TestClient};
use std::io::Write as _;

#[test]
pub(crate) fn team_invitation_certificates() {
    let f = Fixture::start("invitation-certs");
    let owner = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("inviteowner", 11))
        .unwrap();
    let guest_client = TestClient::new(&f.environment, "invitation-guest").unwrap();
    let guest = guest_client
        .create_account(f.host(), &TestAccountSpec::new("inviteguest", 12))
        .unwrap();
    let cred = FederationCredential::Software(&owner.credential);
    let secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([41; 32]),
        member: SecretSeed::new([42; 32]),
        admin: SecretSeed::new([43; 32]),
        owner: SecretSeed::new([44; 32]),
        removal_key: SecretSeed::new([45; 32]),
        team_name_commitment_key: [46; 16],
    };
    let created = f
        .client
        .foks()
        .create_single_owner_named_team(f.host(), &owner.credential, "invitationteam", &secrets)
        .unwrap();
    let team = created.authenticated.verified.team();
    assert!(f
        .client
        .foks()
        .current_team_invitations(f.host(), cred, team)
        .unwrap()
        .is_empty());
    let prepared = f
        .client
        .foks()
        .prepare_team_invitation(f.host(), cred, team)
        .unwrap();
    assert!(f
        .client
        .foks()
        .preview_team_invitation(f.host(), &prepared.invite)
        .is_err());
    let preview = f
        .client
        .foks()
        .upload_team_invitation(f.host(), cred, &prepared)
        .unwrap();
    assert_eq!(&preview.advertised.team.team, team);
    assert_eq!(
        preview.advertised.name.as_deref(),
        Some(b"invitationteam".as_slice())
    );
    // Public lookup carries no authenticated user or team membership.
    let public = guest_client
        .foks()
        .preview_team_invitation(f.host(), &prepared.invite)
        .unwrap();
    assert_eq!(public.certificate, prepared.certificate);
    assert_eq!(
        f.client
            .foks()
            .current_team_invitations(f.host(), cred, team)
            .unwrap()
            .len(),
        1
    );
    // Immutable hash re-upload does not consume quota or create another record.
    f.client
        .foks()
        .upload_team_invitation(f.host(), cred, &prepared)
        .unwrap();
    assert_eq!(
        f.client
            .foks()
            .current_team_invitations(f.host(), cred, team)
            .unwrap()
            .len(),
        1
    );
    assert!(guest_client
        .foks()
        .prepare_team_invitation(
            f.host(),
            FederationCredential::Software(&guest.credential),
            team
        )
        .is_err());
    let mut forged = prepared.invite.clone();
    forged.hash[0] ^= 1;
    assert!(f
        .client
        .foks()
        .preview_team_invitation(f.host(), &forged)
        .is_err());
    let load = || {
        let mut tls = authenticated_stream(&f, &guest.credential);
        tls.write_all(
            &foks_rpc::encode_load_user_chain_request(owner.credential.uid.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        foks_rpc::read_response(&mut tls, 16 * 1024 * 1024, 0)
    };
    assert!(matches!(
        load(),
        Err(foks_rpc::Error::RemoteStatus { code: 1013, .. })
    ));
    f.client
        .foks()
        .grant_local_user_view(f.host(), cred, &guest.credential.uid, Role::OWNER)
        .unwrap();
    assert!(load().is_ok());
    f.client
        .foks()
        .grant_local_user_view(f.host(), cred, team, Role::ADMIN)
        .unwrap();
    // Team-as-viewee grant is a signed current PTK scope, separate from user identity.
    f.client
        .foks()
        .grant_local_team_view(f.host(), cred, team, Role::ADMIN, team, Role::ADMIN)
        .unwrap();
    // Fill only the isolated test database's certificate capacity. Existing
    // exact lookup/re-upload remains available when new uploads are refused.
    let db = rusqlite::Connection::open(f.environment.database_path()).unwrap();
    for n in 0..63u8 {
        let mut hash = [n; 32];
        hash[31] = 99;
        db.execute("INSERT INTO team_invitation_certificates SELECT ?1,team_id,generation,exact_certificate,created_at FROM team_invitation_certificates WHERE certificate_hash=?2",rusqlite::params![hash,prepared.invite.hash]).unwrap();
    }
    drop(db);
    let next = foks_crypto::make_team_certificate(
        preview.advertised.team.clone(),
        &secrets.admin,
        &secrets.admin,
        1,
        b"Another signed label".to_vec(),
        preview.advertised.time,
    )
    .unwrap();
    let next = foks_client::PreparedTeamInvitation {
        invite: foks_crypto::team_certificate_invite(&next).unwrap(),
        certificate: next,
    };
    assert!(matches!(
        f.client
            .foks()
            .upload_team_invitation(f.host(), cred, &next),
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 1060,
            ..
        }))
    ));
    f.client
        .foks()
        .upload_team_invitation(f.host(), cred, &prepared)
        .unwrap();
    let malicious = foks_proto::LocalViewPermissionPayload {
        viewee: owner.credential.uid.clone(),
        viewer: guest.credential.uid.clone(),
        time: 1,
        viewer_role: Some(Role::OWNER),
    };
    let mut tls = authenticated_stream(&f, &guest.credential);
    tls.write_all(&foks_rpc::encode_grant_local_user_view_request(&malicious).unwrap())
        .unwrap();
    assert!(matches!(
        foks_rpc::read_response(&mut tls, 4096, 0),
        Err(foks_rpc::Error::RemoteStatus { code: 1013, .. })
    ));
}

#[test]
fn old_invitation_survives_admin_rotation_and_current_listing_changes() {
    let f = Fixture::start("invitation-rotation");
    let owner = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("rotateowner", 21))
        .unwrap();
    let other_client = TestClient::new(&f.environment, "rotateother").unwrap();
    let other = other_client
        .create_account(f.host(), &TestAccountSpec::new("rotateother", 22))
        .unwrap();
    let cred = FederationCredential::Software(&owner.credential);
    let secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([51; 32]),
        member: SecretSeed::new([52; 32]),
        admin: SecretSeed::new([53; 32]),
        owner: SecretSeed::new([54; 32]),
        removal_key: SecretSeed::new([55; 32]),
        team_name_commitment_key: [56; 16],
    };
    let created = f
        .client
        .foks()
        .create_single_owner_named_team(f.host(), &owner.credential, "rotationteam", &secrets)
        .unwrap();
    let first = f
        .client
        .foks()
        .prepare_team_invitation(f.host(), cred, &created.team)
        .unwrap();
    f.client
        .foks()
        .upload_team_invitation(f.host(), cred, &first)
        .unwrap();
    f.client
        .foks()
        .add_local_user_to_named_team(
            f.host(),
            &owner.credential,
            &created.team,
            &foks_client::AddLocalTeamMemberRequest {
                target_user: &other.authenticated.verified,
                destination_role: Role::ADMIN,
                removal_key: &SecretSeed::new([57; 32]),
            },
        )
        .unwrap();
    let min = SecretSeed::new([61; 32]);
    let member = SecretSeed::new([62; 32]);
    let admin = SecretSeed::new([63; 32]);
    let rotations = [
        foks_client::TeamPtkRotationSeed {
            role: Role::member(-0x4000),
            seed: &min,
        },
        foks_client::TeamPtkRotationSeed {
            role: Role::member(0),
            seed: &member,
        },
        foks_client::TeamPtkRotationSeed {
            role: Role::ADMIN,
            seed: &admin,
        },
    ];
    let mut protected = f.client.open_protected_store().unwrap();
    f.client
        .foks()
        .remove_team_member_and_rotate_ptks(
            f.host(),
            &owner.credential,
            &created.team,
            &foks_client::RemoveTeamMemberRequest {
                target: other.authenticated.verified.uid(),
                rotations: &rotations,
                remaining_users: &[],
            },
            &mut protected,
        )
        .unwrap();
    assert!(f
        .client
        .foks()
        .current_team_invitations(f.host(), cred, &created.team)
        .unwrap()
        .is_empty());
    assert_eq!(
        f.client
            .foks()
            .preview_team_invitation(f.host(), &first.invite)
            .unwrap()
            .advertised
            .key
            .generation,
        1
    );
    let second = f
        .client
        .foks()
        .prepare_team_invitation(f.host(), cred, &created.team)
        .unwrap();
    assert_eq!(second.certificate.signatures.len(), 2);
    f.client
        .foks()
        .upload_team_invitation(f.host(), cred, &second)
        .unwrap();
    assert_eq!(
        f.client
            .foks()
            .current_team_invitations(f.host(), cred, &created.team)
            .unwrap()
            .len(),
        1
    );
    assert!(other_client
        .foks()
        .current_team_invitations(
            f.host(),
            FederationCredential::Software(&other.credential),
            &created.team
        )
        .is_err());
}

#[test]
pub(crate) fn team_local_invitations() {
    let f = Fixture::start("local-invitations");
    let owner = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("localinviteowner", 71))
        .unwrap();
    let jc = TestClient::new(&f.environment, "local-invitation-joiner").unwrap();
    let joiner = jc
        .create_account(f.host(), &TestAccountSpec::new("localinvitejoiner", 72))
        .unwrap();
    let owner_cred = FederationCredential::Software(&owner.credential);
    let joiner_cred = FederationCredential::Software(&joiner.credential);
    let secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([81; 32]),
        member: SecretSeed::new([82; 32]),
        admin: SecretSeed::new([83; 32]),
        owner: SecretSeed::new([84; 32]),
        removal_key: SecretSeed::new([85; 32]),
        team_name_commitment_key: [86; 16],
    };
    let created = f
        .client
        .foks()
        .create_single_owner_named_team(f.host(), &owner.credential, "localinviteteam", &secrets)
        .unwrap();
    let invitation = f
        .client
        .foks()
        .prepare_team_invitation(f.host(), owner_cred, &created.team)
        .unwrap();
    f.client
        .foks()
        .upload_team_invitation(f.host(), owner_cred, &invitation)
        .unwrap();
    assert!(f
        .client
        .foks()
        .prepare_local_invitation_acceptance(f.host(), owner_cred, &invitation.invite)
        .is_err());
    let prepared = jc
        .foks()
        .prepare_local_invitation_acceptance(f.host(), joiner_cred, &invitation.invite)
        .unwrap();
    let receipt = jc
        .foks()
        .submit_local_invitation_acceptance(f.host(), joiner_cred, &prepared)
        .unwrap();
    assert!(!receipt.is_remote());
    assert!(matches!(
        jc.foks()
            .submit_local_invitation_acceptance(f.host(), joiner_cred, &prepared),
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 7015,
            ..
        }))
    ));
    let requested_user = jc
        .foks()
        .authenticate_credential_and_pin(f.host(), joiner_cred)
        .unwrap();
    let memberships = jc
        .foks()
        .authenticated_user_team_memberships(f.host(), &joiner.credential, &requested_user.verified)
        .unwrap();
    assert!(memberships
        .current
        .iter()
        .any(|e| e.membership.team == created.team
            && e.membership.state == foks_proto::TeamMembershipState::Requested));
    let inbox = f
        .client
        .foks()
        .team_invitation_inbox(f.host(), owner_cred, &created.team, None)
        .unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].receipt, receipt);
    assert!(jc
        .foks()
        .team_invitation_inbox(f.host(), joiner_cred, &created.team, None)
        .is_err());
    let target = f
        .client
        .foks()
        .load_local_invitation_joiner(f.host(), owner_cred, &created.team, &inbox[0])
        .unwrap();
    assert_eq!(target.uid(), &joiner.credential.uid);
    // Rejection is repeatable and a later fresh request may rejoin.
    f.client
        .foks()
        .reject_team_invitation(f.host(), owner_cred, &created.team, &receipt)
        .unwrap();
    f.client
        .foks()
        .reject_team_invitation(f.host(), owner_cred, &created.team, &receipt)
        .unwrap();
    assert!(f
        .client
        .foks()
        .team_invitation_inbox(f.host(), owner_cred, &created.team, None)
        .unwrap()
        .is_empty());
    let again = jc
        .foks()
        .prepare_local_invitation_acceptance(f.host(), joiner_cred, &invitation.invite)
        .unwrap();
    let receipt2 = jc
        .foks()
        .submit_local_invitation_acceptance(f.host(), joiner_cred, &again)
        .unwrap();
    assert_ne!(receipt, receipt2);
    let inbox = f
        .client
        .foks()
        .team_invitation_inbox(f.host(), owner_cred, &created.team, None)
        .unwrap();
    let target = f
        .client
        .foks()
        .load_local_invitation_joiner(f.host(), owner_cred, &created.team, &inbox[0])
        .unwrap();
    let added = f
        .client
        .foks()
        .add_local_user_to_named_team(
            f.host(),
            &owner.credential,
            &created.team,
            &foks_client::AddLocalTeamMemberRequest {
                target_user: &target,
                destination_role: Role::member(0),
                removal_key: &SecretSeed::new([87; 32]),
            },
        )
        .unwrap();
    assert_eq!(added.authenticated.verified.members().len(), 2);
    assert!(f
        .client
        .foks()
        .team_invitation_inbox(f.host(), owner_cred, &created.team, None)
        .unwrap()
        .is_empty());
    assert!(matches!(
        f.client
            .foks()
            .reject_team_invitation(f.host(), owner_cred, &created.team, &receipt2),
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 7002,
            ..
        }))
    ));
    let user = jc
        .foks()
        .authenticate_credential_and_pin(f.host(), joiner_cred)
        .unwrap();
    let member = jc
        .foks()
        .load_and_pin_team_with_credential(
            f.host(),
            joiner_cred,
            &user.verified,
            &user.puks,
            &created.team,
        )
        .unwrap();
    assert_eq!(member.ptks.len(), 2);
    assert!(jc
        .foks()
        .prepare_local_invitation_acceptance(f.host(), joiner_cred, &invitation.invite)
        .is_err());
}

#[test]
pub(crate) fn team_remote_invitations() {
    let home = Fixture::start("invitation-home");
    let destination = Fixture::start("invitation-destination");
    let joiner = home
        .client
        .create_account(home.host(), &TestAccountSpec::new("remoteinvitejoiner", 21))
        .unwrap();
    let owner = destination
        .client
        .create_account(
            destination.host(),
            &TestAccountSpec::new("remoteinviteowner", 22),
        )
        .unwrap();
    let c = FederationCredential::Software(&joiner.credential);
    let admin = FederationCredential::Software(&owner.credential);
    let secrets = NamedTeamSecrets {
        member_min: SecretSeed::new([61; 32]),
        member: SecretSeed::new([62; 32]),
        admin: SecretSeed::new([63; 32]),
        owner: SecretSeed::new([64; 32]),
        removal_key: SecretSeed::new([65; 32]),
        team_name_commitment_key: [66; 16],
    };
    let team = destination
        .client
        .foks()
        .create_single_owner_named_team(
            destination.host(),
            &owner.credential,
            "remoteinvites",
            &secrets,
        )
        .unwrap()
        .team;
    let cert = destination
        .client
        .foks()
        .prepare_team_invitation(destination.host(), admin, &team)
        .unwrap();
    destination
        .client
        .foks()
        .upload_team_invitation(destination.host(), admin, &cert)
        .unwrap();
    let prepared = home
        .client
        .foks()
        .prepare_remote_user_invitation(home.host(), destination.host(), c, &cert.invite)
        .unwrap();
    let duplicate = prepared.clone();
    let mut protected = home.client.open_protected_store().unwrap();
    let op = home
        .client
        .foks()
        .prepare_invitation_operation(
            home.host(),
            c,
            foks_client::InvitationIntent::RemoteAcceptance(Box::new(prepared)),
            &mut protected,
        )
        .unwrap()
        .operation;
    home.environment
        .arm_fault(foks_server_testkit::TestFault::InvitationHomeUserAfterCommitBeforeResponse);
    let progress = home
        .client
        .foks()
        .remote_invitation_progress(
            home.host(),
            destination.host(),
            c,
            op.operation_id,
            true,
            &mut protected,
        )
        .unwrap();
    assert_eq!(
        progress.operation.state,
        foks_client_db::MutationState::RemoteVerified
    );
    let receipt = progress.receipt.unwrap();
    let request = destination
        .client
        .foks()
        .load_remote_invitation_request(destination.host(), admin, &team, &receipt)
        .unwrap();
    let payload = destination
        .client
        .foks()
        .open_remote_invitation(destination.host(), admin, &team, &request)
        .unwrap();
    let expanded = destination
        .client
        .foks()
        .verify_remote_invitation_user(home.host(), payload)
        .unwrap();
    assert_eq!(expanded.user.verified.uid(), &joiner.credential.uid);
    let second = home
        .client
        .foks()
        .submit_remote_invitation(destination.host(), &duplicate)
        .unwrap();
    assert_ne!(
        second, receipt,
        "Go returns fresh RSVP even for exact ciphertext"
    );
    assert_eq!(
        destination
            .client
            .foks()
            .team_invitation_inbox(destination.host(), admin, &team, None)
            .unwrap()
            .len(),
        2
    );
    destination
        .client
        .foks()
        .reject_team_invitation(destination.host(), admin, &team, &receipt)
        .unwrap();
    assert!(destination
        .client
        .foks()
        .load_remote_invitation_request(destination.host(), admin, &team, &receipt)
        .is_err());
    destination
        .client
        .foks()
        .reject_team_invitation(destination.host(), admin, &team, &second)
        .unwrap();
    let next = home
        .client
        .foks()
        .prepare_remote_user_invitation(home.host(), destination.host(), c, &cert.invite)
        .unwrap();
    let op = home
        .client
        .foks()
        .prepare_invitation_operation(
            home.host(),
            c,
            foks_client::InvitationIntent::RemoteAcceptance(Box::new(next)),
            &mut protected,
        )
        .unwrap()
        .operation;
    destination
        .environment
        .arm_fault(foks_server_testkit::TestFault::RemoteInvitationAfterCommitBeforeResponse);
    assert!(home
        .client
        .foks()
        .remote_invitation_progress(
            home.host(),
            destination.host(),
            c,
            op.operation_id,
            true,
            &mut protected
        )
        .is_err());
    drop(protected);
    let mut protected = home.client.open_protected_store().unwrap();
    for _ in 0..2 {
        let progress = home
            .client
            .foks()
            .remote_invitation_progress(
                home.host(),
                destination.host(),
                c,
                op.operation_id,
                true,
                &mut protected,
            )
            .unwrap();
        assert_eq!(
            progress.operation.state,
            foks_client_db::MutationState::SubmissionUnknown
        );
    }
    assert_eq!(
        destination
            .client
            .foks()
            .team_invitation_inbox(destination.host(), admin, &team, None)
            .unwrap()
            .len(),
        1
    );
    let row = destination
        .client
        .foks()
        .team_invitation_inbox(destination.host(), admin, &team, None)
        .unwrap()
        .remove(0);
    let foks_proto::RawInboxRequest::Remote(request) = &row.request else {
        panic!("remote row")
    };
    let payload = destination
        .client
        .foks()
        .open_remote_invitation(destination.host(), admin, &team, request)
        .unwrap();
    let expanded = destination
        .client
        .foks()
        .verify_remote_invitation_user(home.host(), payload)
        .unwrap();
    let user = destination
        .client
        .foks()
        .authenticate_and_pin(destination.host(), &owner.credential)
        .unwrap();
    let loaded = destination
        .client
        .foks()
        .load_and_pin_team_with_credential(
            destination.host(),
            admin,
            &user.verified,
            &user.puks,
            &team,
        )
        .unwrap();
    let removal = SecretSeed::new([85; 32]);
    let plan = destination
        .client
        .foks()
        .invited_remote_user_addition_plan(
            &owner.credential.uid,
            &team,
            &loaded,
            &expanded,
            Role::member(0),
            &removal,
        )
        .unwrap();
    let mut destination_protected = destination.client.open_protected_store().unwrap();
    let added = destination
        .client
        .foks()
        .add_invited_remote_user_durable(
            destination.host(),
            admin,
            &team,
            &expanded,
            &row.receipt,
            &plan,
            &removal,
            &mut destination_protected,
        )
        .unwrap();
    assert_eq!(added.authenticated.verified.members().len(), 2);
    assert!(destination
        .client
        .foks()
        .team_invitation_inbox(destination.host(), admin, &team, None)
        .unwrap()
        .is_empty());
    assert!(destination
        .client
        .foks()
        .reject_team_invitation(destination.host(), admin, &team, &row.receipt)
        .is_err());
    let membership = home
        .client
        .foks()
        .load_invited_remote_team(home.host(), destination.host(), c, &team)
        .unwrap();
    assert_eq!(membership.ptks.len(), 2);
    assert!(membership
        .verified
        .members()
        .iter()
        .any(|m| m.party == joiner.credential.uid
            && m.scoped_host.as_ref() == Some(home.host().host_id())));
    // A home PUK rotation can precede the destination's team member-key refresh. Membership
    // remains readable through the authenticated historical recipient key.
    let user = home
        .client
        .foks()
        .authenticate_credential_and_pin(home.host(), c)
        .unwrap();
    let old = user
        .puks
        .iter()
        .find(|key| key.role == Role::OWNER && key.generation == 1)
        .unwrap();
    home.client
        .foks()
        .rotate_software_puks(
            home.host(),
            &joiner.credential,
            &[foks_client::UserPukRotation {
                role: Role::OWNER,
                previous_generation: 1,
                previous_seed: SecretSeed::from_slice(old.seed.as_slice()).unwrap(),
                new_seed: SecretSeed::new([229; 32]),
            }],
            Some(foks_client::NoPassphraseConfigured),
            &mut protected,
        )
        .unwrap();
    let after_rotation = home
        .client
        .foks()
        .load_invited_remote_team(home.host(), destination.host(), c, &team)
        .unwrap();
    assert_eq!(after_rotation.ptks.len(), 2);
}

#[test]
fn team_joiners_keep_source_role_and_admin_signer_separate() {
    let f = Fixture::start("invitation-team-joiner");
    let owner = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("teamjoinerowner", 81))
        .unwrap();
    let cred = FederationCredential::Software(&owner.credential);
    let create = |name: &str, n: u8| {
        f.client
            .foks()
            .create_single_owner_named_team(
                f.host(),
                &owner.credential,
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
    };
    let a = create("joiningsource", 90);
    let b = create("joiningdestination", 100);
    let (source, destination) = (a, b);
    let mut ranges = f.client.open_protected_store().unwrap();
    f.client
        .foks()
        .lower_team_index_range_durable(f.host(), &owner.credential, &source.team, &mut ranges)
        .unwrap();
    f.client
        .foks()
        .raise_team_index_range_durable(f.host(), &owner.credential, &destination.team, &mut ranges)
        .unwrap();
    let invite = f
        .client
        .foks()
        .prepare_team_invitation(f.host(), cred, &destination.team)
        .unwrap();
    f.client
        .foks()
        .upload_team_invitation(f.host(), cred, &invite)
        .unwrap();
    for (source_role, role, n) in [
        (Role::member(0), Role::member(0), 120),
        (Role::ADMIN, Role::ADMIN, 121),
        (Role::OWNER, Role::OWNER, 122),
    ] {
        let prepared = f
            .client
            .foks()
            .prepare_local_team_invitation(
                f.host(),
                cred,
                &source.team,
                source_role,
                &invite.invite,
            )
            .unwrap();
        let decoded = prepared
            .membership_link
            .as_ref()
            .unwrap()
            .link
            .decode_generic()
            .unwrap();
        assert_eq!(decoded.entity, source.team);
        assert_eq!(
            decoded.signer,
            source
                .authenticated
                .verified
                .shared_key(Role::ADMIN)
                .unwrap()
                .verify_key
        );
        let mut protected = f.client.open_protected_store().unwrap();
        let op = f
            .client
            .foks()
            .prepare_invitation_operation(
                f.host(),
                cred,
                foks_client::InvitationIntent::LocalAcceptance(prepared),
                &mut protected,
            )
            .unwrap();
        let delivered = f
            .client
            .foks()
            .invitation_progress(
                f.host(),
                cred,
                op.operation.operation_id,
                true,
                &mut protected,
            )
            .unwrap();
        let rows = f
            .client
            .foks()
            .team_invitation_inbox(f.host(), cred, &destination.team, None)
            .unwrap();
        assert_eq!(rows.len(), 1);
        let verified = f
            .client
            .foks()
            .load_local_invitation_team(f.host(), cred, &destination.team, &rows[0])
            .unwrap();
        let user = f
            .client
            .foks()
            .authenticate_credential_and_pin(f.host(), cred)
            .unwrap();
        let dest = f
            .client
            .foks()
            .load_and_pin_team_with_credential(
                f.host(),
                cred,
                &user.verified,
                &user.puks,
                &destination.team,
            )
            .unwrap();
        let removal = SecretSeed::new([n; 32]);
        let plan = f
            .client
            .foks()
            .invited_team_addition_plan(
                cred.uid(),
                &dest,
                verified.verified(),
                source_role,
                role,
                &removal,
            )
            .unwrap();
        let added = f
            .client
            .foks()
            .add_invited_team_durable(
                f.host(),
                cred,
                &destination.team,
                verified.verified(),
                None,
                delivered.receipt.as_ref().unwrap(),
                &plan,
                &removal,
                &mut protected,
            )
            .unwrap();
        assert!(added
            .authenticated
            .verified
            .members()
            .iter()
            .any(|m| m.party == source.team && m.source_role == source_role && m.role == role));
        assert!(f
            .client
            .foks()
            .team_invitation_inbox(f.host(), cred, &destination.team, None)
            .unwrap()
            .is_empty());
    }
}

#[test]
fn remote_team_invitation_posts_home_intent_and_loads_destination_keys() {
    let home = Fixture::start("remote-team-invite-home");
    let dest = Fixture::start("remote-team-invite-dest");
    let a = home
        .client
        .create_account(home.host(), &TestAccountSpec::new("remoteteamjoiner", 131))
        .unwrap();
    let b = dest
        .client
        .create_account(dest.host(), &TestAccountSpec::new("remoteteamadmin", 132))
        .unwrap();
    let secrets = |n| NamedTeamSecrets {
        member_min: SecretSeed::new([n; 32]),
        member: SecretSeed::new([n + 1; 32]),
        admin: SecretSeed::new([n + 2; 32]),
        owner: SecretSeed::new([n + 3; 32]),
        removal_key: SecretSeed::new([n + 4; 32]),
        team_name_commitment_key: [n + 5; 16],
    };
    let source = home
        .client
        .foks()
        .create_single_owner_named_team(home.host(), &a.credential, "remotechild", &secrets(140))
        .unwrap();
    let destination = dest
        .client
        .foks()
        .create_single_owner_named_team(dest.host(), &b.credential, "remoteparent", &secrets(150))
        .unwrap();
    let mut hp = home.client.open_protected_store().unwrap();
    let mut dp = dest.client.open_protected_store().unwrap();
    home.client
        .foks()
        .lower_team_index_range_durable(home.host(), &a.credential, &source.team, &mut hp)
        .unwrap();
    dest.client
        .foks()
        .raise_team_index_range_durable(dest.host(), &b.credential, &destination.team, &mut dp)
        .unwrap();
    let ac = FederationCredential::Software(&a.credential);
    let bc = FederationCredential::Software(&b.credential);
    let cert = dest
        .client
        .foks()
        .prepare_team_invitation(dest.host(), bc, &destination.team)
        .unwrap();
    dest.client
        .foks()
        .upload_team_invitation(dest.host(), bc, &cert)
        .unwrap();
    let prepared = home
        .client
        .foks()
        .prepare_remote_team_invitation(
            home.host(),
            dest.host(),
            ac,
            &source.team,
            Role::ADMIN,
            &cert.invite,
        )
        .unwrap();
    let op = home
        .client
        .foks()
        .prepare_invitation_operation(
            home.host(),
            ac,
            foks_client::InvitationIntent::RemoteAcceptance(Box::new(prepared)),
            &mut hp,
        )
        .unwrap();
    home.environment
        .arm_fault(foks_server_testkit::TestFault::InvitationHomeTeamAfterCommitBeforeResponse);
    home.client
        .foks()
        .remote_invitation_progress(
            home.host(),
            dest.host(),
            ac,
            op.operation.operation_id,
            true,
            &mut hp,
        )
        .unwrap();
    let row = dest
        .client
        .foks()
        .team_invitation_inbox(dest.host(), bc, &destination.team, None)
        .unwrap()
        .pop()
        .unwrap();
    let foks_proto::RawInboxRequest::Remote(request) = &row.request else {
        panic!()
    };
    let payload = dest
        .client
        .foks()
        .open_remote_invitation(dest.host(), bc, &destination.team, request)
        .unwrap();
    let remote = dest
        .client
        .foks()
        .verify_remote_invitation_team(home.host(), &payload)
        .unwrap();
    let user = dest
        .client
        .foks()
        .authenticate_credential_and_pin(dest.host(), bc)
        .unwrap();
    let loaded = dest
        .client
        .foks()
        .load_and_pin_team_with_credential(
            dest.host(),
            bc,
            &user.verified,
            &user.puks,
            &destination.team,
        )
        .unwrap();
    let removal = SecretSeed::new([170; 32]);
    assert!(dest
        .client
        .foks()
        .invited_team_addition_plan(
            bc.uid(),
            &loaded,
            &remote.verified,
            Role::ADMIN,
            Role::ADMIN,
            &removal
        )
        .is_err());
    let plan = dest
        .client
        .foks()
        .invited_team_addition_plan(
            bc.uid(),
            &loaded,
            &remote.verified,
            Role::ADMIN,
            Role::member(0),
            &removal,
        )
        .unwrap();
    dest.client
        .foks()
        .add_invited_team_durable(
            dest.host(),
            bc,
            &destination.team,
            &remote.verified,
            Some(&payload.permission),
            &row.receipt,
            &plan,
            &removal,
            &mut dp,
        )
        .unwrap();
    let loaded = home
        .client
        .foks()
        .load_invited_remote_team_as_team(
            home.host(),
            dest.host(),
            ac,
            &source.team,
            Role::ADMIN,
            &destination.team,
        )
        .unwrap();
    assert!(loaded.ptks.len() >= 2);
    assert!(dest
        .client
        .foks()
        .team_invitation_inbox(dest.host(), bc, &destination.team, None)
        .unwrap()
        .is_empty());
}

#[test]
fn hardware_admin_approves_local_invitation_with_durable_recovery() {
    use foks_yubi::{MockYubiProvider, Pin, PivPolicy, SlotId, YubiProvider as _};
    let f = Fixture::start("invitation-hardware-admin");
    let pin = Pin::new("654321").unwrap();
    let provider = MockYubiProvider::with_card("invite-key", 72001, &pin).unwrap();
    let card = provider.cards().unwrap().remove(0);
    let key = provider
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
    let mut protected = f.client.open_protected_store().unwrap();
    let owner = f
        .client
        .foks()
        .create_yubi_account(
            f.host(),
            key.device.as_ref(),
            foks_client::YubiAccountRequest {
                username_utf8: "hardwareinviteadmin".into(),
                device_name: "security key".into(),
                invite_code: foks_proto::InviteCode::Empty,
                email: String::new(),
                passphrase: None,
                pq_hint: foks_proto::YubiSlotAndPqKeyId {
                    slot: 0x83,
                    id: key.locator.pq_key_id,
                },
            },
            foks_client::YubiAccountSecrets::new(
                SecretSeed::new([181; 32]),
                SecretSeed::new([182; 32]),
                [183; 17],
            ),
            f.client.soft_state_path(),
            &mut protected,
        )
        .unwrap();
    let cred = FederationCredential::Yubi(&owner.credential);
    let team = f
        .client
        .foks()
        .create_single_owner_named_team_yubi(
            f.host(),
            &owner.credential,
            "hardwareinviteteam",
            &NamedTeamSecrets {
                member_min: SecretSeed::new([184; 32]),
                member: SecretSeed::new([185; 32]),
                admin: SecretSeed::new([186; 32]),
                owner: SecretSeed::new([187; 32]),
                removal_key: SecretSeed::new([188; 32]),
                team_name_commitment_key: [189; 16],
            },
        )
        .unwrap();
    f.client
        .foks()
        .narrow_team_index_range_durable(
            f.host(),
            cred,
            &team.team,
            foks_client::TeamIndexRangeDirection::Raise,
            &mut protected,
        )
        .unwrap();
    let cert = f
        .client
        .foks()
        .prepare_team_invitation(f.host(), cred, &team.team)
        .unwrap();
    f.client
        .foks()
        .upload_team_invitation(f.host(), cred, &cert)
        .unwrap();
    let jc = TestClient::new(&f.environment, "hardware-invite-joiner").unwrap();
    let joiner = jc
        .create_account(f.host(), &TestAccountSpec::new("hardwareinvitejoiner", 191))
        .unwrap();
    let joincred = FederationCredential::Software(&joiner.credential);
    let accept = jc
        .foks()
        .prepare_local_invitation_acceptance(f.host(), joincred, &cert.invite)
        .unwrap();
    jc.foks()
        .submit_local_invitation_acceptance(f.host(), joincred, &accept)
        .unwrap();
    let row = f
        .client
        .foks()
        .team_invitation_inbox(f.host(), cred, &team.team, None)
        .unwrap()
        .pop()
        .unwrap();
    let target = f
        .client
        .foks()
        .load_local_invitation_joiner(f.host(), cred, &team.team, &row)
        .unwrap();
    let actor = f
        .client
        .foks()
        .authenticate_credential_and_pin(f.host(), cred)
        .unwrap();
    let loaded = f
        .client
        .foks()
        .load_and_pin_team_with_credential(f.host(), cred, &actor.verified, &actor.puks, &team.team)
        .unwrap();
    let removal = SecretSeed::new([192; 32]);
    let request = foks_client::AddLocalTeamMemberRequest {
        target_user: &target,
        destination_role: Role::member(0),
        removal_key: &removal,
    };
    let plan = f
        .client
        .foks()
        .local_team_member_addition_plan(cred.uid(), &team.team, &loaded, &request)
        .unwrap();
    f.client
        .foks()
        .add_invited_local_user_durable(f.host(), cred, &team.team, &plan, &request, &mut protected)
        .unwrap();
    let recovered = f
        .client
        .foks()
        .resume_invited_local_addition(f.host(), cred, &team.team, &plan, &mut protected)
        .unwrap();
    assert_eq!(recovered.authenticated.verified.members().len(), 2);
    assert!(f
        .client
        .foks()
        .team_invitation_inbox(f.host(), cred, &team.team, None)
        .unwrap()
        .is_empty());
}

#[test]
fn invitation_inbox_ties_are_bounded_and_do_not_skip_filtered_prefixes() {
    let f = Fixture::start("invitation-inbox-limits");
    let owner = f
        .client
        .create_account(f.host(), &TestAccountSpec::new("inboxlimitadmin", 181))
        .unwrap();
    let cred = FederationCredential::Software(&owner.credential);
    let team = f
        .client
        .foks()
        .create_single_owner_named_team(
            f.host(),
            &owner.credential,
            "inboxlimits",
            &NamedTeamSecrets {
                member_min: SecretSeed::new([182; 32]),
                member: SecretSeed::new([183; 32]),
                admin: SecretSeed::new([184; 32]),
                owner: SecretSeed::new([185; 32]),
                removal_key: SecretSeed::new([186; 32]),
                team_name_commitment_key: [187; 16],
            },
        )
        .unwrap()
        .team;
    let cert = f
        .client
        .foks()
        .prepare_team_invitation(f.host(), cred, &team)
        .unwrap();
    f.client
        .foks()
        .upload_team_invitation(f.host(), cred, &cert)
        .unwrap();
    let remote = include_bytes!(
        "../../../foks-snowpack/tests/fixtures/foks-v0.1.9/invitations/remote.request"
    );
    let mut db = rusqlite::Connection::open(f.environment.database_path()).unwrap();
    let tx = db.transaction().unwrap();
    // Deliberately seed beyond admission quotas to exercise an all-tied page
    // larger than the maximum reader limit (including existing imported rows).
    for n in 0..1001_u32 {
        let mut local = [57_u8; 17];
        local[1..5].copy_from_slice(&n.to_be_bytes());
        let mut remote_receipt = local;
        remote_receipt[0] = 56;
        let mut joiner = [1_u8; 33];
        joiner[1..5].copy_from_slice(&n.to_be_bytes());
        tx.execute("INSERT INTO team_local_join_requests(receipt,team_id,joiner_id,source_role_type,source_visibility,state,permission,created_ms) VALUES(?1,?2,?3,3,0,0,?4,1000)",rusqlite::params![local.as_slice(),team.as_bytes(),joiner.as_slice(),[55_u8;17].as_slice()]).unwrap();
        tx.execute("INSERT INTO team_remote_join_requests(receipt,team_id,certificate_hash,exact_request,state,created_ms) VALUES(?1,?2,?3,?4,0,1000)",rusqlite::params![remote_receipt.as_slice(),team.as_bytes(),cert.invite.hash.as_slice(),remote.as_slice()]).unwrap();
    }
    tx.commit().unwrap();
    let read = |limit, start, end| {
        f.client
            .foks()
            .team_invitation_inbox(
                f.host(),
                cred,
                &team,
                Some(foks_proto::InboxPagination { limit, start, end }),
            )
            .unwrap()
    };
    assert_eq!(read(0, 0, 0).len(), 200); // Go's limit applies to each request kind.
    let page = read(3, 1000, 1000);
    assert_eq!(page.len(), 6);
    assert!(page.iter().all(|r| r.time == 1000));
    assert_eq!(read(10_000, 1000, 1000).len(), 2000); // remains incomplete; never subtract a millisecond.
    let identities = page
        .iter()
        .map(|r| r.receipt.expose().to_vec())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(identities.len(), 6);
    // A newer insert between inclusive reads cannot make the older tie group
    // disappear or be silently treated as already traversed.
    db.execute("UPDATE team_remote_join_requests SET created_ms=1001 WHERE receipt=(SELECT max(receipt) FROM team_remote_join_requests)",[]).unwrap();
    assert_eq!(read(1000, 1000, 1000).len(), 2000);
    assert_eq!(read(1000, 1001, 1001).len(), 1);
    // Terminal rows newer than every pending row must be filtered before LIMIT.
    db.execute("UPDATE team_local_join_requests SET state=2,created_ms=2000 WHERE receipt IN(SELECT receipt FROM team_local_join_requests ORDER BY receipt LIMIT 100)",[]).unwrap();
    assert_eq!(read(100, 0, 0).len(), 200);
    assert!(read(100, 2000, 2000).is_empty());
    assert!(page
        .iter()
        .all(|r| matches!(r.state, foks_proto::JoinRequestState::Pending)));
    // Existing rows remain readable at capacity, but another public guest
    // admission is rejected without creating a receipt or evicting a request.
    let home = Fixture::start("invitation-quota-home");
    let applicant = home
        .client
        .create_account(home.host(), &TestAccountSpec::new("quotaapplicant", 211))
        .unwrap();
    let prepared = home
        .client
        .foks()
        .prepare_remote_user_invitation(
            home.host(),
            f.host(),
            FederationCredential::Software(&applicant.credential),
            &cert.invite,
        )
        .unwrap();
    let before = read(1000, 0, 0).len();
    assert!(matches!(
        home.client
            .foks()
            .submit_remote_invitation(f.host(), &prepared),
        Err(foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: 1060,
            ..
        }))
    ));
    assert_eq!(read(1000, 0, 0).len(), before);
}
