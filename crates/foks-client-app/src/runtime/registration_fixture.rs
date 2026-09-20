use super::*;
use foks_keystore::SecretStore;
use zeroize::Zeroizing;

pub(super) struct Fixture {
    pub dir: tempfile::TempDir,
    pub registry: crate::ProfileRegistry,
    pub credentials: crate::ClientCredentials,
    pub session: crate::ProfileSession,
    pub host: foks_proto::EntityId,
}
impl Fixture {
    pub fn new(pin: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("state");
        let credentials =
            crate::ClientCredentials::initialize(&root, crate::CredentialBackend::PrivateFile)
                .unwrap();
        let mut registry = crate::ProfileRegistry::open(&root).unwrap();
        registry
            .add(crate::Profile {
                name: "local".into(),
                label: None,
                probe: "foks.app".into(),
                protocol: crate::ProtocolPolicy::V019,
                trust: crate::TrustRoot::WebPki,
            })
            .unwrap();
        let session = crate::ProfileSession::open(&registry, "local").unwrap();
        let verified = foks_verify::verify_public_host(
            "foks.app",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
            )),
        )
        .unwrap();
        if pin {
            credentials
                .with_checked_session(&session, |checked| {
                    HardStateStore::open(&checked.paths.hard_database)?
                        .accept_verified_host(&verified.snapshot)?;
                    Ok::<_, crate::Error>(())
                })
                .unwrap();
        }
        Self {
            dir,
            registry,
            credentials,
            session,
            host: foks_proto::EntityId::from_bytes(verified.snapshot.host_id().to_vec()).unwrap(),
        }
    }
}
pub(super) fn credential(tag: u8) -> foks_client::DeviceCredential {
    let mut uid = vec![tag; 33];
    uid[0] = foks_proto::ENTITY_USER;
    foks_client::DeviceCredential {
        key_kind: foks_client::SoftwareKeyKind::Device,
        uid: foks_proto::EntityId::from_bytes(uid).unwrap(),
        seed: foks_proto::SecretSeed::new([tag; 32]),
        certificate_chain: vec![vec![tag]],
    }
}

pub(super) struct CountingStore<S> {
    pub inner: S,
    pub scans: usize,
    pub reads: usize,
    pub account_reads: usize,
}
impl<S> CountingStore<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            scans: 0,
            reads: 0,
            account_reads: 0,
        }
    }
    pub fn reset(&mut self) {
        self.scans = 0;
        self.reads = 0;
        self.account_reads = 0;
    }
}
impl<S: SecretStore> SecretStore for CountingStore<S> {
    fn keys(&mut self) -> foks_keystore::Result<Vec<String>> {
        self.scans += 1;
        self.inner.keys()
    }
    fn get(&mut self, key: &str) -> foks_keystore::Result<Zeroizing<Vec<u8>>> {
        self.reads += 1;
        self.account_reads +=
            usize::from(key.starts_with("account.") || key.starts_with("yubi-account."));
        self.inner.get(key)
    }
    fn put(&mut self, key: &str, value: &[u8]) -> foks_keystore::Result<()> {
        self.inner.put(key, value)
    }
    fn remove(&mut self, key: &str) -> foks_keystore::Result<bool> {
        self.inner.remove(key)
    }
}
