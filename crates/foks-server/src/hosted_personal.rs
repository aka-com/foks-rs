//! Feature-only A1 qualification helpers. These functions do not install a
//! listener route or authenticate frontend ownership. The caller must already
//! establish TLS possession and own the SQL transaction/commit/durable-sync gate.
use foks_metadata_core::{authority, personal, Sql};
use rustls_pki_types::CertificateDer;
use std::sync::Arc;

#[derive(Debug)]
pub enum QualificationError {
    Certificate(crate::Error),
    Personal(personal::PersonalError),
    NativeStatus(foks_rpc::RpcStatus),
}

/// Parse the exact authenticated peer and resolve its current database authority
/// within the caller's transaction. No synthetic identity selector is accepted.
pub fn execute(
    sql: &mut impl Sql,
    certificate: &CertificateDer<'_>,
    now: authority::Number,
    deadline: authority::Number,
    request: authority::Request,
) -> Result<authority::Reply, QualificationError> {
    let peer = crate::auth::parse_hosted_peer_certificate(certificate)
        .map_err(QualificationError::Certificate)?;
    authority::run_personal(
        sql,
        authority::PersonalInvocation {
            peer,
            now,
            deadline,
            request,
        },
    )
    .map_err(QualificationError::Personal)
}

/// Unchanged native key-decision oracle, for differential qualification only.
/// Booleans retain the existing predicate results without inventing route-specific
/// BadArguments versus PermissionDenied status precedence.
pub fn native_key_decisions(
    certificate: &CertificateDer<'_>,
    reader: &foks_server_db::ReadDatabase,
    clock: &Arc<dyn foks_server_db::Clock>,
    requirements: &[personal::KeyRequirement],
) -> Result<Vec<bool>, QualificationError> {
    let now = clock
        .now_micros()
        .map_err(crate::Error::from)
        .map_err(QualificationError::Certificate)?;
    let principal = crate::auth::Principal::authenticate(certificate, reader, now)
        .map_err(QualificationError::Certificate)?;
    crate::services::kv::qualify_personal_key_requirements(&principal, reader, clock, requirements)
        .map_err(QualificationError::NativeStatus)
}

/// Decode public certificate parts for immutable qualification vectors. This is
/// not proof of TLS possession; production callers must get DER from mTLS.
pub fn peer_certificate(
    certificate: &CertificateDer<'_>,
) -> Result<personal::PeerCertificate, QualificationError> {
    crate::auth::parse_hosted_peer_certificate(certificate).map_err(QualificationError::Certificate)
}

/// Revision-bound scalar observation under the same transaction-local resolver.
pub fn read_point(
    sql: &mut impl Sql,
    invocation: authority::PersonalPointInvocation,
) -> Result<foks_metadata_core::wire::Reply, QualificationError> {
    authority::read_personal_point(sql, invocation).map_err(QualificationError::Personal)
}

/// Exact native Usage bytes/status for the public route's existing personal
/// authority and snapshot path. This installs no hosted production route.
pub fn native_usage(
    certificate: &CertificateDer<'_>,
    reader: &foks_server_db::ReadDatabase,
    clock: &Arc<dyn foks_server_db::Clock>,
) -> Result<Result<Vec<u8>, foks_rpc::RpcStatus>, QualificationError> {
    let now = clock
        .now_micros()
        .map_err(crate::Error::from)
        .map_err(QualificationError::Certificate)?;
    let principal = crate::auth::Principal::authenticate(certificate, reader, now)
        .map_err(QualificationError::Certificate)?;
    Ok(crate::services::kv::qualify_personal_usage(
        &principal, reader, clock,
    ))
}
