use super::*;
use foks_proto::{ClientVersionExt, SemVer};

/// The pinned client protocol version this application speaks.
const PINNED_CLIENT_VERSION: SemVer = SemVer {
    major: 0,
    minor: 1,
    patch: 9,
};

const PROFILE_PUBLICATION_MARKER: &str = ".profile-publication-v1";
const PROFILE_STAGING_PREFIX: &str = ".pending-profile-";
/// Held for as long as a publication owns its staging directory. It lives
/// inside that directory and is unlinked before the directory is published, so
/// it never reaches a profile's committed state.
const PROFILE_PUBLICATION_LOCK: &str = ".publication-lock";
/// How many times an add re-reads the registry after waiting out a concurrent
/// publication of the same name. Each wait ends when that publication commits,
/// fails or dies, and a commit is then visible to the next read, so more than a
/// couple of rounds means something is republishing the name in a loop rather
/// than that this caller needs to keep waiting.
const PROFILE_PUBLICATION_ATTEMPTS: u32 = 8;
const PROFILE_PUBLICATION_MARKER_VERSION: u32 = 1;
const PROFILE_PUBLICATION_BINDING_TYPE_ID: u64 = 0x9171_dbed_137a_c227;

pub const PROFILE_LABEL_MAX_BYTES: usize = 64;

/// How long before a validated compatibility lease expires a periodic poll
/// starts refetching it. Compatibility artifacts are capped at a seven-day lifetime
/// (`foks_compat_artifact::CompatibilityArtifact::validate`), so six hours is a small
/// fraction of a full-length lease while still leaving many polling intervals
/// of retry budget before the stored lease lapses. Shorter leases use half
/// their own lifetime instead, so the margin never exceeds the lease.
pub const COMPATIBILITY_RENEWAL_MARGIN_SECONDS: u64 = 6 * 60 * 60;

fn validate_profile_label(label: Option<&str>) -> Result<()> {
    let Some(label) = label else {
        return Ok(());
    };
    if label.trim().is_empty() {
        return Err(Error::InvalidProfile("profile label is empty"));
    }
    if label != label.trim() {
        return Err(Error::InvalidProfile(
            "profile label has surrounding whitespace",
        ));
    }
    if label.len() > PROFILE_LABEL_MAX_BYTES {
        return Err(Error::InvalidProfile("profile label is too long"));
    }
    if label.contains(['\0', '\r', '\n']) {
        return Err(Error::InvalidProfile(
            "profile label contains a forbidden character",
        ));
    }
    Ok(())
}

pub fn normalize_profile_label(name: &str, label: Option<String>) -> Result<Option<String>> {
    validate_name(name)?;
    let label = match label {
        Some(label) => {
            if label.contains(['\0', '\r', '\n']) {
                return Err(Error::InvalidProfile(
                    "profile label contains a forbidden character",
                ));
            }
            let label = label.trim().to_owned();
            if label.is_empty() || label == name {
                None
            } else {
                Some(label)
            }
        }
        None => None,
    };
    validate_profile_label(label.as_deref())?;
    Ok(label)
}

#[cfg(test)]
static TEST_PROFILE_PUBLICATION_CRASH_POINT: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(0);
#[cfg(test)]
static TEST_PROFILE_PUBLICATION_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
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
    Chat,
    Federation,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        capability_artifact_name(self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "generation", rename_all = "kebab-case")]
pub enum ProtocolPolicy {
    /// The checked and fixture-pinned upstream v0.1.9 surface.
    V019,
    /// Current mainline or hosted service, public probing only.
    CurrentProbeOnly {
        compatibility_artifact_public_key: String,
        lease_url: String,
        last_artifact: Option<Box<SignedCompatibilityArtifact>>,
    },
    /// Current service after an external authenticated compatibility run.
    CurrentValidated {
        compatibility_artifact_public_key: String,
        lease_url: String,
        artifact: Box<SignedCompatibilityArtifact>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CompatibilityStatus {
    NotRequired,
    Missing,
    Incompatible {
        reason: CompatibilityFailure,
        expires_at: u64,
    },
    Validated {
        expires_at: u64,
        capabilities: std::collections::BTreeSet<Capability>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompatibilityFailure {
    Drift,
    ProtocolMismatch,
    UnknownCapability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityDenial {
    Missing,
    Incompatible,
    Expired,
    NotGranted,
}

impl CompatibilityStatus {
    pub fn denial_at(&self, capability: Capability, now: u64) -> Option<CapabilityDenial> {
        if capability == Capability::Probe {
            return None;
        }
        match self {
            Self::NotRequired => None,
            Self::Missing => Some(CapabilityDenial::Missing),
            Self::Incompatible { .. } => Some(CapabilityDenial::Incompatible),
            Self::Validated { expires_at, .. } if now >= *expires_at => {
                Some(CapabilityDenial::Expired)
            }
            Self::Validated { capabilities, .. } if !capabilities.contains(&capability) => {
                Some(CapabilityDenial::NotGranted)
            }
            Self::Validated { .. } => None,
        }
    }
}

impl ProtocolPolicy {
    pub fn compatibility_status(&self) -> CompatibilityStatus {
        match self {
            Self::V019 => CompatibilityStatus::NotRequired,
            Self::CurrentProbeOnly {
                last_artifact: None,
                ..
            } => CompatibilityStatus::Missing,
            Self::CurrentProbeOnly {
                last_artifact: Some(artifact),
                ..
            } => CompatibilityStatus::Incompatible {
                reason: if artifact.artifact.outcome == CompatibilityOutcome::Drift {
                    CompatibilityFailure::Drift
                } else if artifact.artifact.protocol_metadata_sha256
                    != PINNED_PROTOCOL_METADATA_SHA256
                {
                    CompatibilityFailure::ProtocolMismatch
                } else {
                    CompatibilityFailure::UnknownCapability
                },
                expires_at: artifact.artifact.expires_at,
            },
            Self::CurrentValidated { artifact, .. } => CompatibilityStatus::Validated {
                expires_at: artifact.artifact.expires_at,
                capabilities: artifact
                    .artifact
                    .capabilities
                    .iter()
                    .filter_map(|name| capability_from_artifact(name).ok())
                    .collect(),
            },
        }
    }

    /// Whether a periodic compatibility poll should fetch this profile's
    /// signed compatibility artifact at `now`.
    ///
    /// A validated lease is refetched only once its remaining lifetime falls
    /// inside [`COMPATIBILITY_RENEWAL_MARGIN_SECONDS`], or inside half the
    /// lease when the issuer published a lease shorter than twice that
    /// margin. Every other state -- no artifact yet, an artifact that does
    /// not grant this client, and an expired one -- keeps the caller's plain
    /// polling cadence, because such a profile still needs a usable lease.
    ///
    /// Skipping a fetch cannot widen what the profile may do: the capability
    /// check in [`CompatibilityStatus::denial_at`] fails closed at the stored
    /// `expires_at` whether or not anything polled. The one behavioural
    /// consequence is that a revocation republished mid-lease is observed up
    /// to one margin later than a poll on every tick would observe it.
    pub fn needs_lease_renewal_at(&self, now: u64) -> bool {
        match self {
            // No lease service is configured for this profile.
            Self::V019 => false,
            Self::CurrentProbeOnly { .. } => true,
            Self::CurrentValidated { artifact, .. } => {
                let artifact = &artifact.artifact;
                let lifetime = artifact.expires_at.saturating_sub(artifact.generated_at);
                let margin = COMPATIBILITY_RENEWAL_MARGIN_SECONDS.min(lifetime / 2);
                artifact.expires_at.saturating_sub(now) <= margin
            }
        }
    }

    fn permits_at(&self, capability: Capability, now: u64) -> bool {
        self.compatibility_status()
            .denial_at(capability, now)
            .is_none()
    }

    fn compatibility_artifact_public_key(&self) -> Option<&str> {
        match self {
            Self::V019 => None,
            Self::CurrentProbeOnly {
                compatibility_artifact_public_key,
                ..
            }
            | Self::CurrentValidated {
                compatibility_artifact_public_key,
                ..
            } => Some(compatibility_artifact_public_key),
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

    fn last_artifact(&self) -> Option<&SignedCompatibilityArtifact> {
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
    CertificateArtifact { sha256: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Profile {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub probe: String,
    pub protocol: ProtocolPolicy,
    pub trust: TrustRoot,
}

impl Profile {
    pub fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;
        validate_profile_label(self.label.as_deref())?;
        if self.label.as_deref() == Some(self.name.as_str()) {
            return Err(Error::InvalidProfile(
                "profile label duplicates the profile name",
            ));
        }
        ProbeTarget::parse(&self.probe)?;
        if let Some(key) = self.protocol.compatibility_artifact_public_key() {
            let decoded = foks_compat_artifact::decode_public_key(key).map_err(|_| {
                Error::InvalidProfile("compatibility artifact public key is invalid")
            })?;
            validate_lease_url(self.protocol.lease_url().ok_or(Error::InvalidProfile(
                "compatibility artifact URL is missing",
            ))?)?;
            if let Some(artifact) = self.protocol.last_artifact() {
                verify_compatibility_artifact_for_target(artifact, &decoded, &self.probe)?;
            }
            match &self.protocol {
                ProtocolPolicy::CurrentValidated { artifact, .. }
                    if !artifact_grants_this_client(artifact) =>
                {
                    return Err(Error::InvalidProfile(
                        "persisted compatibility artifact is not a compatible grant for this target",
                    ));
                }
                ProtocolPolicy::CurrentProbeOnly {
                    last_artifact: Some(artifact),
                    ..
                } if artifact_grants_this_client(artifact) => {
                    return Err(Error::InvalidProfile(
                        "compatible artifact is persisted as probe-only",
                    ));
                }
                _ => {}
            }
        }
        if let TrustRoot::CertificateArtifact { sha256 } = &self.trust {
            crate::portability::trust::validate_digest(sha256)?;
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

    pub fn apply_compatibility_artifact(
        &self,
        signed: &SignedCompatibilityArtifact,
        now: u64,
    ) -> Result<Self> {
        self.validate()?;
        let public_key =
            self.protocol
                .compatibility_artifact_public_key()
                .ok_or(Error::InvalidProfile(
                    "v0.1.9 profiles do not accept compatibility artifacts",
                ))?;
        let decoded_key = foks_compat_artifact::decode_public_key(public_key)
            .map_err(|_| Error::InvalidProfile("compatibility artifact public key is invalid"))?;
        signed
            .verify(&decoded_key)
            .map_err(|_| Error::InvalidProfile("compatibility artifact signature is invalid"))?;
        if signed.artifact.target != self.probe
            || signed.artifact.generated_at > now.saturating_add(300)
            || signed.artifact.expires_at <= now
        {
            return Err(Error::InvalidProfile(
                "compatibility artifact target or validity interval is invalid",
            ));
        }
        if let Some(previous) = self.protocol.last_artifact() {
            if signed.artifact.generation < previous.artifact.generation {
                return Err(Error::InvalidProfile(
                    "compatibility artifact generation rolled back",
                ));
            }
            if signed.artifact.generation == previous.artifact.generation {
                return if signed == previous {
                    Ok(self.clone())
                } else {
                    Err(Error::InvalidProfile(
                        "compatibility artifact generation was reused",
                    ))
                };
            }
        }
        let compatibility_artifact_public_key = public_key.to_owned();
        let lease_url = self
            .protocol
            .lease_url()
            .ok_or(Error::InvalidProfile(
                "compatibility artifact URL is missing",
            ))?
            .to_owned();
        let protocol = if artifact_grants_this_client(signed) {
            ProtocolPolicy::CurrentValidated {
                compatibility_artifact_public_key,
                lease_url,
                artifact: Box::new(signed.clone()),
            }
        } else {
            ProtocolPolicy::CurrentProbeOnly {
                compatibility_artifact_public_key,
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

fn verify_compatibility_artifact_for_target(
    signed: &SignedCompatibilityArtifact,
    public_key: &[u8; 32],
    target: &str,
) -> Result<()> {
    signed
        .verify(public_key)
        .map_err(|_| Error::InvalidProfile("compatibility artifact signature is invalid"))?;
    if signed.artifact.target != target {
        return Err(Error::InvalidProfile(
            "persisted compatibility artifact targets a different service",
        ));
    }
    Ok(())
}

fn artifact_grants_this_client(signed: &SignedCompatibilityArtifact) -> bool {
    signed.artifact.outcome == CompatibilityOutcome::Compatible
        && signed.artifact.protocol_metadata_sha256 == PINNED_PROTOCOL_METADATA_SHA256
        && signed
            .artifact
            .capabilities
            .iter()
            .all(|capability| capability_from_artifact(capability).is_ok())
}

fn validate_lease_url(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 2048 {
        return Err(Error::InvalidProfile(
            "compatibility artifact URL is invalid",
        ));
    }
    let parsed = url::Url::parse(value)
        .map_err(|_| Error::InvalidProfile("compatibility artifact URL is invalid"))?;
    if parsed.scheme() != "https"
        || parsed.cannot_be_a_base()
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(Error::InvalidProfile(
            "compatibility artifact URL must be an HTTPS URL without credentials, query, or fragment",
        ));
    }
    Ok(())
}

fn capability_from_artifact(value: &str) -> Result<Capability> {
    match value {
        "signup" => Ok(Capability::Signup),
        "user-sync" => Ok(Capability::UserSync),
        "kv" => Ok(Capability::Kv),
        "device-administration" => Ok(Capability::DeviceAdministration),
        "recovery" => Ok(Capability::Recovery),
        "passphrases" => Ok(Capability::Passphrases),
        "teams" => Ok(Capability::Teams),
        "chat" => Ok(Capability::Chat),
        "federation" => Ok(Capability::Federation),
        _ => Err(Error::InvalidProfile(
            "compatibility artifact grants an unknown capability",
        )),
    }
}

fn capability_artifact_name(capability: Capability) -> &'static str {
    match capability {
        Capability::Probe => "probe",
        Capability::Signup => "signup",
        Capability::UserSync => "user-sync",
        Capability::Kv => "kv",
        Capability::DeviceAdministration => "device-administration",
        Capability::Recovery => "recovery",
        Capability::Passphrases => "passphrases",
        Capability::Teams => "teams",
        Capability::Chat => "chat",
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

pub struct HostedLeaseRenewal {
    lease: ClientStateLease,
    profile: Profile,
    config: File,
}

impl HostedLeaseRenewal {
    pub fn url(&self) -> &str {
        self.profile.compatibility_lease_url().unwrap_or_default()
    }

    /// Whether `signed` is the still-valid artifact this profile already
    /// stores as its validated lease. [`Self::apply`] would return
    /// `(false, Validated { .. })` for it without writing anything, so a
    /// caller may report "unchanged" directly instead of taking a
    /// profile-scoped admission to re-derive that answer.
    ///
    /// The expiry is part of the test because `Profile::apply_compatibility_artifact`
    /// rejects an artifact that has already lapsed, even one it stores
    /// itself; short-circuiting such an artifact would turn that rejection
    /// into a success.
    pub fn matches_stored_validated_artifact(
        &self,
        signed: &SignedCompatibilityArtifact,
        now: u64,
    ) -> bool {
        matches!(
            &self.profile.protocol,
            ProtocolPolicy::CurrentValidated { artifact, .. }
                if artifact.as_ref() == signed && now < signed.artifact.expires_at
        )
    }

    pub fn apply(
        self,
        signed: &SignedCompatibilityArtifact,
        now: u64,
    ) -> Result<(bool, CompatibilityStatus)> {
        self.lease.validate()?;
        let root = self.lease.root();
        let _lock = RegistryMutationLock::acquire_with_wait(root, false)?;
        let current_config = fs::symlink_metadata(root.join("profiles.toml"))?;
        let expected_config = self.config.metadata()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if current_config.dev() != expected_config.dev()
                || current_config.ino() != expected_config.ino()
            {
                return Err(Error::ProfileRegistryChanged);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (current_config, expected_config);
            return Err(Error::PortabilityUnsupported);
        }
        let mut profiles = load_registry(root)?;
        if profiles.get(&self.profile.name) != Some(&self.profile) {
            return Err(Error::ProfileRegistryChanged);
        }
        let updated = self.profile.apply_compatibility_artifact(signed, now)?;
        let changed = updated != self.profile;
        let status = updated.protocol.compatibility_status();
        if changed {
            profiles.insert(updated.name.clone(), updated);
            save_registry(root, &profiles)?;
        }
        Ok((changed, status))
    }
}

pub struct ProfileRegistry {
    lease: ClientStateLease,
    root: PathBuf,
    profiles: BTreeMap<String, Profile>,
}

trait ProfilePublicationAuthorizer {
    fn begin(&self, profile: &str) -> Result<[u8; 32]>;
    fn bind(&self, profile: &str, authorization: &[u8; 32], marker_digest: &[u8; 32])
        -> Result<()>;
    fn cancel(&self, profile: &str, authorization: &[u8; 32]) -> Result<()>;
}

impl ProfilePublicationAuthorizer for ClientCredentials {
    fn begin(&self, profile: &str) -> Result<[u8; 32]> {
        self.begin_profile_publication(profile)
    }

    fn bind(
        &self,
        profile: &str,
        authorization: &[u8; 32],
        marker_digest: &[u8; 32],
    ) -> Result<()> {
        self.bind_profile_publication(profile, authorization, marker_digest)
    }

    fn cancel(&self, profile: &str, authorization: &[u8; 32]) -> Result<()> {
        self.cancel_profile_publication(profile, authorization)
    }
}

impl ProfileRegistry {
    pub fn try_open(root: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_wait(root.as_ref(), false)
    }

    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_wait(root.as_ref(), true)
    }

    /// Opens the registry under a *shared* lock, which every reader can hold at
    /// once, and upgrades to the exclusive lock only for the uncommon open that
    /// actually has interrupted-publication state to clean up.
    ///
    /// The shared hold still excludes every registry writer, so `profiles.toml`
    /// cannot be replaced under the parse and no profile state directory can be
    /// created, renamed or removed under the scan. It no longer excludes
    /// another reader running the same parse and scan, which is the point: two
    /// agent requests on different profiles no longer serialize here.
    fn open_with_wait(root: &Path, wait: bool) -> Result<Self> {
        let lease = ClientStateLease::acquire(root)?;
        let root = lease.root().to_owned();
        let lock = RegistryMutationLock::acquire_shared_with_wait(&root, wait)?;
        let profiles = load_registry(&root)?;
        let profiles = if scan_profile_publications(&root, &profiles, ScanMode::Observe)? {
            // The observing scan held no lock against a concurrent recovery by
            // another reader, so nothing it saw can be acted on directly. Drop
            // the shared hold, take the exclusive one, and redo both the parse
            // and the scan against whatever state that lock now protects.
            drop(lock);
            let _lock = RegistryMutationLock::acquire_with_wait(&root, wait)?;
            let profiles = load_registry(&root)?;
            scan_profile_publications(&root, &profiles, ScanMode::Recover)?;
            profiles
        } else {
            profiles
        };
        Ok(Self {
            root,
            profiles,
            lease,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn profiles(&self) -> impl ExactSizeIterator<Item = &Profile> {
        self.profiles.values()
    }

    pub fn profile(&self, name: &str) -> Result<&Profile> {
        self.profiles.get(name).ok_or(Error::ProfileMissing)
    }

    pub fn add(&mut self, profile: Profile) -> Result<()> {
        profile.validate()?;
        self.lease.validate()?;
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

    /// Checks a server into an isolated hard-state database and publishes the
    /// profile only after its authenticated host pin is durable. A protected
    /// one-time authorization and checkpoint-bound marker let the next checked
    /// use safely finish a publication interrupted by process termination.
    fn check_and_add_profile_with_control<A: ProfilePublicationAuthorizer>(
        &mut self,
        authorizer: &A,
        profile: Profile,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<ProfilePublicationReport> {
        self.check_and_add_profile_for_host_with_control(
            authorizer,
            profile,
            timeout,
            cancellation,
            None,
        )
    }

    /// Publishes `profile` without holding the registry lock across the network
    /// probe.
    ///
    /// The lock is taken three times instead of once: to read the registry and
    /// reserve a staging directory, then -- after the probe, with no lock held
    /// -- to re-verify and commit, and finally to clean a failure up. The
    /// staging reservation is what makes the unlocked window safe: it excludes
    /// another publication of the same name, and it stops a concurrent registry
    /// open from recovering a staging directory that is still in use.
    fn check_and_add_profile_for_host_with_control<A: ProfilePublicationAuthorizer>(
        &mut self,
        authorizer: &A,
        profile: Profile,
        timeout: Duration,
        cancellation: CancellationToken,
        expected_host: Option<&[u8; 33]>,
    ) -> Result<ProfilePublicationReport> {
        profile.validate()?;
        if timeout.is_zero() {
            return Err(Error::InvalidConfig("zero profile operation timeout"));
        }
        self.lease.validate()?;
        let profiles_directory = self.root.join("profiles");
        let mut attempts = 0;
        let staging = loop {
            let reserved = {
                let _lock = RegistryMutationLock::acquire(&self.root)?;
                let current = load_registry(&self.root)?;
                scan_profile_publications(&self.root, &current, ScanMode::Recover)?;
                if let Some(report) =
                    committed_publication(&self.root, &current, &profile, expected_host)?
                {
                    return Ok(report);
                }
                let profiles_directory = prepare_private_directory(&profiles_directory)?;
                if profiles_directory.join(&profile.name).exists() {
                    return Err(Error::InvalidConfig(
                        "unpublished profile state directory already exists",
                    ));
                }
                PublicationStaging::reserve(&profiles_directory, &profile.name)?
            };
            if let Some(staging) = reserved {
                break staging;
            }
            // Another publication of this name is between its probe and its
            // commit. Waiting for it requires releasing the registry lock,
            // which the owner needs to commit; re-read everything afterwards,
            // since by then it may have published the profile.
            attempts += 1;
            if attempts == PROFILE_PUBLICATION_ATTEMPTS {
                return Err(Error::ProfileRegistryChanged);
            }
            PublicationStaging::wait_for_release(&profiles_directory, &profile.name)?;
        };
        self.publish_checked_profile(
            authorizer,
            profile,
            timeout,
            cancellation,
            expected_host,
            staging,
        )
    }

    /// Probes the server with no registry lock held, then re-acquires it to
    /// commit. `staging` is held throughout, so nothing else recovers or
    /// republishes this name in between.
    fn publish_checked_profile<A: ProfilePublicationAuthorizer>(
        &mut self,
        authorizer: &A,
        profile: Profile,
        timeout: Duration,
        cancellation: CancellationToken,
        expected_host: Option<&[u8; 33]>,
        mut staging: PublicationStaging,
    ) -> Result<ProfilePublicationReport> {
        // The reservation created its directory under the prepared, canonical
        // profiles directory, so publishing is a rename between siblings.
        let staging_directory = staging.directory.clone();
        let profiles_directory = staging_directory
            .parent()
            .ok_or(Error::InvalidConfig("profile publication has no parent"))?
            .to_owned();
        let final_directory = profiles_directory.join(&profile.name);
        let started = create_private_config(
            &staging_directory.join(PROFILE_PUBLICATION_MARKER),
            b"profile-publication-v1",
        )
        .and_then(|()| authorizer.begin(&profile.name));
        let authorization = match started {
            Ok(authorization) => authorization,
            Err(error) => {
                // Nothing is published yet and the reservation is still held,
                // so this directory is provably this publication's own; the
                // registry lock keeps the removal from racing a concurrent
                // open's scan of the same directory.
                let _lock = RegistryMutationLock::acquire(&self.root)?;
                staging.release()?;
                remove_profile_publication_directory(&staging_directory)?;
                return Err(error);
            }
        };

        let crash_was_injected = std::cell::Cell::new(false);
        let registry_was_committed = std::cell::Cell::new(false);
        let publication = (|| {
            let paths = ProfilePaths::for_directory(staging_directory.clone());
            let mut client = client_for_trust(&profile.trust, &self.root)?;
            client.set_timeout(timeout);
            let client = client.with_cancellation_token(cancellation);
            let session = ProfileSession {
                lease: self.lease.clone(),
                profile: profile.clone(),
                paths,
                client,
                adapter_clock: std::sync::Arc::new(crate::SystemAdapterClock),
                read_caches: crate::ReadCaches::default(),
                pairing_budget: None,
            };
            let probe = session.probe_and_pin_unchecked()?;
            if expected_host.is_some_and(|host| probe.host_id_hex != hex(host)) {
                return Err(Error::InvalidProfile(
                    "checked server does not match the expected host",
                ));
            }

            #[cfg(test)]
            if TEST_PROFILE_PUBLICATION_CRASH_POINT
                .compare_exchange(
                    1,
                    0,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                )
                .is_ok()
            {
                crash_was_injected.set(true);
                return Err(Error::InvalidConfig(
                    "test interrupted profile publication after probe",
                ));
            }

            let marker_digest = write_profile_publication_marker(
                &staging_directory.join(PROFILE_PUBLICATION_MARKER),
                &profile,
                &probe,
                &session.rollback_checkpoint()?,
                authorization,
            )?;
            authorizer.bind(&profile.name, &authorization, &marker_digest)?;

            // Everything above ran without the registry lock. Re-take it and
            // re-verify what the reservation does not already cover: the name
            // must still be unpublished, and its published directory must still
            // be absent. Other profiles may have changed freely, so the commit
            // merges into the registry as it is now rather than into the copy
            // read before the probe.
            let _lock = RegistryMutationLock::acquire(&self.root)?;
            let latest = load_registry(&self.root)?;
            if latest.contains_key(&profile.name) {
                return Err(Error::ProfileRegistryChanged);
            }
            if final_directory.exists() {
                return Err(Error::InvalidConfig(
                    "unpublished profile state directory already exists",
                ));
            }
            staging.release()?;
            fs::rename(&staging_directory, &final_directory)?;
            File::open(&profiles_directory)?.sync_all()?;

            #[cfg(test)]
            if TEST_PROFILE_PUBLICATION_CRASH_POINT
                .compare_exchange(
                    2,
                    0,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                )
                .is_ok()
            {
                crash_was_injected.set(true);
                return Err(Error::InvalidConfig(
                    "test interrupted profile publication before registry commit",
                ));
            }

            let mut next = latest;
            next.insert(profile.name.clone(), profile.clone());
            self.save(&next)?;
            self.profiles = next;
            registry_was_committed.set(true);

            #[cfg(test)]
            if TEST_PROFILE_PUBLICATION_CRASH_POINT
                .compare_exchange(
                    3,
                    0,
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                )
                .is_ok()
            {
                crash_was_injected.set(true);
                return Err(Error::InvalidConfig(
                    "test interrupted profile publication after registry commit",
                ));
            }

            Ok(ProfilePublicationReport {
                profile: profile.clone(),
                probe,
            })
        })();

        if publication.is_err() && !registry_was_committed.get() && !crash_was_injected.get() {
            // Ordinary failures are cleaned immediately. Crash-injection
            // paths intentionally retain the marker so reopen recovery is
            // exercised by tests.
            //
            // The removal takes the registry lock so it cannot race a
            // concurrent open's scan of the same directory; the closure above
            // has already dropped its own hold on every path that reaches here.
            let lock = RegistryMutationLock::acquire(&self.root)?;
            if staging.holds_reservation() {
                // Nothing was published, and the reservation still proves this
                // directory belongs to this publication.
                staging.release()?;
                if staging_directory.exists() {
                    remove_profile_publication_directory(&staging_directory)?;
                }
            } else {
                // The reservation was released for the rename, so between then
                // and now a concurrent open may have recovered the directory
                // and another publication of this name may have taken the path.
                // Only the one-time authorization in the marker still
                // identifies this publication's own directory.
                for pending in [&final_directory, &staging_directory] {
                    if publication_is_authorized_by(pending, &authorization) {
                        remove_profile_publication_directory(pending)?;
                        break;
                    }
                }
            }
            drop(lock);
            // A stale cancel is a no-op: the stored authorization must still
            // begin with this one for the record to be removed.
            authorizer.cancel(&profile.name, &authorization)?;
        }
        publication
    }

    pub fn replace(&mut self, profile: Profile) -> Result<()> {
        profile.validate()?;
        self.lease.validate()?;
        let _lock = RegistryMutationLock::acquire(&self.root)?;
        let current = load_registry(&self.root)?;
        if !current.contains_key(&profile.name) {
            return Err(Error::ProfileMissing);
        }
        if current.get(&profile.name) != self.profiles.get(&profile.name) {
            return Err(Error::ProfileRegistryChanged);
        }
        if profile_publication_marker_exists(&self.root, &profile.name)? {
            return Err(Error::InvalidConfig(
                "profile publication checkpoint is still pending",
            ));
        }
        let mut next = current;
        next.insert(profile.name.clone(), profile);
        self.save(&next)?;
        self.profiles = next;
        Ok(())
    }

    pub fn set_label(&mut self, name: &str, label: Option<String>) -> Result<bool> {
        let current = self.profile(name)?.clone();
        let label = normalize_profile_label(name, label)?;
        if current.label == label {
            return Ok(false);
        }
        let mut updated = current;
        updated.label = label;
        self.replace(updated)?;
        Ok(true)
    }

    pub fn hosted_lease_renewal(&self, name: &str) -> Result<Option<HostedLeaseRenewal>> {
        self.lease.validate()?;
        let _lock = RegistryMutationLock::acquire_with_wait(&self.root, false)?;
        let profiles = load_registry(&self.root)?;
        let profile = profiles.get(name).ok_or(Error::ProfileMissing)?.clone();
        if profile.compatibility_lease_url().is_none() {
            return Ok(None);
        }
        let config = File::open(self.root.join("profiles.toml"))?;
        Ok(Some(HostedLeaseRenewal {
            lease: self.lease.clone(),
            profile,
            config,
        }))
    }

    pub fn apply_compatibility_artifact(
        &mut self,
        name: &str,
        signed: &SignedCompatibilityArtifact,
        now: u64,
    ) -> Result<Profile> {
        let current = self.profile(name)?.clone();
        let updated = current.apply_compatibility_artifact(signed, now)?;
        if updated != current {
            self.replace(updated.clone())?;
        }
        Ok(updated)
    }

    pub fn remove(&mut self, name: &str) -> Result<bool> {
        validate_name(name)?;
        self.lease.validate()?;
        let _lock = RegistryMutationLock::acquire(&self.root)?;
        if profile_publication_marker_exists(&self.root, name)? {
            return Err(Error::InvalidConfig(
                "profile publication checkpoint is still pending",
            ));
        }
        let mut next = load_registry(&self.root)?;
        let removed = next.remove(name).is_some();
        if removed {
            self.save(&next)?;
            self.profiles = next;
        }
        Ok(removed)
    }

    pub fn paths(&self, name: &str) -> Result<ProfilePaths> {
        self.lease.validate()?;
        validate_name(name)?;
        let directory = self.root.join("profiles").join(name);
        Ok(ProfilePaths::for_directory(directory))
    }

    pub fn prepare_profile_directory(&self, name: &str) -> Result<ProfilePaths> {
        let paths = self.paths(name)?;
        prepare_private_directory(&paths.directory)?;
        Ok(paths)
    }

    fn save(&self, profiles: &BTreeMap<String, Profile>) -> Result<()> {
        save_registry(&self.root, profiles)
    }
}

pub(crate) fn save_registry(root: &Path, profiles: &BTreeMap<String, Profile>) -> Result<()> {
    let bytes = toml::to_string_pretty(&RegistryFile {
        version: CONFIG_VERSION,
        profiles: profiles.clone(),
    })?;
    atomic_private_write(&root.join("profiles.toml"), bytes.as_bytes())
}

impl ProfilePaths {
    pub(crate) fn for_directory(directory: PathBuf) -> Self {
        Self {
            hard_database: directory.join("hard.sqlite3"),
            soft_database: directory.join("soft.sqlite3"),
            protected_mutations: directory.join("mutations"),
            credential_store: directory.join("credentials"),
            directory,
        }
    }
}

/// What a publication scan may do about what it finds.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ScanMode {
    /// Report whether recovery is needed and touch nothing. Safe to run while
    /// the registry lock is held shared.
    Observe,
    /// Perform the recovery. The caller must hold the registry lock
    /// exclusively.
    Recover,
}

/// Validates every profile state directory and, in [`ScanMode::Recover`],
/// removes what an interrupted publication left behind. Returns whether any
/// removal was, or would have been, needed.
///
/// Both modes run the same validation and report the same errors, so an open
/// that never upgrades to the exclusive lock still rejects exactly the unsafe
/// states an exclusive open rejects. Only the removals are withheld.
fn scan_profile_publications(
    root: &Path,
    profiles: &BTreeMap<String, Profile>,
    mode: ScanMode,
) -> Result<bool> {
    let directory = root.join("profiles");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let mut recovery_needed = false;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or(Error::InvalidConfig("profile state name is not UTF-8"))?;
        let path = entry.path();
        // A publication that fails cleans its own staging directory up without
        // the registry lock, so an entry can disappear between the directory
        // read and this stat. Its absence is the state the scan wants anyway.
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if name.starts_with(PROFILE_STAGING_PREFIX) {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(Error::InvalidConfig(
                    "profile publication staging path is unsafe",
                ));
            }
            if PublicationStaging::is_live(&path)? {
                // A running publication owns this directory. Removing it would
                // destroy an add that is only waiting on its network probe.
                continue;
            }
            recovery_needed = true;
            if mode == ScanMode::Recover {
                remove_profile_publication_directory(&path)?;
            }
            continue;
        }
        let marker = path.join(PROFILE_PUBLICATION_MARKER);
        if read_private_file_optional(&marker, 16 * 1024)?.is_none() {
            continue;
        }
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(Error::InvalidConfig("profile publication path is unsafe"));
        }
        let (publication, _) = read_profile_publication_marker(&marker)?.ok_or(
            Error::InvalidConfig("profile publication marker disappeared during recovery"),
        )?;
        if publication.checkpoint.profile != name || publication.profile.name != name {
            return Err(Error::InvalidConfig(
                "profile publication marker binding changed",
            ));
        }
        match profiles.get(name) {
            Some(profile) if profile == &publication.profile => {}
            Some(_) => {
                return Err(Error::InvalidConfig(
                    "profile publication registry binding changed",
                ));
            }
            None => {
                recovery_needed = true;
                if mode == ScanMode::Recover {
                    remove_profile_publication_directory(&path)?;
                }
            }
        }
    }
    Ok(recovery_needed)
}

#[derive(Deserialize, Serialize)]
struct ProfilePublicationMarker {
    version: u32,
    profile: Profile,
    probe: ProbeReport,
    checkpoint: RollbackCheckpoint,
    authorization: [u8; 32],
}

fn read_profile_publication_marker(
    path: &Path,
) -> Result<Option<(ProfilePublicationMarker, [u8; 32])>> {
    let Some(contents) = read_private_file_optional(path, 16 * 1024)? else {
        return Ok(None);
    };
    let marker: ProfilePublicationMarker = serde_json::from_slice(&contents)
        .map_err(|_| Error::InvalidConfig("profile publication marker is malformed"))?;
    if marker.version != PROFILE_PUBLICATION_MARKER_VERSION {
        return Err(Error::InvalidConfig(
            "profile publication marker version is unsupported",
        ));
    }
    Ok(Some((
        marker,
        prefixed_hash(PROFILE_PUBLICATION_BINDING_TYPE_ID, &contents),
    )))
}

/// The report for a publication that is already committed to the registry but
/// whose marker has not been consumed yet, if `profile` is such a publication.
/// `Err(ProfileExists)` for any other already-registered profile of that name.
///
/// The caller must hold the registry lock, since this decides whether to start
/// a new publication of the same name.
fn committed_publication(
    root: &Path,
    current: &BTreeMap<String, Profile>,
    profile: &Profile,
    expected_host: Option<&[u8; 33]>,
) -> Result<Option<ProfilePublicationReport>> {
    let Some(existing) = current.get(&profile.name) else {
        return Ok(None);
    };
    if existing == profile {
        if let Some((marker, _)) =
            read_profile_publication_marker(&profile_publication_marker_path(root, &profile.name))?
        {
            if &marker.profile != profile {
                return Err(Error::InvalidConfig(
                    "profile publication registry binding changed",
                ));
            }
            if expected_host.is_some_and(|host| marker.probe.host_id_hex != hex(host)) {
                return Err(Error::InvalidProfile(
                    "saved server does not match the expected host",
                ));
            }
            return Ok(Some(ProfilePublicationReport {
                profile: marker.profile,
                probe: marker.probe,
            }));
        }
    }
    Err(Error::ProfileExists)
}

/// Whether the pending publication directory at `path` carries `authorization`
/// in its marker, which is the only proof of ownership left once a publication
/// has released its staging reservation.
///
/// A missing, unreadable or malformed marker answers no. Failing closed here
/// means an abandoned directory is left for the next open's recovery to remove,
/// which is exactly what recovery is for; failing open would let one
/// publication delete another's committed state directory.
fn publication_is_authorized_by(path: &Path, authorization: &[u8; 32]) -> bool {
    matches!(
        read_profile_publication_marker(&path.join(PROFILE_PUBLICATION_MARKER)),
        Ok(Some((publication, _))) if &publication.authorization == authorization
    )
}

fn profile_publication_marker_path(root: &Path, profile: &str) -> PathBuf {
    root.join("profiles")
        .join(profile)
        .join(PROFILE_PUBLICATION_MARKER)
}

fn profile_publication_marker_exists(root: &Path, profile: &str) -> Result<bool> {
    match fs::symlink_metadata(profile_publication_marker_path(root, profile)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn write_profile_publication_marker(
    path: &Path,
    profile: &Profile,
    probe: &ProbeReport,
    checkpoint: &RollbackCheckpoint,
    authorization: [u8; 32],
) -> Result<[u8; 32]> {
    let encoded = serde_json::to_vec(&ProfilePublicationMarker {
        version: PROFILE_PUBLICATION_MARKER_VERSION,
        profile: profile.clone(),
        probe: probe.clone(),
        checkpoint: checkpoint.clone(),
        authorization,
    })?;
    atomic_private_write(path, &encoded)?;
    Ok(prefixed_hash(PROFILE_PUBLICATION_BINDING_TYPE_ID, &encoded))
}

pub(super) struct ProfilePublicationBinding {
    pub(super) nonce: [u8; 32],
    pub(super) marker_digest: [u8; 32],
}

pub(super) fn profile_publication_checkpoint_binding(
    root: &Path,
    session: &ProfileSession,
    current: &RollbackCheckpoint,
) -> Result<Option<ProfilePublicationBinding>> {
    let marker_path = session.paths.directory.join(PROFILE_PUBLICATION_MARKER);
    let Some((marker, marker_digest)) = read_profile_publication_marker(&marker_path)? else {
        return Ok(None);
    };
    let published = load_registry(root)?;
    if published.get(&session.profile.name) != Some(&session.profile) {
        return Err(Error::InvalidConfig(
            "profile publication is not bound to the registry",
        ));
    }
    if marker.profile != session.profile || marker.probe.acceptance != ProbeAcceptance::Inserted {
        return Err(Error::InvalidConfig(
            "profile publication identity binding changed",
        ));
    }
    let target = ProbeTarget::parse(&session.profile.probe)?;
    let stored = HardStateStore::open(&session.paths.hard_database)?
        .host_for_lookup(target.hostname())?
        .ok_or(Error::InvalidConfig("profile publication host is missing"))?;
    if marker.probe.lookup_name != target.hostname()
        || marker.probe.canonical_name != stored.canonical_name
        || marker.probe.host_id_hex != hex(&stored.host_id)
        || marker.probe.host_chain_sequence != stored.chain_seqno
        || marker.probe.merkle_epoch != stored.merkle_root.epoch
    {
        return Err(Error::InvalidConfig(
            "profile publication probe facts changed",
        ));
    }
    if serde_json::to_vec(&marker.checkpoint)? != serde_json::to_vec(current)? {
        return Err(Error::InvalidConfig(
            "profile publication checkpoint binding changed",
        ));
    }
    Ok(Some(ProfilePublicationBinding {
        nonce: marker.authorization,
        marker_digest,
    }))
}

/// Where a profile's publication marker lives while its publication is pending.
pub(super) fn profile_publication_marker(session: &ProfileSession) -> PathBuf {
    session.paths.directory.join(PROFILE_PUBLICATION_MARKER)
}

pub(super) fn profile_publication_is_pending(session: &ProfileSession) -> Result<bool> {
    read_profile_publication_marker(&session.paths.directory.join(PROFILE_PUBLICATION_MARKER))
        .map(|marker| marker.is_some())
}

pub(super) fn complete_profile_publication(session: &ProfileSession) -> Result<()> {
    let marker = session.paths.directory.join(PROFILE_PUBLICATION_MARKER);
    match fs::remove_file(marker) {
        Ok(()) => File::open(&session.paths.directory)?
            .sync_all()
            .map_err(Into::into),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

impl ClientCredentials {
    pub fn check_and_add_profile(
        &self,
        registry: &mut ProfileRegistry,
        profile: Profile,
    ) -> Result<ProfilePublicationReport> {
        self.check_and_add_profile_with_control(
            registry,
            profile,
            Duration::from_secs(30),
            CancellationToken::new(),
        )
    }

    pub fn check_and_add_profile_with_control(
        &self,
        registry: &mut ProfileRegistry,
        profile: Profile,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<ProfilePublicationReport> {
        self.check_and_add_profile_for_host_with_control(
            registry,
            profile,
            timeout,
            cancellation,
            None,
        )
    }

    pub fn check_and_add_profile_for_host_with_control(
        &self,
        registry: &mut ProfileRegistry,
        profile: Profile,
        timeout: Duration,
        cancellation: CancellationToken,
        expected_host: Option<&[u8; 33]>,
    ) -> Result<ProfilePublicationReport> {
        if self.root != registry.root {
            return Err(Error::InvalidConfig(
                "profile registry belongs to a different client state",
            ));
        }
        let result = match expected_host {
            Some(expected_host) => registry.check_and_add_profile_for_host_with_control(
                self,
                profile.clone(),
                timeout,
                cancellation,
                Some(expected_host),
            ),
            None => registry.check_and_add_profile_with_control(
                self,
                profile.clone(),
                timeout,
                cancellation,
            ),
        };
        let report = result?;
        let session = ProfileSession::open(registry, &profile.name)?;
        self.with_checked_session(&session, |_| Ok::<_, Error>(()))?;
        Ok(report)
    }

    /// Removes a profile and deletes all associated local state, including
    /// credential stores.
    ///
    /// Local files are deleted before the registry entry so an interrupted
    /// removal can be safely retried without leaving unreferenced secrets.
    pub fn remove_profile(&self, registry: &mut ProfileRegistry, name: &str) -> Result<bool> {
        if self.root != registry.root {
            return Err(Error::InvalidConfig(
                "profile registry belongs to a different client state",
            ));
        }
        match registry.profile(name) {
            Ok(_) => {}
            Err(Error::ProfileMissing) => return Ok(false),
            Err(error) => return Err(error),
        }
        let session = ProfileSession::open(registry, name)?;
        self.erase_profile_state_for_removal(&session)?;
        super::checkpoint::remove_profile_directory(&session.paths.directory)?;
        registry.remove(name)
    }
}

fn remove_profile_publication_directory(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or(Error::InvalidConfig("profile publication has no parent"))?;
    fs::remove_dir_all(path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

struct RegistryMutationLock(File);

impl RegistryMutationLock {
    fn acquire(root: &Path) -> Result<Self> {
        Self::acquire_with_wait(root, true)
    }

    fn acquire_with_wait(root: &Path, wait: bool) -> Result<Self> {
        Self::acquire_mode(root, wait, true)
    }

    /// Acquires the registry lock shared.
    ///
    /// A shared hold still excludes every registry *writer*, because each one
    /// takes this same lock exclusively: `profiles.toml` cannot be replaced,
    /// and no profile state directory can be created, renamed or removed,
    /// while any shared holder exists. It does not exclude other shared
    /// holders, so a caller that may itself mutate the state root must drop it
    /// and re-acquire exclusively before doing so.
    fn acquire_shared_with_wait(root: &Path, wait: bool) -> Result<Self> {
        Self::acquire_mode(root, wait, false)
    }

    fn acquire_mode(root: &Path, wait: bool, exclusive: bool) -> Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(root.join(REGISTRY_LOCK_FILE))?;
        match (wait, exclusive) {
            (true, true) => fs2::FileExt::lock_exclusive(&file)?,
            (false, true) => fs2::FileExt::try_lock_exclusive(&file)?,
            (true, false) => fs2::FileExt::lock_shared(&file)?,
            (false, false) => fs2::FileExt::try_lock_shared(&file)?,
        }
        Ok(Self(file))
    }
}

impl Drop for RegistryMutationLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

/// A staging directory reserved for one in-flight profile publication.
///
/// A publication no longer holds the registry lock across its network probe,
/// so something else has to tell a concurrent registry open whether a staging
/// directory belongs to a running publication or to one that died. The
/// exclusive lock this holds on [`PROFILE_PUBLICATION_LOCK`] answers exactly
/// that, and the kernel answers it even for a publication that was killed: the
/// lock is released when the owning file description closes, so a staging
/// directory left by a crash always becomes recoverable, while one left by a
/// live publication is never removed under it.
///
/// Lock ordering: a holder of this lock may block on the registry lock (that
/// is what a publication does to commit), so a holder of the registry lock
/// must only ever *try* this one, never wait on it. [`Self::reserve`] and
/// [`Self::is_live`] are the only callers under the registry lock and both
/// only try, so the two locks cannot deadlock against each other.
struct PublicationStaging {
    directory: PathBuf,
    lock: Option<File>,
}

impl PublicationStaging {
    fn directory_for(profiles_directory: &Path, name: &str) -> PathBuf {
        profiles_directory.join(format!("{PROFILE_STAGING_PREFIX}{name}"))
    }

    /// Opens the lock file for `directory`. A probe opens it read-only, since
    /// it only needs a file description to lock; only a reservation creates it.
    fn open_lock_file(directory: &Path, create: bool) -> Result<Option<File>> {
        let mut options = OpenOptions::new();
        options.read(true);
        if create {
            options.write(true).create(true).truncate(false);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        match options.open(directory.join(PROFILE_PUBLICATION_LOCK)) {
            Ok(file) => Ok(Some(file)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Whether a publication still owns `directory`. Never blocks, so it is
    /// safe to call while the registry lock is held.
    ///
    /// The probe takes the lock *shared*, so two concurrent probes cannot make
    /// each other report a live owner; only the exclusive hold a publication
    /// keeps blocks it. A staging directory with no lock file at all is not
    /// live either: the file is unlinked only under the exclusive registry
    /// lock, immediately before the rename that publishes the directory, so its
    /// absence means the publication died inside that window.
    fn is_live(directory: &Path) -> Result<bool> {
        let Some(file) = Self::open_lock_file(directory, false)? else {
            return Ok(false);
        };
        match fs2::FileExt::try_lock_shared(&file) {
            Ok(()) => {
                let _ = fs2::FileExt::unlock(&file);
                Ok(false)
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(true),
            Err(error) => Err(error.into()),
        }
    }

    /// Reserves the staging directory for `name`. The caller must hold the
    /// registry lock exclusively and must already have recovered stale staging
    /// directories, so a live directory found here belongs to a publication
    /// that is still running and `Ok(None)` is returned for it.
    fn reserve(profiles_directory: &Path, name: &str) -> Result<Option<Self>> {
        let directory = Self::directory_for(profiles_directory, name);
        match fs::symlink_metadata(&directory) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(Error::InvalidConfig(
                        "profile publication staging path is unsafe",
                    ));
                }
                if Self::is_live(&directory)? {
                    return Ok(None);
                }
                remove_profile_publication_directory(&directory)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let directory = prepare_private_directory(&directory)?;
        let file = Self::open_lock_file(&directory, true)?
            .ok_or(Error::InvalidConfig("profile publication lock is missing"))?;
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                // Nothing else reserves a name without the registry lock, so
                // this is only the microsecond during which a concurrent scan
                // holds its shared probe. The caller waits and re-reads.
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        }
        Ok(Some(Self {
            directory,
            lock: Some(file),
        }))
    }

    /// Blocks until the publication that owns `name` releases its staging
    /// directory. The caller must not hold the registry lock, since the owner
    /// needs it to commit.
    fn wait_for_release(profiles_directory: &Path, name: &str) -> Result<()> {
        let directory = Self::directory_for(profiles_directory, name);
        let Some(file) = Self::open_lock_file(&directory, false)? else {
            return Ok(());
        };
        fs2::FileExt::lock_exclusive(&file)?;
        let _ = fs2::FileExt::unlock(&file);
        Ok(())
    }

    /// Whether this still holds the reservation. Once it does not, the staging
    /// path alone no longer proves ownership of the directory at it.
    fn holds_reservation(&self) -> bool {
        self.lock.is_some()
    }

    /// Unlinks the lock file, so a published directory never carries it, and
    /// drops the reservation.
    ///
    /// The caller must hold the registry lock exclusively and must rename or
    /// remove the staging directory before releasing that lock: from here on a
    /// scan reads the directory as abandoned and will recover it.
    fn release(&mut self) -> Result<()> {
        match fs::remove_file(self.directory.join(PROFILE_PUBLICATION_LOCK)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if let Some(file) = self.lock.take() {
            let _ = fs2::FileExt::unlock(&file);
        }
        Ok(())
    }
}

impl Drop for PublicationStaging {
    fn drop(&mut self) {
        if let Some(file) = &self.lock {
            let _ = fs2::FileExt::unlock(file);
        }
    }
}

pub(crate) fn load_registry(root: &Path) -> Result<BTreeMap<String, Profile>> {
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProbeReport {
    pub acceptance: ProbeAcceptance,
    pub lookup_name: String,
    pub canonical_name: String,
    pub host_id_hex: String,
    pub host_chain_sequence: u64,
    pub merkle_epoch: u64,
    /// The server's advertised client-version compatibility, when it answered
    /// `Reg.getClientVersionInfo`. Older servers that do not implement the call
    /// leave this unset rather than failing the probe.
    #[serde(default)]
    pub server_version: Option<ServerVersionReport>,
}

/// The server's advertised client-version range plus this client's verdict for
/// its own pinned protocol version.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServerVersionReport {
    /// Oldest client version the server accepts, `"major.minor.patch"`.
    pub minimum: Option<String>,
    /// Newest client version the server accepts, `"major.minor.patch"`.
    pub newest: Option<String>,
    /// Free-text notice the server attached, if any.
    pub message: String,
    /// Whether the pinned client version falls inside the advertised range.
    pub compatible: bool,
}

fn format_semver(version: SemVer) -> String {
    format!("{}.{}.{}", version.major, version.minor, version.patch)
}

/// The server's notice is untrusted display text. Decode it lossily, replace
/// control characters, bound it, and collapse surrounding whitespace so it can
/// never corrupt the desktop's strict response projection.
fn sanitize_server_message(bytes: &[u8]) -> String {
    let mut message = String::new();
    for character in String::from_utf8_lossy(bytes).chars().take(512) {
        let character = if character.is_control() {
            ' '
        } else {
            character
        };
        // Match the desktop projection's byte limit without splitting UTF-8.
        if message.len() + character.len_utf8() > 1024 {
            break;
        }
        message.push(character);
    }
    message.trim().to_owned()
}

fn semver_at_most(left: SemVer, right: SemVer) -> bool {
    (left.major, left.minor, left.patch) <= (right.major, right.minor, right.patch)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProfilePublicationReport {
    pub profile: Profile,
    pub probe: ProbeReport,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ServerStatusSnapshot {
    pub profile: String,
    pub configured_probe: String,
    pub host: Option<StoredHostStatus>,
    pub compatibility: CompatibilityStatus,
    pub chat_supported: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StoredHostStatus {
    pub lookup_name: String,
    pub canonical_name: String,
    pub host_id_hex: String,
    pub host_chain_sequence: u64,
    pub merkle_epoch: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProbeAcceptance {
    Inserted,
    Advanced,
    Unchanged,
}

impl From<foks_client_db::Acceptance> for ProbeAcceptance {
    fn from(value: foks_client_db::Acceptance) -> Self {
        match value {
            foks_client_db::Acceptance::Inserted => Self::Inserted,
            foks_client_db::Acceptance::Advanced => Self::Advanced,
            foks_client_db::Acceptance::Unchanged => Self::Unchanged,
        }
    }
}

pub struct ProfileSession {
    pub(super) lease: ClientStateLease,
    pub(super) profile: Profile,
    pub(super) paths: ProfilePaths,
    pub(super) client: FoksClient,
    pub(super) adapter_clock: std::sync::Arc<dyn crate::AdapterClock>,
    /// Empty unless an embedder attached caches for a read operation. See
    /// [`crate::auth_cache`].
    pub(super) read_caches: crate::ReadCaches,
    /// Total relay wait interactive pairing may spend. Unset means the
    /// client's own default; an embedder that cancels pairing at a cap of its
    /// own sets this below that cap so it never cancels a wait the client
    /// would have kept.
    pub(super) pairing_budget: Option<Duration>,
}

impl ProfileSession {
    pub fn open(registry: &ProfileRegistry, name: &str) -> Result<Self> {
        let profile = registry.profile(name)?.clone();
        let paths = registry.prepare_profile_directory(name)?;
        let client = client_for_trust(&profile.trust, &registry.root)?;
        Ok(Self {
            lease: registry.lease.clone(),
            profile,
            paths,
            client,
            adapter_clock: std::sync::Arc::new(crate::SystemAdapterClock),
            read_caches: crate::ReadCaches::default(),
            pairing_budget: None,
        })
    }

    /// Opens a profile over a transport an embedder already holds, so several
    /// operations against one profile share a connection pool instead of
    /// paying a TLS handshake each. The base client must have been built for
    /// exactly this profile's trust configuration; see
    /// [`base_client_for_profile`].
    pub fn open_with_control_and_client(
        registry: &ProfileRegistry,
        name: &str,
        timeout: Duration,
        cancellation: CancellationToken,
        base: &FoksClient,
    ) -> Result<Self> {
        if timeout.is_zero() {
            return Err(Error::InvalidConfig("zero profile operation timeout"));
        }
        let profile = registry.profile(name)?.clone();
        let paths = registry.prepare_profile_directory(name)?;
        let mut client = base.clone();
        client.set_timeout(timeout);
        let client = client.with_cancellation_token(cancellation);
        Ok(Self {
            lease: registry.lease.clone(),
            profile,
            paths,
            client,
            adapter_clock: std::sync::Arc::new(crate::SystemAdapterClock),
            read_caches: crate::ReadCaches::default(),
            pairing_budget: None,
        })
    }

    /// Bounds the total relay wait of an interactive pairing operation. The
    /// budget must be under the caller's own cancellation cap, so a pairing
    /// the client is still waiting on is never reported as an ambiguous
    /// deadline error by the caller instead.
    pub fn with_pairing_budget(mut self, budget: Duration) -> Self {
        self.pairing_budget = Some(budget);
        self
    }

    /// The pairing budget for this session, or the client default when the
    /// caller stated none.
    pub fn pairing_budget(&self) -> Duration {
        self.pairing_budget
            .unwrap_or(foks_client::DEFAULT_KEX_PAIRING_BUDGET)
    }

    /// Attaches read caches. Callers attach them only for operations that do
    /// not mutate, so a mutating operation always authenticates against the
    /// current server state.
    pub fn with_read_caches(mut self, caches: crate::ReadCaches) -> Self {
        self.read_caches = caches;
        self
    }

    pub fn with_authenticated_user_cache(
        mut self,
        cache: std::sync::Arc<dyn crate::AuthenticatedUserCache>,
    ) -> Self {
        self.read_caches.authenticated_users = Some(cache);
        self
    }

    pub fn with_team_view_token_cache(
        mut self,
        cache: std::sync::Arc<dyn crate::TeamViewTokenCache>,
    ) -> Self {
        self.read_caches.team_view_tokens = Some(cache);
        self
    }

    pub fn with_kv_node_cache(mut self, cache: std::sync::Arc<dyn crate::KvNodeCache>) -> Self {
        self.read_caches.kv_nodes = Some(cache);
        self
    }

    /// Inject both clocks together; retention never reads wall time directly.
    pub fn with_adapter_clock(mut self, clock: std::sync::Arc<dyn crate::AdapterClock>) -> Self {
        self.adapter_clock = clock;
        self
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

    /// Whether this session may consult read caches. A session opened for a
    /// mutating operation never can.
    pub fn serves_reads_from_cache(&self) -> bool {
        self.read_caches.authenticated_users.is_some()
            || self.read_caches.team_view_tokens.is_some()
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
        let client = client_for_trust(&profile.trust, &registry.root)?
            .with_operation_controls_from(&self.client);
        Ok(Self {
            lease: registry.lease.clone(),
            profile,
            paths,
            client,
            adapter_clock: std::sync::Arc::clone(&self.adapter_clock),
            // A related profile is only opened by cross-profile mutations, so
            // it never inherits read caches.
            read_caches: crate::ReadCaches::default(),
            pairing_budget: self.pairing_budget,
        })
    }

    pub(super) fn rollback_checkpoint(&self) -> Result<RollbackCheckpoint> {
        rollback_checkpoint(self)
    }
}

impl ProfileSession {
    pub fn has_hard_state_artifacts(&self) -> Result<bool> {
        super::checkpoint::hard_state_artifacts_exist(&self.paths.hard_database)
    }

    /// Asks the verified host which client versions it accepts and judges this
    /// client's pinned version against that range. A server too old to know the
    /// call yields an error, which the caller treats as "not reported".
    fn server_version_report(&self, host: &foks_client::PinnedHost) -> Result<ServerVersionReport> {
        let info = self.client.client_version_info(
            host,
            &ClientVersionExt {
                version: PINNED_CLIENT_VERSION,
                linker_version: format!("foks-rs/{}", env!("CARGO_PKG_VERSION")).into_bytes(),
                linker_packaging: b"rust".to_vec(),
            },
        )?;
        let compatible = info
            .minimum
            .is_none_or(|minimum| semver_at_most(minimum, PINNED_CLIENT_VERSION))
            && info
                .newest
                .is_none_or(|newest| semver_at_most(PINNED_CLIENT_VERSION, newest));
        Ok(ServerVersionReport {
            minimum: info.minimum.map(format_semver),
            newest: info.newest.map(format_semver),
            message: sanitize_server_message(&info.message),
            compatible,
        })
    }

    fn probe_and_pin_unchecked(&self) -> Result<ProbeReport> {
        let target = ProbeTarget::parse(&self.profile.probe)?;
        let outcome = self
            .client
            .probe_and_pin(&target, &self.paths.hard_database)?;
        let server_version = self.server_version_report(&outcome.pinned).ok();
        Ok(ProbeReport {
            acceptance: outcome.acceptance.into(),
            lookup_name: target.hostname().to_owned(),
            canonical_name: outcome.verified.snapshot.canonical_name().to_owned(),
            host_id_hex: hex(outcome.pinned.host_id().as_bytes()),
            host_chain_sequence: outcome.verified.snapshot.chain_seqno(),
            merkle_epoch: outcome.verified.snapshot.merkle_root().epoch(),
            server_version,
        })
    }

    /// Returns configured and signed-lease facts before this profile has any
    /// pinned hard state. Once any hard-state artifact exists, callers must
    /// use a checked session so rollback verification cannot be bypassed.
    pub fn server_status_without_pinned_host(&self) -> Result<ServerStatusSnapshot> {
        self.lease.validate()?;
        self.profile.require(Capability::Probe)?;
        if super::checkpoint::hard_state_artifacts_exist(&self.paths.hard_database)? {
            return Err(Error::InvalidConfig(
                "server status requires a checked profile session",
            ));
        }
        server_status_snapshot(&self.profile, None)
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
    let store = HardStateStore::open(&session.paths.hard_database)?;
    checkpoint_for_store(&session.profile, &store)
}

pub(crate) fn checkpoint_for_store(
    profile: &Profile,
    store: &HardStateStore,
) -> Result<RollbackCheckpoint> {
    let target = ProbeTarget::parse(&profile.probe)?;
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
        profile: profile.name.clone(),
        database_id: metadata.database_id,
        hard_state_revision: metadata.revision,
        write_token: metadata.write_token,
        host,
    })
}

impl CheckedProfileSession<'_> {
    pub fn reconcile_saved_host(&self) -> Result<(String, String)> {
        self.reconcile_saved_host_with_client(&self.client)
    }

    pub fn reconcile_saved_host_with_timeout(&self, timeout: Duration) -> Result<(String, String)> {
        if timeout.is_zero() {
            return Err(Error::Client(foks_client::Error::DeadlineExceeded));
        }
        let mut client = self.client.clone();
        client.set_timeout(timeout);
        self.reconcile_saved_host_with_client(&client)
    }

    fn reconcile_saved_host_with_client(&self, client: &FoksClient) -> Result<(String, String)> {
        self.lease.validate()?;
        self.profile.require(Capability::Probe)?;
        let target = ProbeTarget::parse(&self.profile.probe)?;
        let stored = HardStateStore::open(&self.paths.hard_database)?
            .host_for_lookup(target.hostname())?
            .ok_or(Error::SavedTrustMissing)?;
        let expected = self.pinned_host()?;
        if expected.host_id().as_bytes() != stored.host_id.as_slice() {
            return Err(Error::RollbackDetected("saved host identity changed"));
        }
        let outcome =
            client.probe_and_pin_host_id(&target, expected.host_id(), &self.paths.hard_database)?;
        Ok((hex(outcome.pinned.host_id().as_bytes()), target.address()))
    }

    pub fn probe_and_pin(&self) -> Result<ProbeReport> {
        self.profile.require(Capability::Probe)?;
        self.probe_and_pin_unchecked()
    }

    /// Returns only facts authenticated and retained by an earlier probe.
    /// It deliberately has no checked-at or trusted-since field because the
    /// hard-state schema does not persist either fact.
    pub fn server_status(&self) -> Result<ServerStatusSnapshot> {
        self.profile.require(Capability::Probe)?;
        let target = ProbeTarget::parse(&self.profile.probe)?;
        let stored =
            HardStateStore::open(&self.paths.hard_database)?.host_for_lookup(target.hostname())?;
        let host = stored.map(|stored| StoredHostStatus {
            lookup_name: target.hostname().to_owned(),
            canonical_name: stored.canonical_name,
            host_id_hex: hex(&stored.host_id),
            host_chain_sequence: stored.chain_seqno,
            merkle_epoch: stored.merkle_root.epoch,
        });
        let chat_supported = host
            .as_ref()
            .map(|_| self.pinned_host().map(|host| host.supports_chat()))
            .transpose()?;
        let mut status = server_status_snapshot(&self.profile, host)?;
        status.chat_supported = chat_supported;
        Ok(status)
    }

    pub fn pinned_host(&self) -> Result<foks_client::PinnedHost> {
        let target = ProbeTarget::parse(&self.profile.probe)?;
        self.client
            .pinned_host(target.hostname(), &self.paths.hard_database)
            .map_err(Into::into)
    }
}

fn server_status_snapshot(
    profile: &Profile,
    host: Option<StoredHostStatus>,
) -> Result<ServerStatusSnapshot> {
    // Parse again even for the no-host case so a status response never
    // blesses malformed configured probe text.
    ProbeTarget::parse(&profile.probe)?;
    Ok(ServerStatusSnapshot {
        profile: profile.name.clone(),
        configured_probe: profile.probe.clone(),
        host,
        compatibility: profile.protocol.compatibility_status(),
        chat_supported: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::{
        begin_profile_publication_with_store, bind_profile_publication_with_store,
        cancel_profile_publication_with_store, database_claim_record_key,
        profile_publication_authorization_key, rollback_record_key, CheckpointStore,
        ProfilePublicationAuthorization, TEST_FAIL_AFTER_CHECKPOINT_PUBLICATION,
    };
    use foks_keystore::MemorySecretStore;
    use foks_server_testkit::TestEnvironment;
    use std::collections::BTreeSet;

    #[test]
    fn desktop_capability_contract_covers_profile_and_artifact_grants() {
        fn grant_index(capability: Capability) -> Option<usize> {
            match capability {
                Capability::Probe => None,
                Capability::Signup => Some(0),
                Capability::UserSync => Some(1),
                Capability::Kv => Some(2),
                Capability::DeviceAdministration => Some(3),
                Capability::Recovery => Some(4),
                Capability::Passphrases => Some(5),
                Capability::Teams => Some(6),
                Capability::Chat => Some(7),
                Capability::Federation => Some(8),
            }
        }
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../apps/desktop/src-tauri/wire-contract.json"
        ))
        .unwrap();
        let names = fixture["protocolCapabilities"].as_array().unwrap();
        assert_eq!(names.len(), 9);
        for (index, name) in names.iter().enumerate() {
            let capability: Capability = serde_json::from_value(name.clone()).unwrap();
            assert_eq!(grant_index(capability), Some(index));
            assert_eq!(serde_json::to_value(capability).unwrap(), *name);
            assert_eq!(capability.as_str(), name.as_str().unwrap());
            assert_eq!(
                capability_from_artifact(name.as_str().unwrap()).unwrap(),
                capability
            );
        }
        assert_eq!(grant_index(Capability::Probe), None);
        assert!(capability_from_artifact("probe").is_err());
        for name in ["invented", "", " KV "] {
            assert!(capability_from_artifact(name).is_err());
        }
        for case in fixture["compatibilityCases"].as_array().unwrap() {
            if case.get("decoded").is_none() {
                continue;
            }
            let status: CompatibilityStatus = serde_json::from_value(case["wire"].clone()).unwrap();
            let now = case["nowSeconds"].as_u64().unwrap();
            let granted = case["granted"].as_array().unwrap();
            for name in names {
                let capability = capability_from_artifact(name.as_str().unwrap()).unwrap();
                assert_eq!(
                    status.denial_at(capability, now).is_none(),
                    granted.contains(name),
                    "{}: {}",
                    case["name"],
                    name
                );
            }
        }
    }

    #[test]
    fn server_notices_fit_the_desktop_utf8_byte_limit() {
        for (notice, expected_bytes) in [
            ("a".repeat(600), 512),
            ("界".repeat(512), 1023),
            ("🦊".repeat(512), 1024),
            (format!("{}a界", "界".repeat(341)), 1024),
        ] {
            let sanitized = sanitize_server_message(notice.as_bytes());
            assert_eq!(sanitized.len(), expected_bytes);
            assert!(notice.starts_with(&sanitized));
        }
        let invalid_utf8 = vec![0xff; 512];
        assert_eq!(sanitize_server_message(&invalid_utf8).len(), 1023);
        assert_eq!(
            sanitize_server_message(b"\nhello\0world\r\n"),
            "hello world"
        );
    }

    struct MemoryPublicationAuthorizer {
        store: std::sync::Mutex<MemorySecretStore>,
        begins: std::sync::atomic::AtomicUsize,
    }

    impl MemoryPublicationAuthorizer {
        fn new() -> Self {
            Self {
                store: std::sync::Mutex::new(MemorySecretStore::default()),
                begins: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn contains(&self, profile: &str) -> bool {
            CheckpointStore::get(
                &mut *self.store.lock().unwrap(),
                &profile_publication_authorization_key(profile).unwrap(),
            )
            .is_ok()
        }
    }

    impl ProfilePublicationAuthorizer for MemoryPublicationAuthorizer {
        fn begin(&self, profile: &str) -> Result<[u8; 32]> {
            self.begins
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            begin_profile_publication_with_store(profile, &mut *self.store.lock().unwrap())
        }

        fn bind(
            &self,
            profile: &str,
            authorization: &[u8; 32],
            marker_digest: &[u8; 32],
        ) -> Result<()> {
            bind_profile_publication_with_store(
                profile,
                authorization,
                marker_digest,
                &mut *self.store.lock().unwrap(),
            )
        }

        fn cancel(&self, profile: &str, authorization: &[u8; 32]) -> Result<()> {
            cancel_profile_publication_with_store(
                profile,
                authorization,
                &mut *self.store.lock().unwrap(),
            )
        }
    }

    fn registry_only_profile(name: &str) -> Profile {
        Profile {
            name: name.to_owned(),
            label: None,
            probe: "foks.example.test".to_owned(),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::WebPki,
        }
    }

    /// States exactly what the shared open still excludes. Readers no longer
    /// exclude one another, but every writer takes the same lock exclusively,
    /// so a shared holder still excludes every registry mutation: no rewrite of
    /// `profiles.toml` and no creation, rename or removal of a profile state
    /// directory can overlap a parse or a scan.
    #[test]
    fn a_shared_registry_open_still_excludes_every_writer() {
        let temporary = tempfile::tempdir().unwrap();
        let mut registry = ProfileRegistry::open(temporary.path().join("state")).unwrap();
        registry.add(registry_only_profile("one")).unwrap();
        registry.add(registry_only_profile("two")).unwrap();
        let root = registry.root().to_owned();
        drop(registry);

        let shared = RegistryMutationLock::acquire_shared_with_wait(&root, false).unwrap();
        // Readers share: an open succeeds, and so does another shared hold.
        assert_eq!(
            ProfileRegistry::try_open(&root).unwrap().profiles().len(),
            2
        );
        let second = RegistryMutationLock::acquire_shared_with_wait(&root, false).unwrap();
        // Writers do not: every mutation path takes this lock exclusively.
        assert!(RegistryMutationLock::acquire_with_wait(&root, false).is_err());
        drop((shared, second));

        // And an exclusive holder still excludes readers, so no parse or scan
        // ever observes a half-written registry.
        let exclusive = RegistryMutationLock::acquire_with_wait(&root, false).unwrap();
        assert!(ProfileRegistry::try_open(&root).is_err());
        drop(exclusive);
        assert!(ProfileRegistry::try_open(&root).is_ok());
    }

    /// Two reads on different profiles complete while a third shared holder is
    /// parked on the lock. Under an exclusive open they would serialize behind
    /// it and the receive would time out.
    #[test]
    fn two_reads_on_different_profiles_open_concurrently() {
        let temporary = tempfile::tempdir().unwrap();
        let mut registry = ProfileRegistry::open(temporary.path().join("state")).unwrap();
        registry.add(registry_only_profile("one")).unwrap();
        registry.add(registry_only_profile("two")).unwrap();
        let root = registry.root().to_owned();
        drop(registry);

        let parked = RegistryMutationLock::acquire_shared_with_wait(&root, false).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let threads: Vec<_> = ["one", "two"]
            .into_iter()
            .map(|name| {
                let root = root.clone();
                let sender = sender.clone();
                std::thread::spawn(move || {
                    let registry = ProfileRegistry::open(&root).unwrap();
                    registry.profile(name).unwrap();
                    sender.send(name).unwrap();
                })
            })
            .collect();
        for _ in 0..2 {
            receiver
                .recv_timeout(Duration::from_secs(20))
                .expect("a read must not wait for another reader's registry lock");
        }
        for thread in threads {
            thread.join().unwrap();
        }
        drop(parked);
    }

    /// The publication scan is read-only under the shared lock and recovers
    /// only after the upgrade, so an interrupted publication still disappears.
    #[test]
    fn an_interrupted_publication_is_still_recovered_after_a_shared_open() {
        let temporary = tempfile::tempdir().unwrap();
        let registry = ProfileRegistry::open(temporary.path().join("state")).unwrap();
        let root = registry.root().to_owned();
        drop(registry);
        let profiles_directory = prepare_private_directory(&root.join("profiles")).unwrap();
        let abandoned = PublicationStaging::directory_for(&profiles_directory, "ghost");
        prepare_private_directory(&abandoned).unwrap();
        create_private_config(&abandoned.join(PROFILE_PUBLICATION_LOCK), b"").unwrap();

        {
            let _shared = RegistryMutationLock::acquire_shared_with_wait(&root, false).unwrap();
            let profiles = load_registry(&root).unwrap();
            assert!(scan_profile_publications(&root, &profiles, ScanMode::Observe).unwrap());
            assert!(abandoned.exists(), "an observing scan removes nothing");
        }
        drop(ProfileRegistry::open(&root).unwrap());
        assert!(!abandoned.exists());

        // The same holds for a published directory whose registry commit never
        // landed: no marker owner means the state is unreachable and removed.
        let orphan = profiles_directory.join("orphan");
        prepare_private_directory(&orphan).unwrap();
        create_private_config(&orphan.join(PROFILE_PUBLICATION_MARKER), b"{}").unwrap();
        {
            let _shared = RegistryMutationLock::acquire_shared_with_wait(&root, false).unwrap();
            let profiles = load_registry(&root).unwrap();
            assert!(matches!(
                scan_profile_publications(&root, &profiles, ScanMode::Observe),
                Err(Error::InvalidConfig(
                    "profile publication marker is malformed"
                ))
            ));
        }
    }

    /// The staging reservation is what replaces holding the registry lock
    /// across a probe: it refuses a second publication of the same name, and a
    /// concurrent reader neither blocks on it nor destroys it.
    #[test]
    fn a_live_publication_reservation_excludes_a_second_add_of_the_same_name() {
        let temporary = tempfile::tempdir().unwrap();
        let registry = ProfileRegistry::open(temporary.path().join("state")).unwrap();
        let root = registry.root().to_owned();
        drop(registry);
        let profiles_directory = prepare_private_directory(&root.join("profiles")).unwrap();

        let staging = PublicationStaging::reserve(&profiles_directory, "inflight")
            .unwrap()
            .expect("the first publication reserves the name");
        assert!(
            PublicationStaging::reserve(&profiles_directory, "inflight")
                .unwrap()
                .is_none(),
            "a second publication of the same name must not reserve it"
        );
        // A different name is unaffected.
        let other_name = PublicationStaging::reserve(&profiles_directory, "other")
            .unwrap()
            .expect("a different name reserves independently");

        let profiles = load_registry(&root).unwrap();
        assert!(
            !scan_profile_publications(&root, &profiles, ScanMode::Observe).unwrap(),
            "a live reservation is not recovery work, so a reader never upgrades"
        );
        drop(ProfileRegistry::try_open(&root).unwrap());
        assert!(staging.directory.exists());

        // Once the publication ends -- including by dying, since the kernel
        // drops the lock with the file description -- the next open recovers it.
        let directory = staging.directory.clone();
        drop(staging);
        let profiles = load_registry(&root).unwrap();
        assert!(scan_profile_publications(&root, &profiles, ScanMode::Observe).unwrap());
        drop(ProfileRegistry::open(&root).unwrap());
        assert!(!directory.exists());
        // The still-live reservation for the other name survived that recovery.
        assert!(other_name.directory.exists());
    }

    /// The whole point of splitting the add path: an add parked on its network
    /// probe no longer holds the registry lock, so every other profile stays
    /// readable while it runs.
    #[test]
    fn an_add_parked_on_its_probe_does_not_block_other_profiles() {
        let _guard = TEST_PROFILE_PUBLICATION_MUTEX.lock().unwrap();
        let environment = TestEnvironment::new().unwrap();
        let state = environment
            .client_path("add-probe-window", "state")
            .unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(local_profile(
                &environment,
                "existing",
                "127.0.0.1:1".to_owned(),
            ))
            .unwrap();
        drop(registry);

        // A bound but never accepted listener completes the TCP connection from
        // the backlog and then answers nothing, so the probe stays in flight
        // until its own timeout.
        let listener = environment.reserve_loopback_listener().unwrap();
        let port = listener.local_addr().unwrap().port();
        let slow = local_profile(&environment, "slow", format!("127.0.0.1:{port}"));
        let authorizer = std::sync::Arc::new(MemoryPublicationAuthorizer::new());
        let add = {
            let state = state.clone();
            let authorizer = authorizer.clone();
            std::thread::spawn(move || {
                let mut registry = ProfileRegistry::open(&state).unwrap();
                registry.check_and_add_profile_with_control(
                    authorizer.as_ref(),
                    slow,
                    Duration::from_secs(10),
                    CancellationToken::new(),
                )
            })
        };

        let staging = PublicationStaging::directory_for(&state.join("profiles"), "slow");
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !staging.exists() {
            assert!(std::time::Instant::now() < deadline, "probe never started");
            std::thread::sleep(Duration::from_millis(5));
        }
        // The probe is in flight. Every other profile is still readable, and
        // the reader does not have to wait out the probe's timeout to say so.
        let started = std::time::Instant::now();
        let other = ProfileRegistry::try_open(&state).unwrap();
        assert!(other.profile("existing").is_ok());
        assert!(matches!(other.profile("slow"), Err(Error::ProfileMissing)));
        assert!(started.elapsed() < Duration::from_secs(5));
        drop(other);

        assert!(add.join().unwrap().is_err());
        assert!(!staging.exists());
        assert!(!authorizer.contains("slow"));
        drop(listener);
    }

    fn local_profile(environment: &TestEnvironment, name: &str, probe: String) -> Profile {
        let root = environment
            .client_path(name, "publication-probe-root.der")
            .unwrap();
        environment.write_probe_root(&root).unwrap();
        Profile {
            name: name.to_owned(),
            label: None,
            probe,
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::CertificateDer { path: root },
        }
    }

    #[test]
    fn profile_label_is_optional_normalized_and_persistent() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let mut registry = ProfileRegistry::open(&root).unwrap();
        let original = Profile {
            name: "setup-foks-app-4430".to_owned(),
            label: None,
            probe: "foks.app:4430".to_owned(),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::WebPki,
        };
        registry.add(original.clone()).unwrap();

        let stored = fs::read_to_string(root.join("profiles.toml")).unwrap();
        assert!(!stored.contains("label"));
        drop(registry);
        let mut registry = ProfileRegistry::open(&root).unwrap();
        assert_eq!(registry.profile(&original.name).unwrap().label, None);

        assert!(registry
            .set_label(&original.name, Some("  FOKS  ".to_owned()))
            .unwrap());
        assert!(!registry
            .set_label(&original.name, Some("FOKS".to_owned()))
            .unwrap());
        drop(registry);
        let mut registry = ProfileRegistry::open(&root).unwrap();
        let labeled = registry.profile(&original.name).unwrap();
        assert_eq!(labeled.label.as_deref(), Some("FOKS"));
        assert_eq!(labeled.name, original.name);
        assert_eq!(labeled.probe, original.probe);
        assert_eq!(labeled.protocol, original.protocol);
        assert_eq!(labeled.trust, original.trust);

        assert!(registry
            .set_label(&original.name, Some("Work".to_owned()))
            .unwrap());
        assert!(registry
            .set_label(&original.name, Some(original.name.clone()))
            .unwrap());
        assert_eq!(registry.profile(&original.name).unwrap().label, None);
        assert!(!registry.set_label(&original.name, None).unwrap());
        assert!(matches!(
            registry.set_label("missing", Some("Missing".to_owned())),
            Err(Error::ProfileMissing)
        ));
    }

    #[test]
    fn profile_labels_reject_invalid_and_oversized_values() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(Profile {
                name: "local".to_owned(),
                label: None,
                probe: "foks.example.test".to_owned(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();
        for label in ["a\nb".to_owned(), "a\rb".to_owned(), "a\0b".to_owned()] {
            assert!(matches!(
                registry.set_label("local", Some(label)),
                Err(Error::InvalidProfile(_))
            ));
        }
        assert!(matches!(
            registry.set_label("local", Some("界".repeat(22))),
            Err(Error::InvalidProfile("profile label is too long"))
        ));
        let invalid_from_disk = Profile {
            label: Some("  ".to_owned()),
            ..registry.profile("local").unwrap().clone()
        };
        assert!(matches!(
            invalid_from_disk.validate(),
            Err(Error::InvalidProfile("profile label is empty"))
        ));
    }

    fn current_probe_only_profile(
        environment: &TestEnvironment,
        name: &str,
        probe: String,
    ) -> Profile {
        let mut profile = local_profile(environment, name, probe);
        profile.protocol = ProtocolPolicy::CurrentProbeOnly {
            compatibility_artifact_public_key:
                "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a".to_owned(),
            lease_url: "https://updates.example.test/foks/compatibility-artifact.json".to_owned(),
            last_artifact: None,
        };
        profile
    }

    #[test]
    fn a_session_opened_without_caches_never_consults_one() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(Profile {
                name: "local".to_owned(),
                label: None,
                probe: "127.0.0.1:1".to_owned(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();

        let session = ProfileSession::open(&registry, "local").unwrap();
        assert!(!session.serves_reads_from_cache());
        assert!(!ProfileSession::open_with_control(
            &registry,
            "local",
            Duration::from_secs(5),
            CancellationToken::new(),
        )
        .unwrap()
        .serves_reads_from_cache());
    }

    #[test]
    fn a_session_spends_the_default_pairing_budget_until_a_caller_states_its_own() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(Profile {
                name: "local".to_owned(),
                label: None,
                probe: "127.0.0.1:1".to_owned(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();

        // The CLI states no budget, so pairing keeps its five-minute total.
        let session = ProfileSession::open(&registry, "local").unwrap();
        assert_eq!(
            session.pairing_budget(),
            foks_client::DEFAULT_KEX_PAIRING_BUDGET
        );
        assert_eq!(session.pairing_budget(), Duration::from_secs(5 * 60));
        // A caller that cancels pairing at a cap of its own states a budget
        // under that cap instead.
        let bounded = ProfileSession::open(&registry, "local")
            .unwrap()
            .with_pairing_budget(Duration::from_secs(255));
        assert_eq!(bounded.pairing_budget(), Duration::from_secs(255));
    }

    #[test]
    fn saved_host_reconciliation_never_inserts_or_replaces_trust() {
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let addresses = environment.addresses().unwrap();
        let state = environment.client_path("saved-reconcile", "state").unwrap();
        let credentials =
            ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(local_profile(
                &environment,
                "saved",
                format!("localhost:{}", addresses.probe.port()),
            ))
            .unwrap();
        let session = ProfileSession::open(&registry, "saved").unwrap();
        credentials
            .with_checked_session(&session, |checked| {
                assert!(matches!(
                    checked.reconcile_saved_host(),
                    Err(Error::SavedTrustMissing)
                ));
                assert!(checked.server_status()?.host.is_none());
                checked.probe_and_pin()?;
                let before = checked.server_status()?.host.unwrap();
                assert!(matches!(
                    checked.reconcile_saved_host_with_timeout(Duration::ZERO),
                    Err(Error::Client(foks_client::Error::DeadlineExceeded))
                ));
                assert_eq!(checked.server_status()?.host.as_ref(), Some(&before));
                _server.shutdown().unwrap();
                environment.rotate_host_key().unwrap();
                let _rotated_server = environment.start_server().unwrap();
                let (host_id, configured_probe) =
                    checked.reconcile_saved_host_with_timeout(Duration::from_secs(5))?;
                assert_eq!(host_id, before.host_id_hex);
                assert_eq!(
                    configured_probe,
                    format!("localhost:{}", addresses.probe.port())
                );
                let after = checked.server_status()?.host.unwrap();
                assert_eq!(after.host_id_hex, before.host_id_hex);
                assert!(after.host_chain_sequence > before.host_chain_sequence);
                Ok::<_, Error>(())
            })
            .unwrap();
        let other = TestEnvironment::new().unwrap();
        let _other_server = other.start_server().unwrap();
        let hostile = local_profile(
            &other,
            "saved",
            format!("localhost:{}", other.addresses().unwrap().probe.port()),
        );
        registry.replace(hostile).unwrap();
        let hostile_session = ProfileSession::open(&registry, "saved").unwrap();
        credentials
            .with_checked_session(&hostile_session, |checked| {
                let before = checked.server_status()?.host.unwrap();
                assert!(checked.reconcile_saved_host().is_err());
                assert_eq!(
                    checked.server_status()?.host.unwrap().host_id_hex,
                    before.host_id_hex
                );
                Ok::<_, Error>(())
            })
            .unwrap();
    }

    #[test]
    fn compatibility_facts_preserve_grants_and_explain_denials() {
        let validated = CompatibilityStatus::Validated {
            expires_at: 200,
            capabilities: [Capability::Kv, Capability::Chat].into_iter().collect(),
        };
        assert_eq!(validated.denial_at(Capability::Kv, 199), None);
        assert_eq!(
            validated.denial_at(Capability::Teams, 199),
            Some(CapabilityDenial::NotGranted)
        );
        assert_eq!(
            validated.denial_at(Capability::Kv, 200),
            Some(CapabilityDenial::Expired)
        );
        assert_eq!(validated.denial_at(Capability::Probe, 200), None);
        assert_eq!(
            CompatibilityStatus::Missing.denial_at(Capability::Kv, 100),
            Some(CapabilityDenial::Missing)
        );
        assert_eq!(
            CompatibilityStatus::Incompatible {
                reason: CompatibilityFailure::Drift,
                expires_at: 200
            }
            .denial_at(Capability::Kv, 100),
            Some(CapabilityDenial::Incompatible)
        );
        assert_eq!(
            CompatibilityStatus::NotRequired.denial_at(Capability::Federation, 200),
            None
        );
    }

    #[test]
    fn lease_renewal_polls_only_within_the_expiry_margin() {
        fn signed(generated_at: u64, expires_at: u64) -> Box<SignedCompatibilityArtifact> {
            Box::new(SignedCompatibilityArtifact {
                artifact: foks_compat_artifact::CompatibilityArtifact {
                    schema_version: foks_compat_artifact::SCHEMA_VERSION,
                    generation: 1,
                    target: "foks.example.test".to_owned(),
                    run_id: "run-1".to_owned(),
                    generated_at,
                    expires_at,
                    protocol_metadata_sha256: PINNED_PROTOCOL_METADATA_SHA256.to_owned(),
                    mutation_digest: "11".repeat(32),
                    read_digest: "11".repeat(32),
                    outcome: CompatibilityOutcome::Compatible,
                    capabilities: ["kv".to_owned()].into_iter().collect(),
                    drift_reason: String::new(),
                },
                key_id: "11".repeat(32),
                signature: "11".repeat(64),
            })
        }
        fn validated(generated_at: u64, expires_at: u64) -> ProtocolPolicy {
            ProtocolPolicy::CurrentValidated {
                compatibility_artifact_public_key: "unused-by-selection".to_owned(),
                lease_url: "https://updates.example.test/lease".to_owned(),
                artifact: signed(generated_at, expires_at),
            }
        }

        // A profile with no lease service never polls.
        assert!(!ProtocolPolicy::V019.needs_lease_renewal_at(0));

        // A profile that has never fetched a lease, and one whose last
        // artifact does not grant this client, both keep polling.
        for last_artifact in [None, Some(signed(0, 7 * 24 * 60 * 60))] {
            assert!(ProtocolPolicy::CurrentProbeOnly {
                compatibility_artifact_public_key: "unused-by-selection".to_owned(),
                lease_url: "https://updates.example.test/lease".to_owned(),
                last_artifact,
            }
            .needs_lease_renewal_at(0));
        }

        // A full-length lease uses the flat margin.
        let week = 7 * 24 * 60 * 60;
        let policy = validated(0, week);
        assert!(!policy.needs_lease_renewal_at(0));
        assert!(!policy.needs_lease_renewal_at(week - COMPATIBILITY_RENEWAL_MARGIN_SECONDS - 1));
        assert!(policy.needs_lease_renewal_at(week - COMPATIBILITY_RENEWAL_MARGIN_SECONDS));
        // An expired lease polls on every tick, and so does one whose expiry
        // is already behind the caller's clock.
        assert!(policy.needs_lease_renewal_at(week));
        assert!(policy.needs_lease_renewal_at(week + 1));

        // A lease shorter than twice the margin uses half its own lifetime,
        // so the margin can never swallow the whole lease.
        let hour = 60 * 60;
        let short = validated(0, hour);
        assert!(!short.needs_lease_renewal_at(hour / 2 - 1));
        assert!(short.needs_lease_renewal_at(hour / 2));
    }

    #[test]
    fn server_status_distinguishes_lease_free_and_leased_protocols() {
        let v019 = Profile {
            name: "v019".to_owned(),
            label: None,
            probe: "foks.example.test".to_owned(),
            protocol: ProtocolPolicy::V019,
            trust: TrustRoot::WebPki,
        };
        let current = Profile {
            name: "current".to_owned(),
            label: None,
            protocol: ProtocolPolicy::CurrentProbeOnly {
                compatibility_artifact_public_key: "unused-by-status".to_owned(),
                lease_url: "https://updates.example.test/lease".to_owned(),
                last_artifact: None,
            },
            ..v019.clone()
        };

        let v019_status = server_status_snapshot(&v019, None).unwrap();
        assert_eq!(v019_status.compatibility, CompatibilityStatus::NotRequired);
        assert_eq!(v019_status.chat_supported, None);
        let current_status = server_status_snapshot(&current, None).unwrap();
        assert_eq!(current_status.compatibility, CompatibilityStatus::Missing);
        assert_eq!(current_status.chat_supported, None);
    }

    fn assert_no_pending_publication(registry: &ProfileRegistry, name: &str) {
        assert!(!registry.paths(name).unwrap().directory.exists());
        let profiles = registry.root.join("profiles");
        if profiles.exists() {
            assert!(fs::read_dir(profiles).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(PROFILE_STAGING_PREFIX)
            }));
        }
    }

    #[test]
    fn checked_profile_publication_is_failure_and_crash_atomic() {
        let _guard = TEST_PROFILE_PUBLICATION_MUTEX.lock().unwrap();
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let addresses = environment.addresses().unwrap();
        let state = environment
            .client_path("profile-publication", "state")
            .unwrap();

        let credentials =
            ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        let unreachable = local_profile(&environment, "unreachable", "localhost:1".to_owned());
        assert!(credentials
            .check_and_add_profile_with_control(
                &mut registry,
                unreachable,
                Duration::from_millis(250),
                CancellationToken::new(),
            )
            .is_err());
        assert!(matches!(
            registry.profile("unreachable"),
            Err(Error::ProfileMissing)
        ));
        assert_no_pending_publication(&registry, "unreachable");

        for (point, name) in [(1, "after-probe"), (2, "after-rename")] {
            TEST_PROFILE_PUBLICATION_CRASH_POINT.store(point, std::sync::atomic::Ordering::SeqCst);
            let profile = local_profile(
                &environment,
                name,
                format!("localhost:{}", addresses.probe.port()),
            );
            assert!(credentials
                .check_and_add_profile(&mut registry, profile)
                .is_err());
            drop(registry);
            registry = ProfileRegistry::open(&state).unwrap();
            assert!(matches!(registry.profile(name), Err(Error::ProfileMissing)));
            assert_no_pending_publication(&registry, name);
        }

        TEST_PROFILE_PUBLICATION_CRASH_POINT.store(3, std::sync::atomic::Ordering::SeqCst);
        let committed = local_profile(
            &environment,
            "after-commit",
            format!("localhost:{}", addresses.probe.port()),
        );
        assert!(credentials
            .check_and_add_profile(&mut registry, committed.clone())
            .is_err());
        drop(registry);
        let registry = ProfileRegistry::open(&state).unwrap();
        assert_eq!(registry.profile("after-commit").unwrap(), &committed);
        let session = ProfileSession::open(&registry, "after-commit").unwrap();
        assert_eq!(
            session
                .client
                .pinned_host("localhost", &session.paths.hard_database)
                .unwrap()
                .lookup_name(),
            "localhost"
        );
        assert!(profile_publication_is_pending(&session).unwrap());
        let resumed = credentials
            .check_and_add_profile(&mut ProfileRegistry::open(&state).unwrap(), committed)
            .unwrap();
        assert_eq!(resumed.probe.lookup_name, "localhost");
        assert!(!profile_publication_is_pending(&session).unwrap());
    }

    #[test]
    fn checked_profile_publication_returns_the_committed_canonical_identity() {
        let _guard = TEST_PROFILE_PUBLICATION_MUTEX.lock().unwrap();
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let addresses = environment.addresses().unwrap();
        let state = environment
            .client_path("profile-publication-success", "state")
            .unwrap();
        let credentials =
            ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        let profile = local_profile(
            &environment,
            "local",
            format!("localhost:{}", addresses.probe.port()),
        );

        let report = credentials
            .check_and_add_profile(&mut registry, profile.clone())
            .unwrap();
        assert_eq!(report.profile, profile);
        assert_eq!(report.probe.acceptance, ProbeAcceptance::Inserted);
        assert_eq!(report.probe.lookup_name, "localhost");
        assert!(!report.probe.canonical_name.is_empty());
        assert_eq!(report.probe.host_id_hex.len(), 66);
        assert_eq!(registry.profile("local").unwrap(), &report.profile);
        assert_eq!(
            hex(ProfileSession::open(&registry, "local")
                .unwrap()
                .client
                .pinned_host("localhost", &registry.paths("local").unwrap().hard_database,)
                .unwrap()
                .host_id()
                .as_bytes()),
            report.probe.host_id_hex
        );
        assert!(!profile_publication_is_pending(
            &ProfileSession::open(&registry, "local").unwrap()
        )
        .unwrap());
    }

    #[test]
    fn checked_profile_publication_rejects_an_unexpected_host_before_commit() {
        let _guard = TEST_PROFILE_PUBLICATION_MUTEX.lock().unwrap();
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let addresses = environment.addresses().unwrap();
        let state = environment
            .client_path("profile-publication-host-binding", "state")
            .unwrap();
        let credentials =
            ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        let profile = local_profile(
            &environment,
            "wrong-host",
            format!("localhost:{}", addresses.probe.port()),
        );
        let mut expected = [0x55; 33];
        expected[0] = 2;
        assert!(credentials
            .check_and_add_profile_for_host_with_control(
                &mut registry,
                profile,
                Duration::from_secs(30),
                CancellationToken::new(),
                Some(&expected),
            )
            .is_err());
        assert!(matches!(
            registry.profile("wrong-host"),
            Err(Error::ProfileMissing)
        ));
        assert!(!registry.paths("wrong-host").unwrap().directory.exists());
    }

    #[test]
    fn checked_profile_publication_enrolls_native_checkpoint_once_and_recovers_after_crash() {
        let _guard = TEST_PROFILE_PUBLICATION_MUTEX.lock().unwrap();
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let addresses = environment.addresses().unwrap();
        let state = environment
            .client_path("profile-publication-native", "state")
            .unwrap();
        let publication_credentials =
            ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        TEST_PROFILE_PUBLICATION_CRASH_POINT.store(3, std::sync::atomic::Ordering::SeqCst);
        assert!(publication_credentials
            .check_and_add_profile(
                &mut registry,
                local_profile(
                    &environment,
                    "local",
                    format!("localhost:{}", addresses.probe.port()),
                ),
            )
            .is_err());
        let session = ProfileSession::open(&registry, "local").unwrap();
        let credentials = ClientCredentials {
            lease: ClientStateLease::acquire(state.canonicalize().unwrap()).unwrap(),
            root: state.canonicalize().unwrap(),
            state_id: "native-memory-test".to_owned(),
            backend: CredentialBackend::Native,
        };
        let current = session.rollback_checkpoint().unwrap();
        let binding = profile_publication_checkpoint_binding(&credentials.root, &session, &current)
            .unwrap()
            .unwrap();
        let marker_path = session.paths.directory.join(PROFILE_PUBLICATION_MARKER);
        let marker = fs::read(&marker_path).unwrap();
        let mut external = MemorySecretStore::default();
        begin_profile_publication_with_store("local", &mut external).unwrap();
        CheckpointStore::put(
            &mut external,
            &profile_publication_authorization_key("local").unwrap(),
            &binding.nonce,
        )
        .unwrap();
        bind_profile_publication_with_store(
            "local",
            &binding.nonce,
            &binding.marker_digest,
            &mut external,
        )
        .unwrap();
        let publication = ProfilePublicationAuthorization {
            authorized: true,
            nonce: binding.nonce,
        };

        TEST_FAIL_AFTER_CHECKPOINT_PUBLICATION.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(credentials
            .verify_native_checkpoint_for_use_with_store(
                &session,
                &current,
                true,
                Some(&publication),
                &mut external,
            )
            .is_err());
        assert!(profile_publication_is_pending(&session).unwrap());
        assert!(
            CheckpointStore::get(&mut external, &rollback_record_key("local").unwrap()).is_ok()
        );
        assert!(CheckpointStore::get(
            &mut external,
            &database_claim_record_key(&current.database_id)
        )
        .is_ok());

        credentials
            .verify_native_checkpoint_for_use_with_store(
                &session,
                &current,
                true,
                Some(&publication),
                &mut external,
            )
            .unwrap();
        assert!(!profile_publication_is_pending(&session).unwrap());

        atomic_private_write(&marker_path, &marker).unwrap();
        CheckpointStore::remove(&mut external, &rollback_record_key("local").unwrap()).unwrap();
        let unauthorized = ProfilePublicationAuthorization {
            authorized: false,
            nonce: binding.nonce,
        };
        assert!(matches!(
            credentials.verify_native_checkpoint_for_use_with_store(
                &session,
                &current,
                false,
                Some(&unauthorized),
                &mut external,
            ),
            Err(Error::CheckpointResetRequired {
                reason: "external checkpoint is missing",
                ..
            })
        ));
        assert!(profile_publication_is_pending(&session).unwrap());

        let mut tampered_marker: serde_json::Value = serde_json::from_slice(&marker).unwrap();
        tampered_marker["probe"]["canonical_name"] = serde_json::json!("attacker.invalid");
        atomic_private_write(&marker_path, &serde_json::to_vec(&tampered_marker).unwrap()).unwrap();
        assert!(matches!(
            profile_publication_checkpoint_binding(&credentials.root, &session, &current),
            Err(Error::InvalidConfig(
                "profile publication probe facts changed"
            ))
        ));
        atomic_private_write(&marker_path, &marker).unwrap();

        let connection = rusqlite::Connection::open(&session.paths.hard_database).unwrap();
        connection
            .execute(
                "UPDATE hard_state_metadata SET hard_state_revision = hard_state_revision + 1, write_token = randomblob(16) WHERE singleton = 1",
                [],
            )
            .unwrap();
        drop(connection);
        let changed = session.rollback_checkpoint().unwrap();
        assert!(matches!(
            profile_publication_checkpoint_binding(&credentials.root, &session, &changed,),
            Err(Error::InvalidConfig(
                "profile publication checkpoint binding changed"
            ))
        ));
    }

    #[test]
    fn checked_profile_publication_serializes_same_name_authorizations_and_cleans_failures() {
        let _guard = TEST_PROFILE_PUBLICATION_MUTEX.lock().unwrap();
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let addresses = environment.addresses().unwrap();
        let state = environment
            .client_path("profile-publication-concurrency", "state")
            .unwrap();
        let profile = local_profile(
            &environment,
            "same-name",
            format!("localhost:{}", addresses.probe.port()),
        );
        let authorizer = std::sync::Arc::new(MemoryPublicationAuthorizer::new());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let mut threads = Vec::new();
        for _ in 0..2 {
            let authorizer = authorizer.clone();
            let barrier = barrier.clone();
            let state = state.clone();
            let profile = profile.clone();
            threads.push(std::thread::spawn(move || {
                let mut registry = ProfileRegistry::open(state).unwrap();
                barrier.wait();
                registry.check_and_add_profile_with_control(
                    authorizer.as_ref(),
                    profile,
                    Duration::from_secs(30),
                    CancellationToken::new(),
                )
            }));
        }
        barrier.wait();
        for thread in threads {
            thread.join().unwrap().unwrap();
        }
        assert_eq!(
            authorizer.begins.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert!(authorizer.contains("same-name"));

        let mut registry = ProfileRegistry::open(&state).unwrap();
        let unreachable = local_profile(&environment, "unreachable-atomic", "localhost:1".into());
        assert!(registry
            .check_and_add_profile_with_control(
                authorizer.as_ref(),
                unreachable,
                Duration::from_millis(250),
                CancellationToken::new(),
            )
            .is_err());
        assert!(!authorizer.contains("unreachable-atomic"));

        registry
            .add(local_profile(
                &environment,
                "already-present",
                format!("localhost:{}", addresses.probe.port()),
            ))
            .unwrap();
        let begins = authorizer.begins.load(std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(
            registry.check_and_add_profile_with_control(
                authorizer.as_ref(),
                registry.profile("already-present").unwrap().clone(),
                Duration::from_secs(30),
                CancellationToken::new(),
            ),
            Err(Error::ProfileExists)
        ));
        assert_eq!(
            authorizer.begins.load(std::sync::atomic::Ordering::SeqCst),
            begins
        );
        assert!(!authorizer.contains("already-present"));

        let lease_profile = current_probe_only_profile(
            &environment,
            "lease-race",
            format!("localhost:{}", addresses.probe.port()),
        );
        TEST_PROFILE_PUBLICATION_CRASH_POINT.store(3, std::sync::atomic::Ordering::SeqCst);
        assert!(registry
            .check_and_add_profile_with_control(
                authorizer.as_ref(),
                lease_profile.clone(),
                Duration::from_secs(30),
                CancellationToken::new(),
            )
            .is_err());
        assert!(profile_publication_marker_exists(&state, "lease-race").unwrap());

        let seed = [
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ];
        let lease = SignedCompatibilityArtifact::sign(
            foks_compat_artifact::CompatibilityArtifact {
                schema_version: foks_compat_artifact::SCHEMA_VERSION,
                generation: 1,
                target: lease_profile.probe.clone(),
                run_id: "publication-race".to_owned(),
                generated_at: 100,
                expires_at: 200,
                protocol_metadata_sha256: PINNED_PROTOCOL_METADATA_SHA256.to_owned(),
                mutation_digest: "22".repeat(32),
                read_digest: "33".repeat(32),
                outcome: CompatibilityOutcome::Drift,
                capabilities: BTreeSet::new(),
                drift_reason: "deterministic test drift".to_owned(),
            },
            &seed,
        )
        .unwrap();
        assert!(matches!(
            registry.apply_compatibility_artifact("lease-race", &lease, 101),
            Err(Error::InvalidConfig(
                "profile publication checkpoint is still pending"
            ))
        ));
        assert_eq!(registry.profile("lease-race").unwrap(), &lease_profile);
        assert!(profile_publication_marker_exists(&state, "lease-race").unwrap());
    }

    #[test]
    fn forgetting_a_profile_erases_its_local_state() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(Profile {
                name: "local".to_owned(),
                label: None,
                probe: "foks.example.test".to_owned(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();

        // The artifacts a used profile leaves behind. The credential store is
        // the one that matters: it holds this Mac's device keys. The rollback
        // checkpoint has to be real rather than a stub file, or the removal
        // silently skips the database locks and the claim record it owns.
        let session = ProfileSession::open(&registry, "local").unwrap();
        let paths = session.paths().clone();
        session.rollback_checkpoint().unwrap();
        drop(session);
        assert!(paths.hard_database.is_file());
        fs::create_dir_all(&paths.credential_store).unwrap();
        fs::write(paths.credential_store.join("account.personal"), b"key").unwrap();
        fs::create_dir_all(&paths.protected_mutations).unwrap();
        fs::write(&paths.soft_database, b"soft").unwrap();

        assert!(credentials.remove_profile(&mut registry, "local").unwrap());
        assert!(matches!(
            registry.profile("local"),
            Err(Error::ProfileMissing)
        ));
        assert!(!paths.directory.exists());

        // A second remove is idempotent: it returns false when the profile is already
        // gone, so a removal interrupted before the registry write can be retried.
        assert!(!credentials.remove_profile(&mut registry, "local").unwrap());
    }

    #[test]
    fn forgetting_a_profile_refuses_a_pending_publication() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let credentials =
            ClientCredentials::initialize(&root, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&root).unwrap();
        registry
            .add(Profile {
                name: "local".to_owned(),
                label: None,
                probe: "foks.example.test".to_owned(),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::WebPki,
            })
            .unwrap();
        let session = ProfileSession::open(&registry, "local").unwrap();
        let paths = session.paths().clone();
        fs::create_dir_all(&paths.credential_store).unwrap();
        fs::write(paths.credential_store.join("account.personal"), b"key").unwrap();
        write_profile_publication_marker(
            &paths.directory.join(PROFILE_PUBLICATION_MARKER),
            session.profile(),
            &ProbeReport {
                acceptance: ProbeAcceptance::Inserted,
                lookup_name: "foks.example.test".to_owned(),
                canonical_name: "foks.example.test".to_owned(),
                host_id_hex: "02".repeat(33),
                host_chain_sequence: 1,
                merkle_epoch: 1,
                server_version: None,
            },
            &session.rollback_checkpoint().unwrap(),
            [7; 32],
        )
        .unwrap();

        assert!(matches!(
            credentials.remove_profile(&mut registry, "local"),
            Err(Error::InvalidConfig(
                "profile publication checkpoint is still pending"
            ))
        ));
        assert!(registry.profile("local").is_ok());
        assert!(paths.credential_store.join("account.personal").is_file());
    }
}
