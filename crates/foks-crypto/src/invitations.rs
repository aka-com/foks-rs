//! Certificate cryptography; verified team loading is still required for admission.
use crate::{
    derive_shared_public, hepk_fingerprint, prefixed_hash_signable, sign_shared_key_typed,
    verify_typed, Error, Result,
};
use foks_proto::{
    FqTeam, Role, SecretSeed, TeamCertificate, TeamCertificatePayload, TeamInvite, UserSharedKey,
    ENTITY_PTK_VERIFY, TEAM_CERT_SIGNED_TYPE_ID, TEAM_CERT_TYPE_ID,
};

pub fn team_certificate_invite(cert: &TeamCertificate) -> Result<TeamInvite> {
    let payload = TeamCertificatePayload::decode(&cert.payload)?;
    Ok(TeamInvite {
        host: payload.team.host,
        hash: prefixed_hash_signable(TEAM_CERT_TYPE_ID, &cert.encoded()?)?,
    })
}
/// Checks the immutable invitation hash, advertised host, HEPK and complete stack.
/// The name is signed information, not a fresh name binding or membership proof.
pub fn verify_team_certificate(
    cert: &TeamCertificate,
    invite: &TeamInvite,
) -> Result<TeamCertificatePayload> {
    foks_snowpack::validate_signable(&cert.payload)?;
    if team_certificate_invite(cert)? != *invite {
        return Err(Error::PublicKey);
    }
    let p = TeamCertificatePayload::decode(&cert.payload)?;
    if hepk_fingerprint(&p.hepk)? != p.key.hepk_fingerprint {
        return Err(Error::PublicKey);
    }
    let keys = if p.key.generation == 1 {
        vec![&p.team.team]
    } else {
        vec![&p.key.verify_key, &p.team.team]
    };
    if cert.signatures.len() != keys.len() {
        return Err(Error::PublicKey);
    }
    if p.key.generation == 1 && p.team.team.ed25519_key()? != p.key.verify_key.ed25519_key()? {
        return Err(Error::PublicKey);
    }
    for (i, key) in keys.into_iter().enumerate() {
        verify_typed(
            key,
            &cert.signatures[i],
            TEAM_CERT_SIGNED_TYPE_ID,
            &cert.signing_bytes(i)?,
        )?;
    }
    Ok(p)
}
pub fn make_team_certificate(
    team: FqTeam,
    first: &SecretSeed,
    current: &SecretSeed,
    generation: u64,
    name: Vec<u8>,
    time: u64,
) -> Result<TeamCertificate> {
    let public = derive_shared_public(current, ENTITY_PTK_VERIFY)?;
    let p = TeamCertificatePayload {
        team,
        key: UserSharedKey {
            generation,
            role: Role::ADMIN,
            verify_key: public.verify_key,
            hepk_fingerprint: hepk_fingerprint(&public.hepk)?,
        },
        time,
        hepk: public.hepk,
        name: Some(name),
    };
    let mut cert = TeamCertificate {
        payload: p.encoded()?,
        signatures: vec![],
    };
    for seed in if generation == 1 {
        vec![current]
    } else {
        vec![current, first]
    } {
        cert.signatures.push(sign_shared_key_typed(
            seed,
            TEAM_CERT_SIGNED_TYPE_ID,
            &cert.signing_bytes(cert.signatures.len())?,
        )?);
    }
    verify_team_certificate(&cert, &team_certificate_invite(&cert)?)?;
    Ok(cert)
}
/// Boxing establishes payload confidentiality, not the claimed joiner's authority.
pub fn seal_remote_join_request(
    sender_seed: &SecretSeed,
    certificate: &TeamCertificatePayload,
    payload: &foks_proto::RemoteJoinPayload,
    randomness: &crate::PukBoxRandomness,
) -> Result<foks_proto::RemoteJoinRequest> {
    let sender = derive_shared_public(sender_seed, ENTITY_PTK_VERIFY)?;
    let clear = zeroize::Zeroizing::new(payload.encoded()?);
    let encrypted = crate::seal_hybrid_payload(
        sender_seed,
        &sender.hepk,
        &certificate.hepk,
        foks_proto::TEAM_REMOTE_JOIN_PAYLOAD_TYPE_ID,
        &clear,
        randomness,
        true,
    )?;
    Ok(foks_proto::RemoteJoinRequest {
        hepk_fingerprint: hepk_fingerprint(&certificate.hepk)?,
        encrypted,
        visible: payload.visible.clone(),
    })
}
/// Caller must subsequently verify the claimed joiner's chain using the permission.
pub fn open_remote_join_request(
    request: &foks_proto::RemoteJoinRequest,
    receiver: &dyn crate::HybridSecretDecapsulator,
) -> Result<foks_proto::RemoteJoinPayload> {
    if hepk_fingerprint(receiver.hepk())? != request.hepk_fingerprint {
        return Err(Error::WrongReceiver);
    }
    let sender = request
        .encrypted
        .sender_dh
        .as_ref()
        .ok_or(Error::HybridBox)?;
    let clear = crate::open_hybrid_box(
        &request.encrypted,
        receiver,
        sender,
        foks_proto::TEAM_REMOTE_JOIN_PAYLOAD_TYPE_ID,
    )?;
    let payload = foks_proto::RemoteJoinPayload::decode(&clear)?;
    if payload.visible != request.visible {
        return Err(Error::PukBinding);
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_remote_box_and_inbox_keep_visibility_and_receipt_kinds() {
        let root = "../foks-snowpack/tests/fixtures/foks-v0.1.9/invitations";
        let read = |name: &str| std::fs::read(format!("{root}/{name}")).unwrap();
        let cert = TeamCertificate::decode(&read("initial.cert")).unwrap();
        let cp = TeamCertificatePayload::decode(&cert.payload).unwrap();
        let first = SecretSeed::new(std::array::from_fn(|i| i as u8 + 11));
        let receiver = crate::SharedKeyDecapsulator::new(&first, cp.team.team.clone()).unwrap();
        let req = foks_proto::RemoteJoinRequest::decode(&read("remote.request")).unwrap();
        assert_eq!(req.encoded().unwrap(), read("remote.request"));
        let payload = open_remote_join_request(&req, &receiver).unwrap();
        assert_eq!(payload.encoded().unwrap(), read("remote.payload"));
        let rows = foks_proto::decode_team_inbox(&read("local.inbox")).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].receipt.is_remote());
        assert_eq!(
            foks_proto::encode_team_inbox(&rows).unwrap(),
            read("local.inbox")
        );
        let grant =
            foks_proto::LocalViewPermissionPayload::decode(&read("local-grant.payload")).unwrap();
        assert_eq!(grant.encoded().unwrap(), read("local-grant.payload"));
        let mut wrong = req.clone();
        wrong.hepk_fingerprint[0] ^= 1;
        assert!(open_remote_join_request(&wrong, &receiver).is_err());
        wrong = req.clone();
        wrong.encrypted.ciphertext[0] ^= 1;
        assert!(open_remote_join_request(&wrong, &receiver).is_err());
        wrong = req.clone();
        wrong.visible.index_range = Some(foks_proto::RationalRange {
            low: foks_proto::Rational {
                infinity: false,
                base: vec![],
                exponent: 0,
            },
            high: foks_proto::Rational {
                infinity: true,
                base: vec![],
                exponent: 0,
            },
        });
        assert!(open_remote_join_request(&wrong, &receiver).is_err());
    }
    #[test]
    fn official_initial_and_rotated_certificates_verify_exact_hash_and_stack() {
        for n in ["initial", "rotated"] {
            let root = "../foks-snowpack/tests/fixtures/foks-v0.1.9/invitations";
            let read = |ext: &str| std::fs::read(format!("{root}/{n}.{ext}")).unwrap();
            let bytes = read("cert");
            let cert = TeamCertificate::decode(&bytes).unwrap();
            assert_eq!(cert.encoded().unwrap(), bytes);
            let invite = TeamInvite::decode(&read("invite")).unwrap();
            assert_eq!(invite.export().unwrap().as_bytes(), read("txt"));
            assert_eq!(
                TeamInvite::import(&invite.export().unwrap()).unwrap(),
                invite
            );
            let p = verify_team_certificate(&cert, &invite).unwrap();
            let first = SecretSeed::new(std::array::from_fn(|i| i as u8 + 11));
            let current = SecretSeed::new(std::array::from_fn(|i| {
                i as u8 + if n == "initial" { 11 } else { 43 }
            }));
            let made = make_team_certificate(
                p.team.clone(),
                &first,
                &current,
                p.key.generation,
                p.name.unwrap(),
                p.time,
            )
            .unwrap();
            assert_eq!(made.encoded().unwrap(), bytes);
            let mut bad = invite.clone();
            bad.hash[0] ^= 1;
            assert!(verify_team_certificate(&cert, &bad).is_err());
            let mut forged = cert.clone();
            forged.payload[5] ^= 1;
            assert!(verify_team_certificate(&forged, &invite).is_err());
            let mut swapped = cert.clone();
            swapped.signatures.reverse();
            if n == "rotated" {
                assert!(verify_team_certificate(
                    &swapped,
                    &team_certificate_invite(&swapped).unwrap()
                )
                .is_err());
            }
        }
    }
}
