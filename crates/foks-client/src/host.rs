//! Host discovery, durable pin restoration, and persisted identity loading.

use super::{
    decode, restore_merkle_anchor, restore_public_host_identity, restore_verified_team,
    restore_verified_user, verify_public_host, Acceptance, CertificateDer, EntityId, Error,
    FoksClient, HardStateStore, HostService, Path, PathBuf, Result, ServiceType,
    StoredHostSnapshot, Value, VerifiedPublicHost, VerifiedTeamState, VerifiedUserState,
    DEFAULT_PROBE_PORT,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeTarget {
    pub(crate) hostname: String,
    pub(crate) port: u16,
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
    pub(crate) lookup_name: String,
    pub(crate) host_id: EntityId,
    pub(crate) database_path: PathBuf,
    pub(crate) registration: ProbeTarget,
    pub(crate) user: ProbeTarget,
    pub(crate) merkle_query: ProbeTarget,
    pub(crate) kv_store: ProbeTarget,
    pub(crate) tls_ca_certificates: Vec<Vec<u8>>,
}

impl PinnedHost {
    pub fn lookup_name(&self) -> &str {
        &self.lookup_name
    }

    pub fn host_id(&self) -> &EntityId {
        &self.host_id
    }

    /// Delegated CA certificates authenticated by this pinned host identity.
    pub fn tls_ca_certificates(&self) -> &[Vec<u8>] {
        &self.tls_ca_certificates
    }
}

impl FoksClient {
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
            .ok_or(Error::HostBinding("pinned host is missing"))?;
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
            .ok_or(Error::HostBinding("pinned host is missing"))?;
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

    /// Replays the exact persisted team transcript before exposing the public
    /// roster or PTK metadata.
    pub fn pinned_team(
        &self,
        host: &PinnedHost,
        team: &EntityId,
    ) -> Result<Option<VerifiedTeamState>> {
        let store = HardStateStore::open(&host.database_path)?;
        let host_snapshot = store
            .host_for_lookup(&host.lookup_name)?
            .ok_or(Error::HostBinding("pinned host is missing"))?;
        let Some(team) = store.team_for_host(host.host_id.as_bytes(), team.as_bytes())? else {
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
        Ok(Some(restore_verified_team(
            team.parts(),
            &anchor.authenticated_root_set(),
            &host_snapshot.chain_bytes,
        )?))
    }
}

pub(crate) fn authenticated_tls_roots(host: &PinnedHost) -> Result<rustls::RootCertStore> {
    let mut roots = rustls::RootCertStore::empty();
    for certificate in &host.tls_ca_certificates {
        roots
            .add(CertificateDer::from(certificate.clone()))
            .map_err(|_| Error::HostTlsRoots)?;
    }
    if roots.is_empty() {
        return Err(Error::HostTlsRoots);
    }
    Ok(roots)
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
        return Err(Error::HostBinding(
            "stored host projection does not match authenticated evidence",
        ));
    }
    let registration = service_target(
        identity.services(),
        ServiceType::Registration,
        "registration",
    )?;
    let user = service_target(identity.services(), ServiceType::User, "user")?;
    let merkle_query = service_target(
        identity.services(),
        ServiceType::MerkleQuery,
        "Merkle query",
    )?;
    let kv_store = service_target(identity.services(), ServiceType::KvStore, "KV store")?;
    let tls_ca_certificates = identity.tls_ca_certificates().to_vec();
    if tls_ca_certificates.is_empty() {
        return Err(Error::HostTlsRoots);
    }
    let host_id = identity.host_id().clone();
    Ok(PinnedHost {
        lookup_name: snapshot.lookup_name,
        host_id,
        database_path: path.to_owned(),
        registration,
        user,
        merkle_query,
        kv_store,
        tls_ca_certificates,
    })
}

fn service_target(
    services: &[HostService],
    service_type: ServiceType,
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
