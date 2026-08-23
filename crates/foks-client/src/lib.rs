//! Native FOKS v0.1.9 public-host discovery.
//!
//! This is deliberately the smallest useful client slice: WebPKI TLS, the
//! public probe RPC, full host/Merkle verification, and one atomic SQLite
//! hard-state advancement. It does not create an account or handle secrets.

#![forbid(unsafe_code)]

use std::io::Write as _;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use foks_client_db::{Acceptance, HardStateStore, StoredHostSnapshot};
use foks_crypto::{
    derive_device_public, derive_subkey_id, device_signing_key_pkcs8, open_puk_parcel,
    open_puk_parcel_with, HybridSecretDecapsulator,
};
use foks_proto::{
    EntityId, HostchainTail, PukParcel, Role, SecretSeed, ENTITY_USER, SERVICE_MERKLE_QUERY,
    SERVICE_REG, SERVICE_USER,
};
use foks_rpc::{
    encode_get_client_cert_chain_request, encode_get_current_merkle_root_request,
    encode_get_historical_merkle_roots_request, encode_get_owner_puk_request,
    encode_load_user_chain_request, read_probe_response, read_response, write_probe_request,
    DEFAULT_MAX_FRAME_LENGTH,
};
use foks_snowpack::{decode, Value};
use foks_verify::{
    merkle_history_requirements, restore_merkle_anchor, restore_public_host_identity,
    restore_verified_user, verify_merkle_advance, verify_public_host, verify_user_chain,
    HostService, VerifiedMerkleAdvance, VerifiedPublicHost, VerifiedUserState,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use thiserror::Error;

pub const DEFAULT_PROBE_PORT: u16 = 4430;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeTarget {
    hostname: String,
    port: u16,
}

impl ProbeTarget {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            return Err(Error::Target("hostname is empty"));
        }
        let (hostname, port) = match input.rsplit_once(':') {
            Some((hostname, port)) if !hostname.contains(':') => {
                let port = port
                    .parse::<u16>()
                    .map_err(|_| Error::Target("port must be an integer from 1 through 65535"))?;
                (hostname, port)
            }
            Some(_) if input.contains(':') => {
                return Err(Error::Target("IPv6 literals are not FOKS hostnames"));
            }
            _ => (input, DEFAULT_PROBE_PORT),
        };
        let hostname = hostname.trim_end_matches('.').to_ascii_lowercase();
        if hostname.is_empty()
            || !hostname.is_ascii()
            || hostname.len() > 253
            || hostname.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        {
            return Err(Error::Target("invalid DNS hostname"));
        }
        if port == 0 {
            return Err(Error::Target("port must not be zero"));
        }
        Ok(Self { hostname, port })
    }

    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn address(&self) -> String {
        format!("{}:{}", self.hostname, self.port)
    }
}

#[derive(Debug)]
pub struct ProbeOutcome {
    pub acceptance: Acceptance,
    pub verified: VerifiedPublicHost,
    pub pinned: PinnedHost,
}

/// An authenticated host identity and its delegated service endpoints.
///
/// Values can only be loaded from durable hard state. Keeping the database
/// path inside this capability prevents an endpoint projection from one trust
/// store from being used to advance another.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PinnedHost {
    lookup_name: String,
    host_id: EntityId,
    database_path: PathBuf,
    registration: ProbeTarget,
    user: ProbeTarget,
    merkle_query: ProbeTarget,
}

impl PinnedHost {
    pub fn lookup_name(&self) -> &str {
        &self.lookup_name
    }

    pub fn host_id(&self) -> &EntityId {
        &self.host_id
    }
}

/// Device credential material used for mTLS. The master seed is never written
/// by this crate; callers should source it from the encrypted local key store.
pub struct DeviceCredential {
    pub uid: EntityId,
    pub seed: SecretSeed,
    pub certificate_chain: Vec<Vec<u8>>,
}

/// Yubi parent operations plus the software Ed25519 subkey used for mTLS.
pub struct YubiCredential<'a> {
    pub uid: EntityId,
    pub parent: &'a dyn HybridSecretDecapsulator,
    pub subkey_seed: SecretSeed,
    pub certificate_chain: Vec<Vec<u8>>,
}

#[derive(Debug)]
pub struct AuthenticatedUserOutcome {
    pub merkle_acceptance: Acceptance,
    pub acceptance: Acceptance,
    pub verified: VerifiedUserState,
    pub puk_seed: SecretSeed,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid probe target: {0}")]
    Target(&'static str),
    #[error("DNS lookup returned no addresses for {0}")]
    NoAddress(String),
    #[error("TCP connection to every resolved address failed: {0}")]
    Connect(std::io::Error),
    #[error("TLS configuration failed: {0}")]
    Tls(#[from] rustls::Error),
    #[error("invalid TLS server name")]
    ServerName,
    #[error("FOKS RPC failed: {0}")]
    Rpc(#[from] foks_rpc::Error),
    #[error("FOKS public state verification failed: {0}")]
    Verify(#[from] foks_verify::Error),
    #[error("FOKS hard-state update failed: {0}")]
    Database(#[from] foks_client_db::Error),
    #[error("invalid FOKS protocol value: {0}")]
    Protocol(#[from] foks_proto::Error),
    #[error("invalid canonical Snowpack: {0}")]
    Snowpack(#[from] foks_snowpack::Error),
    #[error("FOKS device cryptography failed: {0}")]
    Crypto(#[from] foks_crypto::Error),
    #[error("registration returned an invalid certificate chain")]
    CertificateChain,
    #[error("the supplied UID or device seed does not match the verified chain")]
    DeviceBinding,
    #[error("pinned host is missing or has a malformed {0} service endpoint")]
    PinnedService(&'static str),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub struct PublicClient {
    roots: rustls::RootCertStore,
    timeout: Duration,
    maximum_frame_length: usize,
}

impl Default for PublicClient {
    fn default() -> Self {
        Self::webpki()
    }
}

impl PublicClient {
    pub fn webpki() -> Self {
        Self {
            roots: rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
            timeout: Duration::from_secs(15),
            maximum_frame_length: DEFAULT_MAX_FRAME_LENGTH,
        }
    }

    pub fn with_roots(roots: rustls::RootCertStore) -> Self {
        Self {
            roots,
            timeout: Duration::from_secs(15),
            maximum_frame_length: DEFAULT_MAX_FRAME_LENGTH,
        }
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    pub fn probe(&self, target: &ProbeTarget) -> Result<Vec<u8>> {
        self.probe_with_stream(target)
    }

    fn probe_with_stream(&self, target: &ProbeTarget) -> Result<Vec<u8>> {
        let tcp = self.connect_tcp(target)?;
        let config = self.tls_config(None)?;
        let mut tls = self.connect_tls(target, tcp, config)?;
        write_probe_request(&mut tls, &target.hostname, 0, None)?;
        read_probe_response(&mut tls, self.maximum_frame_length).map_err(Into::into)
    }

    fn connect_tcp(&self, target: &ProbeTarget) -> Result<TcpStream> {
        let socket_addresses = (target.hostname.as_str(), target.port)
            .to_socket_addrs()
            .map_err(Error::Connect)?
            .collect::<Vec<_>>();
        if socket_addresses.is_empty() {
            return Err(Error::NoAddress(target.address()));
        }
        let mut last_error = None;
        let mut tcp = None;
        for address in socket_addresses {
            match TcpStream::connect_timeout(&address, self.timeout) {
                Ok(stream) => {
                    tcp = Some(stream);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let tcp = tcp.ok_or_else(|| {
            Error::Connect(last_error.unwrap_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "no resolved address")
            }))
        })?;
        tcp.set_read_timeout(Some(self.timeout))
            .map_err(Error::Connect)?;
        tcp.set_write_timeout(Some(self.timeout))
            .map_err(Error::Connect)?;
        Ok(tcp)
    }

    fn tls_config(&self, credential: Option<&DeviceCredential>) -> Result<rustls::ClientConfig> {
        self.tls_config_material(
            credential
                .map(|credential| (&credential.seed, credential.certificate_chain.as_slice())),
        )
    }

    fn tls_config_material(
        &self,
        credential: Option<(&SecretSeed, &[Vec<u8>])>,
    ) -> Result<rustls::ClientConfig> {
        // Both ring and aws-lc can enter the workspace graph. Name the
        // provider so rustls never has to guess which process default to use.
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let builder = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_root_certificates(self.roots.clone());
        match credential {
            None => Ok(builder.with_no_client_auth()),
            Some((seed, certificate_chain)) => {
                if certificate_chain.is_empty() {
                    return Err(Error::CertificateChain);
                }
                let certificates = certificate_chain
                    .iter()
                    .cloned()
                    .map(CertificateDer::from)
                    .collect();
                let mut key = device_signing_key_pkcs8(seed)?;
                let key_bytes = std::mem::take(&mut *key);
                let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_bytes));
                Ok(builder.with_client_auth_cert(certificates, key)?)
            }
        }
    }

    fn connect_tls(
        &self,
        target: &ProbeTarget,
        tcp: TcpStream,
        config: rustls::ClientConfig,
    ) -> Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>> {
        let server_name =
            ServerName::try_from(target.hostname.clone()).map_err(|_| Error::ServerName)?;
        let connection = rustls::ClientConnection::new(Arc::new(config), server_name)?;
        Ok(rustls::StreamOwned::new(connection, tcp))
    }

    fn call(
        &self,
        target: &ProbeTarget,
        request: &[u8],
        credential: Option<&DeviceCredential>,
    ) -> Result<Vec<u8>> {
        let tcp = self.connect_tcp(target)?;
        let config = self.tls_config(credential)?;
        let mut tls = self.connect_tls(target, tcp, config)?;
        tls.write_all(request).map_err(foks_rpc::Error::Io)?;
        tls.flush().map_err(foks_rpc::Error::Io)?;
        read_response(&mut tls, self.maximum_frame_length, 0).map_err(Into::into)
    }

    fn call_with_material(
        &self,
        target: &ProbeTarget,
        request: &[u8],
        seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
    ) -> Result<Vec<u8>> {
        let tcp = self.connect_tcp(target)?;
        let config = self.tls_config_material(Some((seed, certificate_chain)))?;
        let mut tls = self.connect_tls(target, tcp, config)?;
        tls.write_all(request).map_err(foks_rpc::Error::Io)?;
        tls.flush().map_err(foks_rpc::Error::Io)?;
        read_response(&mut tls, self.maximum_frame_length, 0).map_err(Into::into)
    }

    /// Requests the X.509 certificate chain for an already enrolled device.
    /// This registration call is intentionally unauthenticated; possession of
    /// the matching Ed25519 private key is proved by the subsequent mTLS
    /// handshake.
    pub fn fetch_device_certificate_chain(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        seed: &SecretSeed,
    ) -> Result<Vec<Vec<u8>>> {
        if uid.entity_type() != ENTITY_USER {
            return Err(Error::DeviceBinding);
        }
        let device = derive_device_public(seed)?;
        let request = encode_get_client_cert_chain_request(uid.as_bytes(), device.id.as_bytes())?;
        let response = self.call(&host.registration, &request, None)?;
        let Value::Array(certificates) = decode(&response)? else {
            return Err(Error::CertificateChain);
        };
        let certificates = certificates
            .into_iter()
            .map(|certificate| match certificate {
                Value::Binary(bytes) if !bytes.is_empty() => Ok(bytes),
                _ => Err(Error::CertificateChain),
            })
            .collect::<Result<Vec<_>>>()?;
        if certificates.is_empty() {
            return Err(Error::CertificateChain);
        }
        Ok(certificates)
    }

    pub fn fetch_subkey_certificate_chain(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        subkey_seed: &SecretSeed,
    ) -> Result<Vec<Vec<u8>>> {
        if uid.entity_type() != ENTITY_USER {
            return Err(Error::DeviceBinding);
        }
        let subkey = derive_subkey_id(subkey_seed)?;
        let request = encode_get_client_cert_chain_request(uid.as_bytes(), subkey.as_bytes())?;
        let response = self.call(&host.registration, &request, None)?;
        let Value::Array(certificates) = decode(&response)? else {
            return Err(Error::CertificateChain);
        };
        let certificates = certificates
            .into_iter()
            .map(|certificate| match certificate {
                Value::Binary(bytes) if !bytes.is_empty() => Ok(bytes),
                _ => Err(Error::CertificateChain),
            })
            .collect::<Result<Vec<_>>>()?;
        if certificates.is_empty() {
            return Err(Error::CertificateChain);
        }
        Ok(certificates)
    }

    fn load_user_chain(&self, host: &PinnedHost, credential: &DeviceCredential) -> Result<Vec<u8>> {
        let request = encode_load_user_chain_request(credential.uid.as_bytes(), 1)?;
        self.call(&host.user, &request, Some(credential))
    }

    fn fetch_owner_puk_parcel(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<Vec<u8>> {
        let device = derive_device_public(&credential.seed)?;
        let request = encode_get_owner_puk_request(device.id.as_bytes())?;
        self.call(&host.user, &request, Some(credential))
    }

    pub fn advance_merkle_root(
        &self,
        pinned: &PinnedHost,
    ) -> Result<(Acceptance, VerifiedMerkleAdvance)> {
        let mut store = HardStateStore::open(&pinned.database_path)?;
        let host = store
            .host_for_lookup(&pinned.lookup_name)?
            .ok_or(Error::DeviceBinding)?;
        if host.host_id.as_slice() != pinned.host_id.as_bytes() {
            return Err(Error::DeviceBinding);
        }
        let latest_bytes = self.call(
            &pinned.merkle_query,
            &encode_get_current_merkle_root_request()?,
            None,
        )?;
        let latest = foks_proto::MerkleRoot::decode(&latest_bytes)?;
        let (full_epochs, hash_epochs) =
            merkle_history_requirements(latest.epoch, host.merkle_root.epoch)?;
        let historical_bytes = if full_epochs.is_empty() && hash_epochs.is_empty() {
            foks_snowpack::encode(&Value::Array(vec![Value::Null, Value::Null]))?
        } else {
            self.call(
                &pinned.merkle_query,
                &encode_get_historical_merkle_roots_request(&full_epochs, &hash_epochs)?,
                None,
            )?
        };
        let anchor = restore_merkle_anchor(
            host.merkle_root.epoch,
            host.merkle_root.root_hash,
            &host.merkle_root.root_bytes,
            &host.merkle_root.evidence,
            &host.merkle_root.authenticated_roots,
            &host.chain_bytes,
        )?;
        let verified = verify_merkle_advance(
            &anchor,
            &latest_bytes,
            &historical_bytes,
            &HostchainTail {
                seqno: host.chain_seqno,
                hash: host.chain_tail_hash,
            },
        )?;
        let acceptance = store.accept_verified_merkle_root(&host.host_id, verified.snapshot())?;
        Ok((acceptance, verified))
    }

    /// Advances the host's Merkle pin, authenticates with device mTLS, replays
    /// the user chain, unboxes the current owner PUK, and atomically advances
    /// public user hard state in SQLite.
    pub fn authenticate_and_pin(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<AuthenticatedUserOutcome> {
        let derived = derive_device_public(&credential.seed)?;
        let (merkle_acceptance, merkle) = self.advance_merkle_root(host)?;
        let chain_bytes = self.load_user_chain(host, credential)?;
        let verified = verify_user_chain(
            &chain_bytes,
            &credential.uid,
            &host.host_id,
            merkle.authenticated_roots(),
            &merkle.root().hostchain,
        )?;
        if !verified
            .devices()
            .iter()
            .any(|device| device.id == derived.id && device.hepk == derived.hepk)
        {
            return Err(Error::DeviceBinding);
        }
        let parcel_bytes = self.fetch_owner_puk_parcel(host, credential)?;
        let parcel = PukParcel::decode(&parcel_bytes)?;
        let sender = verified
            .devices()
            .iter()
            .find(|device| device.id == parcel.sender)
            .ok_or(Error::DeviceBinding)?;
        let owner_key = verified
            .shared_key(Role::OWNER)
            .ok_or(Error::DeviceBinding)?;
        let clear = open_puk_parcel(
            &parcel,
            &credential.seed,
            &sender.hepk,
            &owner_key.verify_key,
            &host.host_id,
        )?;
        let puk_seed = clear.into_seed();
        let mut store = HardStateStore::open(&host.database_path)?;
        let acceptance = store.accept_verified_user(&verified.hard_state_snapshot()?)?;
        Ok(AuthenticatedUserOutcome {
            merkle_acceptance,
            acceptance,
            verified,
            puk_seed,
        })
    }

    /// Authenticates a Yubi-backed device using its software subkey for mTLS
    /// and the hardware parent for PUK decapsulation.
    pub fn authenticate_yubi_and_pin(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
    ) -> Result<AuthenticatedUserOutcome> {
        let subkey = derive_subkey_id(&credential.subkey_seed)?;
        let (merkle_acceptance, merkle) = self.advance_merkle_root(host)?;
        let request = encode_load_user_chain_request(credential.uid.as_bytes(), 1)?;
        let chain_bytes = self.call_with_material(
            &host.user,
            &request,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        let verified = verify_user_chain(
            &chain_bytes,
            &credential.uid,
            &host.host_id,
            merkle.authenticated_roots(),
            &merkle.root().hostchain,
        )?;
        let parent = verified
            .devices()
            .iter()
            .find(|device| {
                device.id == *credential.parent.entity_id()
                    && device.hepk == *credential.parent.hepk()
                    && device.subkey.as_ref() == Some(&subkey)
            })
            .ok_or(Error::DeviceBinding)?;
        let request = encode_get_owner_puk_request(parent.id.as_bytes())?;
        let parcel_bytes = self.call_with_material(
            &host.user,
            &request,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        let parcel = PukParcel::decode(&parcel_bytes)?;
        let sender = verified
            .devices()
            .iter()
            .find(|device| device.id == parcel.sender)
            .ok_or(Error::DeviceBinding)?;
        let owner_key = verified
            .shared_key(Role::OWNER)
            .ok_or(Error::DeviceBinding)?;
        let clear = open_puk_parcel_with(
            &parcel,
            credential.parent,
            &sender.hepk,
            &owner_key.verify_key,
            &host.host_id,
        )?;
        let puk_seed = clear.into_seed();
        let mut store = HardStateStore::open(&host.database_path)?;
        let acceptance = store.accept_verified_user(&verified.hard_state_snapshot()?)?;
        Ok(AuthenticatedUserOutcome {
            merkle_acceptance,
            acceptance,
            verified,
            puk_seed,
        })
    }

    pub fn probe_and_pin(
        &self,
        target: &ProbeTarget,
        database_path: &Path,
    ) -> Result<ProbeOutcome> {
        let response = self.probe(target)?;
        let verified = verify_public_host(&target.hostname, &response)?;
        let mut store = HardStateStore::open(database_path)?;
        let acceptance = store.accept_verified_host(&verified.snapshot)?;
        drop(store);
        let pinned = self.pinned_host(&target.hostname, database_path)?;
        Ok(ProbeOutcome {
            acceptance,
            verified,
            pinned,
        })
    }

    /// Loads an authenticated host capability from durable hard state.
    pub fn pinned_host(&self, lookup_name: &str, database_path: &Path) -> Result<PinnedHost> {
        let store = HardStateStore::open(database_path)?;
        let snapshot = store
            .host_for_lookup(lookup_name)?
            .ok_or(Error::DeviceBinding)?;
        pinned_host_from_snapshot(snapshot, database_path)
    }

    /// Replays the exact persisted user transcript before returning durable
    /// user state to the caller.
    pub fn pinned_user(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
    ) -> Result<Option<VerifiedUserState>> {
        let store = HardStateStore::open(&host.database_path)?;
        let host_snapshot = store
            .host_for_lookup(&host.lookup_name)?
            .ok_or(Error::DeviceBinding)?;
        let Some(user) = store.user_for_host(host.host_id.as_bytes(), uid.as_bytes())? else {
            return Ok(None);
        };
        let anchor = restore_merkle_anchor(
            host_snapshot.merkle_root.epoch,
            host_snapshot.merkle_root.root_hash,
            &host_snapshot.merkle_root.root_bytes,
            &host_snapshot.merkle_root.evidence,
            &host_snapshot.merkle_root.authenticated_roots,
            &host_snapshot.chain_bytes,
        )?;
        let authenticated_roots = anchor.authenticated_root_set();
        let verified = restore_verified_user(
            user.parts(),
            &authenticated_roots,
            &host_snapshot.chain_bytes,
        )?;
        Ok(Some(verified))
    }
}

fn pinned_host_from_snapshot(snapshot: StoredHostSnapshot, path: &Path) -> Result<PinnedHost> {
    let identity = restore_public_host_identity(
        &snapshot.host_id,
        &snapshot.genesis_key,
        snapshot.chain_seqno,
        snapshot.chain_tail_hash,
        &snapshot.chain_bytes,
        &snapshot.public_zone_bytes,
    )?;
    restore_merkle_anchor(
        snapshot.merkle_root.epoch,
        snapshot.merkle_root.root_hash,
        &snapshot.merkle_root.root_bytes,
        &snapshot.merkle_root.evidence,
        &snapshot.merkle_root.authenticated_roots,
        &snapshot.chain_bytes,
    )?;
    if identity.canonical_name() != snapshot.canonical_name
        || identity.services() != snapshot.services
    {
        return Err(Error::DeviceBinding);
    }
    let registration = service_target(identity.services(), SERVICE_REG, "registration")?;
    let user = service_target(identity.services(), SERVICE_USER, "user")?;
    let merkle_query = service_target(identity.services(), SERVICE_MERKLE_QUERY, "Merkle query")?;
    let host_id = identity.host_id().clone();
    Ok(PinnedHost {
        lookup_name: snapshot.lookup_name,
        host_id,
        database_path: path.to_owned(),
        registration,
        user,
        merkle_query,
    })
}

fn service_target(
    services: &[HostService],
    service_type: u64,
    label: &'static str,
) -> Result<ProbeTarget> {
    let service = services
        .iter()
        .find(|service| service.service_type == service_type)
        .ok_or(Error::PinnedService(label))?;
    let Value::Text(endpoint) = decode(&service.endpoint_bytes)? else {
        return Err(Error::PinnedService(label));
    };
    let endpoint = std::str::from_utf8(&endpoint).map_err(|_| Error::PinnedService(label))?;
    ProbeTarget::parse(endpoint).map_err(|_| Error::PinnedService(label))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROBE: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
    );
    const USER_ROOT: &[u8] =
        include_bytes!("../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/merkle-root-998.snowp");
    const USER_HISTORY: &[u8] = include_bytes!(
        "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/merkle-historical-response.snowp"
    );
    const USER_CHAIN: &[u8] =
        include_bytes!("../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/user-chain.snowp");

    #[test]
    fn target_normalizes_name_and_defaults_port() {
        let target = ProbeTarget::parse("FOKS.APP.").unwrap();
        assert_eq!(target.hostname(), "foks.app");
        assert_eq!(target.port(), DEFAULT_PROBE_PORT);
        assert_eq!(target.address(), "foks.app:4430");
    }

    #[test]
    fn target_accepts_explicit_port() {
        let target = ProbeTarget::parse("localhost:9443").unwrap();
        assert_eq!(target.hostname(), "localhost");
        assert_eq!(target.port(), 9443);
    }

    #[test]
    fn target_rejects_ambiguous_or_invalid_names() {
        for target in ["", ".", "a..b", "-bad.test", "bad-.test", "bad name", "::1"] {
            assert!(ProbeTarget::parse(target).is_err(), "accepted {target:?}");
        }
    }

    #[test]
    fn pinned_capability_uses_only_authenticated_service_endpoints() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("hard.sqlite3");
        let verified = verify_public_host("foks.app", PROBE).unwrap();
        HardStateStore::open(&database)
            .unwrap()
            .accept_verified_host(&verified.snapshot)
            .unwrap();

        let client = PublicClient::webpki();
        let pinned = client.pinned_host("foks.app", &database).unwrap();
        assert_eq!(pinned.host_id.as_bytes(), verified.snapshot.host_id());
        assert_eq!(
            pinned.registration.address(),
            verified.public_zone.services.registration
        );
        assert_eq!(pinned.user.address(), verified.public_zone.services.user);
        assert_eq!(
            pinned.merkle_query.address(),
            verified.public_zone.services.merkle_query
        );
    }

    #[test]
    fn interrupted_user_sync_retries_from_durable_merkle_history() {
        let public = verify_public_host("foks.app", PROBE).unwrap();
        let advance = verify_merkle_advance(
            public.snapshot.merkle_root(),
            USER_ROOT,
            USER_HISTORY,
            &HostchainTail {
                seqno: public.snapshot.chain_seqno(),
                hash: public.snapshot.chain_tail_hash(),
            },
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("hard.sqlite3");
        let mut database = HardStateStore::open(&database_path).unwrap();
        database.accept_verified_host(&public.snapshot).unwrap();
        database
            .accept_verified_merkle_root(public.snapshot.host_id(), advance.snapshot())
            .unwrap();
        drop(database); // Simulate a failed user fetch followed by a fresh process.

        let database = HardStateStore::open(&database_path).unwrap();
        let pinned = database.host_for_lookup("foks.app").unwrap().unwrap();
        let anchor = restore_merkle_anchor(
            pinned.merkle_root.epoch,
            pinned.merkle_root.root_hash,
            &pinned.merkle_root.root_bytes,
            &pinned.merkle_root.evidence,
            &pinned.merkle_root.authenticated_roots,
            &pinned.chain_bytes,
        )
        .unwrap();
        let retry = verify_merkle_advance(
            &anchor,
            USER_ROOT,
            &foks_snowpack::encode(&Value::Array(vec![Value::Null, Value::Null])).unwrap(),
            &HostchainTail {
                seqno: pinned.chain_seqno,
                hash: pinned.chain_tail_hash,
            },
        )
        .unwrap();
        assert!(retry.authenticated_roots().contains_epoch(996));
        assert!(retry.authenticated_roots().contains_epoch(997));

        let Value::Binary(uid) = decode(include_bytes!(
            "../../foks-snowpack/tests/fixtures/foks-v0.1.9/user/uid.snowp"
        ))
        .unwrap() else {
            panic!("UID fixture is not a binary EntityID");
        };
        let uid = EntityId::from_bytes(uid).unwrap();
        let chain = foks_proto::UserChain::decode(USER_CHAIN).unwrap();
        let host = chain.links[0].decode_eldest().unwrap().host;
        verify_user_chain(
            USER_CHAIN,
            &uid,
            &host,
            retry.authenticated_roots(),
            &retry.root().hostchain,
        )
        .unwrap();
    }
}

#[cfg(test)]
#[path = "../tests/authenticated_user.rs"]
mod authenticated_user_tests;
