//! Native protocol dispatch over a disposable, fenced qualification projection.
use super::*;
use crate::rpc::{RouteId, RoutedCall};

pub(super) fn response(
    data: &ServerData,
    listener: Listener,
    routed: std::result::Result<RoutedCall, RouteError>,
    certificate: Option<Vec<u8>>,
    peer_ip: std::net::IpAddr,
    sequence: u64,
) -> Result<Vec<u8>> {
    let adapter = data
        .proof
        .as_ref()
        .ok_or(crate::Error::Config("proof adapter missing"))?;
    let route = routed.as_ref().ok().map(|call| call.route.id);
    adapter.execute(
        route,
        Box::new(move |projection| {
            let mut projected = data.clone();
            projected.proof = None;
            projected.peer_ip = Some(peer_ip);
            projected.read_database = Some(crate::read_pool::ReadPool::new(
                projection.read_database,
                2,
            )?);
            projected.writer = Some(projection.writer);
            let principal = if listener == Listener::Authenticated {
                let certificate = CertificateDer::from(certificate.ok_or(crate::Error::Config(
                    "authenticated TLS session has no client certificate",
                ))?);
                let reader = projected
                    .read_database()
                    .map_err(|_| crate::Error::ReaderPool)?;
                Some(Principal::authenticate(
                    &certificate,
                    &reader,
                    projected.clock.now_micros()?,
                )?)
            } else {
                None
            };
            let result = match routed {
                Ok(call) if call.route.supported && allowed(call.route.id) => {
                    let authorization = if let Some(principal) = principal.as_ref() {
                        projected.authorize_principal(principal).map(|()| {
                            projected.writer = projected.writer.as_ref().map(|writer| {
                                writer.for_authenticated_request(
                                    principal.uid(),
                                    principal.device_id(),
                                    projected.clock.clone(),
                                )
                            });
                        })
                    } else {
                        Ok(())
                    };
                    authorization
                        .and_then(|()| handlers::response(&projected, call, principal.as_ref()))
                }
                Ok(_) | Err(RouteError::Unknown { .. } | RouteError::WrongListener) => {
                    Err(RpcStatus::Unsupported)
                }
                Err(RouteError::RequestTooLarge { .. }) => Err(RpcStatus::BadArguments(
                    "request exceeds the method limit".to_owned(),
                )),
            };
            match result {
                Ok(bytes) => Ok(bytes),
                Err(status) => Ok(encode_status_response_at(&status, sequence)?),
            }
        }),
    )
}

// This is deliberately an allowlist. Identity, credentials, memberships, roots,
// directories and keys are provisioned before import and immutable in the proof.
// The adapter additionally rejects changes outside its narrow KV base tables.
fn allowed(route: RouteId) -> bool {
    use RouteId::*;
    matches!(
        route,
        ProbeProbe
            | BeaconBeaconLookup
            | RegResolveUsername
            | RegGetClientCertChain
            | RegSelectVHost
            | RegStretchVersion
            | RegLoadUserChain
            | RegCheckNameExists
            | RegGetServerConfig
            | RegGetClientVersionInfo
            | RegProbeKeyExists
            | RegGetHostID
            | RegGetVHostMgmtHost
            | UserResolveUsername
            | UserPing
            | UserGetSalt
            | UserStretchVersion
            | UserLoadUserChain
            | UserGetPukForRole
            | UserGetPpeParcel
            | UserGetHostConfig
            | UserLoadGenericChain
            | UserGetTreeLocation
            | MerkleQueryLookup
            | MerkleQueryGetHistoricalRoots
            | MerkleQueryGetCurrentRoot
            | MerkleQueryGetCurrentRootHash
            | MerkleQueryCheckKeyExists
            | MerkleQueryGetCurrentRootSigned
            | MerkleQueryMLookup
            | MerkleQuerySelectVHost
            | KvStorePut
            | KvStorePutSmallFileOrSymlink
            | KvStoreGetRoot
            | KvStoreGet
            | KvStoreGetNode
            | KvStoreGetDir
            | KvStoreCacheCheck
            | KvStoreList
            | KvStoreLockAcquire
            | KvStoreLockRelease
            | KvStoreUsage
            | KvStoreSelectVHost
    )
}
