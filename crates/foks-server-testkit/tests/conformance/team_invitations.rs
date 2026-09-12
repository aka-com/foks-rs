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
