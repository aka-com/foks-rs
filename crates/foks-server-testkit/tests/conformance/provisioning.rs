use foks_client::{
    NewSoftwareDeviceSecrets, NoPassphraseConfigured, SoftwareDeviceProvisionRequest,
    UserPukRotation,
};
use foks_proto::{
    Role, RtAppId, RtGetInboxVersionArgument, RtInboxKey, RtPollInbox, RtPollInboxArgument,
    SecretSeed,
};
use foks_rpc::RealtimeRequest;
use foks_server_testkit::TestAccountSpec;
use std::io::Write as _;

use crate::authorization::MAX_RESPONSE;
use crate::support::Fixture;

#[test]
pub(crate) fn provisioning_success() {
    let fixture = Fixture::start("provisioning-client");
    let created = fixture
        .client
        .create_account(fixture.host(), &TestAccountSpec::new("provisionuser", 0x31))
        .unwrap();
    let mut protected = fixture.client.open_protected_store().unwrap();
    let provisioned = fixture
        .client
        .foks()
        .provision_software_device(
            fixture.host(),
            &created.credential,
            SoftwareDeviceProvisionRequest {
                role: Role::OWNER,
                device_name: "second device".to_owned(),
                serial: 2,
            },
            NewSoftwareDeviceSecrets::new(SecretSeed::new([0x41; 32]), None, [0x42; 17]),
            &mut protected,
        )
        .unwrap();
    assert_eq!(provisioned.authenticated.verified.chain_seqno(), 2);
    assert_eq!(provisioned.authenticated.verified.devices().len(), 2);

    let original = foks_crypto::derive_device_public(&SecretSeed::new([0x31; 32])).unwrap();
    let revoked = fixture
        .client
        .foks()
        .revoke_user_credential_with_software_device(
            fixture.host(),
            &provisioned.credential,
            &original.id,
            &[UserPukRotation {
                role: Role::OWNER,
                previous_generation: 1,
                previous_seed: SecretSeed::new([0x32; 32]),
                new_seed: SecretSeed::new([0x51; 32]),
            }],
            Some(NoPassphraseConfigured),
            &mut protected,
        )
        .unwrap();
    assert_eq!(revoked.verified.chain_seqno(), 3);
    assert_eq!(revoked.verified.devices().len(), 1);
    assert_eq!(
        revoked.verified.shared_key(Role::OWNER).unwrap().generation,
        2
    );

    // The revoked device's certificate still chains to the client CA and is
    // within its validity, so TLS accepts it. As on a go-foks host, each
    // authenticated request on that connection, bounded or realtime poll,
    // is answered with AUTH_ERROR at its sequence and the connection stays
    // open.
    let mut tls = crate::authorization::authenticated_stream(&fixture, &created.credential);
    let inbox_version = RealtimeRequest::GetInboxVersion(RtGetInboxVersionArgument {
        key: RtInboxKey { app: RtAppId::Chat },
    });
    let poll = RealtimeRequest::PollInbox(RtPollInboxArgument {
        poll: RtPollInbox {
            app: RtAppId::Chat,
            since: 0,
            timeout_milliseconds: 200,
        },
    });
    for (sequence, request) in [&inbox_version, &poll, &inbox_version]
        .into_iter()
        .enumerate()
    {
        let sequence = sequence as u64;
        tls.write_all(&request.encode_at(sequence).unwrap())
            .unwrap();
        tls.flush().unwrap();
        let error = foks_rpc::read_response(&mut tls, MAX_RESPONSE, sequence).unwrap_err();
        assert!(
            matches!(
                error,
                foks_rpc::Error::RemoteStatus {
                    code: foks_rpc::STATUS_AUTH_ERROR,
                    ..
                }
            ),
            "request {sequence}: {error:?}"
        );
    }
    let error = fixture
        .client
        .foks()
        .host_config(fixture.host(), &created.credential)
        .unwrap_err();
    assert!(
        matches!(
            error,
            foks_client::Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: foks_rpc::STATUS_AUTH_ERROR,
                ..
            })
        ),
        "{error:?}"
    );
}
