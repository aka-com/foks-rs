//! Native ordering for the restricted imported List owner. These SQL-derived
//! values are not a TLS, schema, ownership, durability or publication proof.
use super::{
    admission, bytes, stored_role, unsigned, CredentialKind, CurrentCredential, PersonalError,
    Result,
};
use crate::{Sql, Statement as S, Value as V};
use foks_proto::EntityId;

/// Current issuance binding, before native authority/role resolution.
/// Only this module's fixed SQL admission constructs it.
#[derive(Debug, Eq, PartialEq)]
pub struct IssuedCredential {
    uid: [u8; 33],
    credential: EntityId,
    kind: CredentialKind,
    stored_expiry_micros: u64,
}
impl IssuedCredential {
    pub fn uid(&self) -> &[u8; 33] {
        &self.uid
    }
    pub fn credential_id(&self) -> &[u8] {
        self.credential.as_bytes()
    }
    pub fn kind(&self) -> CredentialKind {
        self.kind
    }
    pub fn stored_expiry_micros(&self) -> u64 {
        self.stored_expiry_micros
    }
}

/// The caller supplies original TLS-derived parsed facts and an open transaction.
/// Import has established serial-padding completeness and exact-DER uniqueness.
/// DER expiry conversion occurs in the native caller after this succeeds.
pub fn authenticate(
    sql: &mut impl Sql,
    exact_der: &[u8],
    serial_magnitude: &[u8],
    public_key: &[u8; 32],
    now_micros: u64,
) -> Result<IssuedCredential> {
    if exact_der.is_empty()
        || exact_der.len() > super::MAX_PEER_CERTIFICATE_BYTES
        || serial_magnitude.is_empty()
        || serial_magnitude.len() > 20
        || (serial_magnitude.len() > 1 && serial_magnitude[0] == 0)
    {
        return Err(PersonalError::Corrupt);
    }
    let now = crate::integer(now_micros)?;
    let mut found = None;
    for width in serial_magnitude.len()..=20 {
        let mut serial = admission::zeroed(width)?;
        serial[width - serial_magnitude.len()..].copy_from_slice(serial_magnitude);
        let row = sql.query(
            S::ListIssuedCredential,
            &[
                V::Blob(serial),
                V::Blob(admission::copy(exact_der)?),
                now.clone(),
            ],
        )?;
        let Some(row) = row else { continue };
        if found.is_some() {
            return Err(PersonalError::Corrupt);
        }
        let [uid, credential, role_type, visibility, expiry]: [V; 5] =
            row.try_into().map_err(|_| PersonalError::Corrupt)?;
        let uid: [u8; 33] = bytes(&uid)?
            .try_into()
            .map_err(|_| PersonalError::Corrupt)?;
        // R3 proves personal UID identity. Preserve the native scalar unsigned
        // conversions here without interpreting a full Role before the anchor.
        unsigned(&role_type)?;
        if !matches!(visibility, V::Integer(_)) {
            return Err(PersonalError::Corrupt);
        }
        let stored_expiry_micros = unsigned(&expiry)?;
        let credential = EntityId::from_bytes(admission::into_blob(credential)?)
            .map_err(|_| PersonalError::Corrupt)?;
        if credential
            .ed25519_key()
            .map_err(|_| PersonalError::Corrupt)?
            != *public_key
        {
            return Err(PersonalError::Unauthenticated);
        }
        let kind = match credential.entity_type() {
            foks_proto::ENTITY_DEVICE => CredentialKind::SoftwareDevice,
            foks_proto::ENTITY_SUBKEY => CredentialKind::DelegatedSubkey,
            foks_proto::ENTITY_BACKUP_KEY => CredentialKind::BackupKey,
            foks_proto::ENTITY_BOT_TOKEN_KEY => CredentialKind::BotToken,
            _ => return Err(PersonalError::Unauthenticated),
        };
        found = Some(IssuedCredential {
            uid,
            credential,
            kind,
            stored_expiry_micros,
        });
    }
    found.ok_or(PersonalError::Unauthenticated)
}

/// The observed native authority prefix, not a portable/replayable capability.
#[derive(Debug, Eq, PartialEq)]
pub enum AuthorityDecision {
    NoOwner,
    NoAnchor {
        owner: Vec<u8>,
    },
    InvalidRole {
        owner: Vec<u8>,
        role_type: i64,
        visibility: i64,
    },
    Authorized(CurrentCredential),
}

/// Called only after the native outer argument decode, authority clock and
/// personal classification. No full-state reconstruction or issuance-owner pin.
pub fn authority(sql: &mut impl Sql, issued: IssuedCredential) -> Result<AuthorityDecision> {
    let row = sql.query(
        S::ListCurrentOwner,
        &[
            V::Blob(admission::copy(&issued.uid)?),
            V::Blob(admission::copy(issued.credential.as_bytes())?),
        ],
    )?;
    let Some(row) = row else {
        return Ok(AuthorityDecision::NoOwner);
    };
    let [owner]: [V; 1] = row.try_into().map_err(|_| PersonalError::Corrupt)?;
    let owner = admission::into_blob(owner)?;
    if !matches!(owner.len(), 33 | 34) {
        return Err(PersonalError::Corrupt);
    }
    if sql
        .query(S::PersonalAnchor, &[V::Blob(admission::copy(&issued.uid)?)])?
        .is_none()
    {
        return Ok(AuthorityDecision::NoAnchor { owner });
    }
    let row = sql
        .query(
            S::PersonalCurrentCredential,
            &[
                V::Blob(admission::copy(&owner)?),
                V::Blob(admission::copy(&issued.uid)?),
                V::Blob(admission::copy(issued.credential.as_bytes())?),
            ],
        )?
        .ok_or(PersonalError::Corrupt)?;
    let [V::Integer(role_type), V::Integer(visibility)]: [V; 2] =
        row.try_into().map_err(|_| PersonalError::Corrupt)?
    else {
        return Err(PersonalError::Corrupt);
    };
    let Ok(role) = stored_role(&V::Integer(role_type), &V::Integer(visibility)) else {
        return Ok(AuthorityDecision::InvalidRole {
            owner,
            role_type,
            visibility,
        });
    };
    Ok(AuthorityDecision::Authorized(CurrentCredential {
        uid: issued.uid,
        credential: issued.credential,
        owner,
        role,
        kind: issued.kind,
        expires_at_micros: issued.stored_expiry_micros,
    }))
}

#[cfg(test)]
mod tests;
