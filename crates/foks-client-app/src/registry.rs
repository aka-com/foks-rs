use super::*;
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    Probe,
    Signup,
    UserSync,
    Kv,
    DeviceAdministration,
    Recovery,
    Passphrases,
    Teams,
    Federation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "generation", rename_all = "kebab-case")]
pub enum ProtocolPolicy {
    /// The checked and fixture-pinned upstream v0.1.9 surface.
    V019,
    /// Current mainline or hosted service, public probing only.
    CurrentProbeOnly {
        canary_public_key: String,
        lease_url: String,
        last_artifact: Option<Box<SignedCanaryArtifact>>,
    },
    /// Current service after an external authenticated compatibility run.
    CurrentValidated {
        canary_public_key: String,
        lease_url: String,
        artifact: Box<SignedCanaryArtifact>,
    },
}

impl ProtocolPolicy {
    fn permits_at(&self, capability: Capability, now: u64) -> bool {
        capability == Capability::Probe
            || match self {
                Self::V019 => true,
                Self::CurrentProbeOnly { .. } => false,
                Self::CurrentValidated { artifact, .. } => {
                    now < artifact.artifact.expires_at
                        && artifact
                            .artifact
                            .capabilities
                            .contains(capability_canary_name(capability))
                }
            }
    }

    fn canary_public_key(&self) -> Option<&str> {
        match self {
            Self::V019 => None,
            Self::CurrentProbeOnly {
                canary_public_key, ..
            }
            | Self::CurrentValidated {
                canary_public_key, ..
            } => Some(canary_public_key),
        }
    }

    fn lease_url(&self) -> Option<&str> {
        match self {
            Self::V019 => None,
            Self::CurrentProbeOnly { lease_url, .. } | Self::CurrentValidated { lease_url, .. } => {
                Some(lease_url)
            }
        }
    }

    fn last_artifact(&self) -> Option<&SignedCanaryArtifact> {
        match self {
            Self::V019 => None,
            Self::CurrentProbeOnly { last_artifact, .. } => last_artifact.as_deref(),
            Self::CurrentValidated { artifact, .. } => Some(artifact),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TrustRoot {
    WebPki,
    CertificateDer { path: PathBuf },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Profile {
    pub name: String,
    pub probe: String,
    pub protocol: ProtocolPolicy,
    pub trust: TrustRoot,
}

impl Profile {
    pub fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;
        ProbeTarget::parse(&self.probe)?;
        if let Some(key) = self.protocol.canary_public_key() {
            let decoded = foks_compat_artifact::decode_public_key(key)
                .map_err(|_| Error::InvalidProfile("canary public key is invalid"))?;
            validate_lease_url(
                self.protocol
                    .lease_url()
                    .ok_or(Error::InvalidProfile("canary lease URL is missing"))?,
            )?;
            if let Some(artifact) = self.protocol.last_artifact() {
                verify_canary_for_target(artifact, &decoded, &self.probe)?;
            }
            match &self.protocol {
                ProtocolPolicy::CurrentValidated { artifact, .. }
                    if !canary_grants_this_client(artifact) =>
                {
                    return Err(Error::InvalidProfile(
                        "persisted canary lease is not a compatible grant for this target",
                    ));
                }
                ProtocolPolicy::CurrentProbeOnly {
                    last_artifact: Some(artifact),
                    ..
                } if canary_grants_this_client(artifact) => {
                    return Err(Error::InvalidProfile(
                        "compatible canary is persisted as probe-only",
                    ));
                }
                _ => {}
            }
        }
        if let TrustRoot::CertificateDer { path } = &self.trust {
            if path.as_os_str().is_empty() {
                return Err(Error::InvalidProfile("certificate path is empty"));
            }
        }
        Ok(())
    }

    pub fn require(&self, capability: Capability) -> Result<()> {
        self.require_at(capability, unix_seconds()?)
    }

    pub fn require_at(&self, capability: Capability, now: u64) -> Result<()> {
        self.validate()?;
        if self.protocol.permits_at(capability, now) {
            Ok(())
        } else {
            Err(Error::CapabilityDenied(capability))
        }
    }

    pub fn apply_canary(&self, signed: &SignedCanaryArtifact, now: u64) -> Result<Self> {
        self.validate()?;
        let public_key = self
            .protocol
            .canary_public_key()
            .ok_or(Error::InvalidProfile(
                "v0.1.9 profiles do not accept canaries",
            ))?;
        let decoded_key = foks_compat_artifact::decode_public_key(public_key)
            .map_err(|_| Error::InvalidProfile("canary public key is invalid"))?;
        signed
            .verify(&decoded_key)
            .map_err(|_| Error::InvalidProfile("canary signature is invalid"))?;
        if signed.artifact.target != self.probe
            || signed.artifact.generated_at > now.saturating_add(300)
            || signed.artifact.expires_at <= now
        {
            return Err(Error::InvalidProfile(
                "canary target or validity interval is invalid",
            ));
        }
        if let Some(previous) = self.protocol.last_artifact() {
            if signed.artifact.generation < previous.artifact.generation {
                return Err(Error::InvalidProfile("canary generation rolled back"));
            }
            if signed.artifact.generation == previous.artifact.generation {
                return if signed == previous {
                    Ok(self.clone())
                } else {
                    Err(Error::InvalidProfile("canary generation was reused"))
                };
            }
        }
        let canary_public_key = public_key.to_owned();
        let lease_url = self
            .protocol
            .lease_url()
            .ok_or(Error::InvalidProfile("canary lease URL is missing"))?
            .to_owned();
        let protocol = if canary_grants_this_client(signed) {
            ProtocolPolicy::CurrentValidated {
                canary_public_key,
                lease_url,
                artifact: Box::new(signed.clone()),
            }
        } else {
            ProtocolPolicy::CurrentProbeOnly {
                canary_public_key,
                lease_url,
                last_artifact: Some(Box::new(signed.clone())),
            }
        };
        let updated = Self {
            protocol,
            ..self.clone()
        };
        updated.validate()?;
        Ok(updated)
    }

    pub fn compatibility_lease_url(&self) -> Option<&str> {
        self.protocol.lease_url()
    }
}

fn verify_canary_for_target(
    signed: &SignedCanaryArtifact,
    public_key: &[u8; 32],
    target: &str,
) -> Result<()> {
    signed
        .verify(public_key)
        .map_err(|_| Error::InvalidProfile("canary signature is invalid"))?;
    if signed.artifact.target != target {
        return Err(Error::InvalidProfile(
            "persisted canary lease targets a different service",
        ));
    }
    Ok(())
}

fn canary_grants_this_client(signed: &SignedCanaryArtifact) -> bool {
    signed.artifact.outcome == CanaryOutcome::Compatible
        && signed.artifact.protocol_metadata_sha256 == PINNED_PROTOCOL_METADATA_SHA256
        && signed
            .artifact
            .capabilities
            .iter()
            .all(|capability| capability_from_canary(capability).is_ok())
}

fn validate_lease_url(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 2048 {
        return Err(Error::InvalidProfile("canary lease URL is invalid"));
    }
    let parsed =
        url::Url::parse(value).map_err(|_| Error::InvalidProfile("canary lease URL is invalid"))?;
    if parsed.scheme() != "https"
        || parsed.cannot_be_a_base()
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(Error::InvalidProfile(
            "canary lease URL must be an HTTPS URL without credentials, query, or fragment",
        ));
    }
    Ok(())
}

fn capability_from_canary(value: &str) -> Result<Capability> {
    match value {
        "signup" => Ok(Capability::Signup),
        "user-sync" => Ok(Capability::UserSync),
        "kv" => Ok(Capability::Kv),
        "device-administration" => Ok(Capability::DeviceAdministration),
        "recovery" => Ok(Capability::Recovery),
        "passphrases" => Ok(Capability::Passphrases),
        "teams" => Ok(Capability::Teams),
        "federation" => Ok(Capability::Federation),
        _ => Err(Error::InvalidProfile("canary grants an unknown capability")),
    }
}

fn capability_canary_name(capability: Capability) -> &'static str {
    match capability {
        Capability::Probe => "probe",
        Capability::Signup => "signup",
        Capability::UserSync => "user-sync",
        Capability::Kv => "kv",
        Capability::DeviceAdministration => "device-administration",
        Capability::Recovery => "recovery",
        Capability::Passphrases => "passphrases",
        Capability::Teams => "teams",
        Capability::Federation => "federation",
    }
}

fn unix_seconds() -> Result<u64> {
    use std::time::{SystemTime, UNIX_EPOCH};
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::InvalidConfig("system clock predates Unix epoch"))?
        .as_secs())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfilePaths {
    pub directory: PathBuf,
    pub hard_database: PathBuf,
    pub soft_database: PathBuf,
    pub protected_mutations: PathBuf,
    pub credential_store: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct RegistryFile {
    version: u32,
    profiles: BTreeMap<String, Profile>,
}

pub struct ProfileRegistry {
    root: PathBuf,
    profiles: BTreeMap<String, Profile>,
}

impl ProfileRegistry {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = prepare_private_directory(root.as_ref())?;
        let profiles = load_registry(&root)?;
        Ok(Self { root, profiles })
    }

    pub fn profiles(&self) -> impl ExactSizeIterator<Item = &Profile> {
        self.profiles.values()
    }

    pub fn profile(&self, name: &str) -> Result<&Profile> {
        self.profiles.get(name).ok_or(Error::ProfileMissing)
    }

    pub fn add(&mut self, profile: Profile) -> Result<()> {
        profile.validate()?;
        let _lock = RegistryMutationLock::acquire(&self.root)?;
        let current = load_registry(&self.root)?;
        if current.contains_key(&profile.name) {
            return Err(Error::ProfileExists);
        }
        let mut next = current;
        next.insert(profile.name.clone(), profile);
        self.save(&next)?;
        self.profiles = next;
        Ok(())
    }

    pub fn replace(&mut self, profile: Profile) -> Result<()> {
        profile.validate()?;
        let _lock = RegistryMutationLock::acquire(&self.root)?;
        let current = load_registry(&self.root)?;
        if !current.contains_key(&profile.name) {
            return Err(Error::ProfileMissing);
        }
        if current.get(&profile.name) != self.profiles.get(&profile.name) {
            return Err(Error::ProfileRegistryChanged);
        }
        let mut next = current;
        next.insert(profile.name.clone(), profile);
        self.save(&next)?;
        self.profiles = next;
        Ok(())
    }

    pub fn apply_canary(
        &mut self,
        name: &str,
        signed: &SignedCanaryArtifact,
        now: u64,
    ) -> Result<Profile> {
        let current = self.profile(name)?.clone();
        let updated = current.apply_canary(signed, now)?;
        if updated != current {
            self.replace(updated.clone())?;
        }
        Ok(updated)
    }

    pub fn remove(&mut self, name: &str) -> Result<bool> {
        validate_name(name)?;
        let _lock = RegistryMutationLock::acquire(&self.root)?;
        let mut next = load_registry(&self.root)?;
        let removed = next.remove(name).is_some();
        if removed {
            self.save(&next)?;
            self.profiles = next;
        }
        Ok(removed)
    }

    pub fn paths(&self, name: &str) -> Result<ProfilePaths> {
        validate_name(name)?;
        let directory = self.root.join("profiles").join(name);
        Ok(ProfilePaths {
            hard_database: directory.join("hard.sqlite3"),
            soft_database: directory.join("soft.sqlite3"),
            protected_mutations: directory.join("mutations"),
            credential_store: directory.join("credentials"),
            directory,
        })
    }

    pub fn prepare_profile_directory(&self, name: &str) -> Result<ProfilePaths> {
        let paths = self.paths(name)?;
        prepare_private_directory(&paths.directory)?;
        Ok(paths)
    }

    fn save(&self, profiles: &BTreeMap<String, Profile>) -> Result<()> {
        let bytes = toml::to_string_pretty(&RegistryFile {
            version: CONFIG_VERSION,
            profiles: profiles.clone(),
        })?;
        atomic_private_write(&self.root.join("profiles.toml"), bytes.as_bytes())
    }
}

struct RegistryMutationLock(File);

impl RegistryMutationLock {
    fn acquire(root: &Path) -> Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(root.join(REGISTRY_LOCK_FILE))?;
        fs2::FileExt::lock_exclusive(&file)?;
        Ok(Self(file))
    }
}

impl Drop for RegistryMutationLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

fn load_registry(root: &Path) -> Result<BTreeMap<String, Profile>> {
    let path = root.join("profiles.toml");
    let Some(bytes) = read_private_file_optional(&path, MAX_CONFIG_BYTES)? else {
        return Ok(BTreeMap::new());
    };
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(Error::InvalidConfig("profile registry is too large"));
    }
    let file: RegistryFile = toml::from_slice(&bytes)?;
    if file.version != CONFIG_VERSION {
        return Err(Error::InvalidConfig("unsupported profile registry version"));
    }
    for (name, profile) in &file.profiles {
        if name != &profile.name {
            return Err(Error::InvalidConfig("profile map key does not match name"));
        }
        profile.validate()?;
    }
    Ok(file.profiles)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProbeReport {
    pub lookup_name: String,
    pub canonical_name: String,
    pub host_id_hex: String,
    pub host_chain_sequence: u64,
    pub merkle_epoch: u64,
}

pub struct ProfileSession {
    pub(super) profile: Profile,
    pub(super) paths: ProfilePaths,
    pub(super) client: FoksClient,
}

impl ProfileSession {
    pub fn open(registry: &ProfileRegistry, name: &str) -> Result<Self> {
        let profile = registry.profile(name)?.clone();
        let paths = registry.prepare_profile_directory(name)?;
        let client = client_for_trust(&profile.trust)?;
        Ok(Self {
            profile,
            paths,
            client,
        })
    }

    pub fn open_with_control(
        registry: &ProfileRegistry,
        name: &str,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<Self> {
        if timeout.is_zero() {
            return Err(Error::InvalidConfig("zero profile operation timeout"));
        }
        let mut session = Self::open(registry, name)?;
        session.client.set_timeout(timeout);
        session.client = session.client.with_cancellation_token(cancellation);
        Ok(session)
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    pub fn paths(&self) -> &ProfilePaths {
        &self.paths
    }

    /// Opens another profile under the same operation controls. This is used
    /// only after that profile's own rollback checkpoint and operation lock
    /// are acquired by the caller.
    pub(crate) fn related_profile(&self, registry: &ProfileRegistry, name: &str) -> Result<Self> {
        let profile = registry.profile(name)?.clone();
        let paths = registry.prepare_profile_directory(name)?;
        Ok(Self {
            profile,
            paths,
            client: self.client.clone(),
        })
    }

    pub(super) fn rollback_checkpoint(&self) -> Result<RollbackCheckpoint> {
        rollback_checkpoint(self)
    }
}

/// A profile capability handle that only exists while its operation lock is
/// held and its external rollback checkpoint has been verified.
pub struct CheckedProfileSession<'a> {
    pub(super) session: &'a ProfileSession,
}

impl Deref for CheckedProfileSession<'_> {
    type Target = ProfileSession;

    fn deref(&self) -> &Self::Target {
        self.session
    }
}

fn rollback_checkpoint(session: &ProfileSession) -> Result<RollbackCheckpoint> {
    let target = ProbeTarget::parse(&session.profile.probe)?;
    let store = HardStateStore::open(&session.paths.hard_database)?;
    let metadata = store.metadata()?;
    let host = store
        .host_for_lookup(target.hostname())?
        .map(|snapshot| RollbackHostCheckpoint {
            host_id: snapshot.host_id,
            host_chain_sequence: snapshot.chain_seqno,
            host_chain_tail: snapshot.chain_tail_hash,
            host_chain_bytes: snapshot.chain_bytes,
            merkle_epoch: snapshot.merkle_root.epoch,
            merkle_root_hash: snapshot.merkle_root.root_hash,
            authenticated_roots: snapshot
                .merkle_root
                .authenticated_roots
                .iter()
                .map(|root| (root.epoch(), root.root_hash()))
                .collect(),
        });
    Ok(RollbackCheckpoint {
        profile: session.profile.name.clone(),
        database_id: metadata.database_id,
        hard_state_revision: metadata.revision,
        write_token: metadata.write_token,
        host,
    })
}

impl CheckedProfileSession<'_> {
    pub fn probe_and_pin(&self) -> Result<ProbeReport> {
        self.profile.require(Capability::Probe)?;
        let target = ProbeTarget::parse(&self.profile.probe)?;
        let outcome = self
            .client
            .probe_and_pin(&target, &self.paths.hard_database)?;
        Ok(ProbeReport {
            lookup_name: target.hostname().to_owned(),
            canonical_name: outcome.verified.snapshot.canonical_name().to_owned(),
            host_id_hex: hex(outcome.pinned.host_id().as_bytes()),
            host_chain_sequence: outcome.verified.snapshot.chain_seqno(),
            merkle_epoch: outcome.verified.snapshot.merkle_root().epoch(),
        })
    }

    pub fn pinned_host(&self) -> Result<foks_client::PinnedHost> {
        let target = ProbeTarget::parse(&self.profile.probe)?;
        self.client
            .pinned_host(target.hostname(), &self.paths.hard_database)
            .map_err(Into::into)
    }
}
