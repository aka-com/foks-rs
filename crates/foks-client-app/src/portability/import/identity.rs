use super::*;
use serde::{Deserialize, Serialize};
pub(super) const MARKER: &str = ".state-import-v1";
pub(crate) const INTENT: &str = "state-import-v1";
pub(crate) const RECEIPT: &str = "state-import-receipt-v1";
pub(super) const CLAIM_PREFIX: &str = "pending-import-claim.";
pub(super) const MAX_METADATA: u64 = 32 * 1024;
const IDENTITY_MAC_DOMAIN: u64 = 0x7b93_b685_8164_13ca;

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    pub version: u32,
    pub nonce: [u8; 32],
    pub state_id: String,
    pub archive_id: [u8; 16],
    pub source_state_id: String,
    pub source_projection_digest: [u8; 32],
    pub destination: PathBuf,
    pub staging: PathBuf,
    pub empty_hold: PathBuf,
    pub parent: DirectoryIdentity,
    pub root: DirectoryIdentity,
    pub empty_destination: Option<DirectoryIdentity>,
    pub select_desktop: bool,
}
impl Identity {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || !self.destination.is_absolute()
            || self.destination == self.staging
            || self.destination == self.empty_hold
            || self.root.device != self.parent.device
        {
            return Err(Error::StateRecoveryRequired);
        }
        crate::validate_name(&self.state_id)?;
        crate::validate_name(&self.source_state_id)?;
        let parent = self.destination.parent().ok_or(Error::StatePathChanged)?;
        let nonce = crate::hex(&self.nonce);
        if self.staging != parent.join(format!(".foks-import-{nonce}"))
            || self.empty_hold != parent.join(format!(".foks-import-empty-{nonce}"))
        {
            return Err(Error::StateRecoveryRequired);
        }
        Ok(())
    }
    pub fn verify_parent(&self) -> Result<()> {
        self.validate()?;
        if canonical_reservation(&self.destination)? != self.destination
            || DirectoryIdentity::read(self.destination.parent().ok_or(Error::StatePathChanged)?)?
                != self.parent
        {
            return Err(Error::StatePathChanged);
        }
        Ok(())
    }
    pub fn tag(&self, source_master: &[u8; 32]) -> Result<[u8; 32]> {
        Ok(foks_crypto::capability_mac(
            source_master,
            IDENTITY_MAC_DOMAIN,
            &serde_json::to_vec(self)?,
        ))
    }
    pub fn paths(&self) -> [PathBuf; 3] {
        [
            self.destination.clone(),
            self.staging.clone(),
            self.empty_hold.clone(),
        ]
    }
}
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub(super) enum Phase {
    Staging,
    Prepared,
    FilesInstalled,
    ClaimsInstalled,
    Verified,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Marker {
    pub identity: Identity,
    pub identity_mac: [u8; 32],
    pub phase: Phase,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Intent {
    pub marker: Marker,
    pub content_digest: [u8; 32],
    pub claims_digest: [u8; 32],
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Locator {
    pub kind: String,
    pub marker: Marker,
}
impl Locator {
    pub fn decode(path: &Path) -> Result<Self> {
        let value: Self =
            serde_json::from_slice(&crate::read_bounded_regular_file(path, MAX_METADATA)?)?;
        if value.kind != "import" {
            return Err(Error::StateRecoveryRequired);
        }
        value.marker.identity.validate()?;
        Ok(value)
    }
}
pub(super) fn native_intent(native: &NativeManifestStore, key: &str) -> Result<Option<Intent>> {
    native
        .records
        .get(key)
        .map(|bytes| {
            let intent: Intent = serde_json::from_slice(bytes)?;
            intent.marker.identity.validate()?;
            if intent.marker.phase == Phase::Staging {
                return Err(Error::StateRecoveryRequired);
            }
            Ok(intent)
        })
        .transpose()
}
pub(super) fn claims_digest(records: &BTreeMap<String, Vec<u8>>) -> Result<[u8; 32]> {
    Ok(Sha256::digest(serde_json::to_vec(records)?).into())
}
pub(super) fn content_digest(snapshot: &StateSnapshot) -> Result<[u8; 32]> {
    let mut hash = Sha256::new();
    hash.update(b"foks-import-content-v1");
    hash.update(serde_json::to_vec(&snapshot.artifacts)?);
    for (key, value) in &snapshot.native.records {
        if matches!(key.as_str(), crate::STATE_ROOT_RECORD | INTENT | RECEIPT) {
            continue;
        }
        hash.update((key.len() as u64).to_be_bytes());
        hash.update(key.as_bytes());
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value);
    }
    Ok(hash.finalize().into())
}
pub(super) fn write_marker(
    guard: &mut ClientStateMaintenanceGuard,
    root: &Path,
    marker: &Marker,
    hook: &mut Hook<'_>,
) -> Result<()> {
    let path = root.join(MARKER);
    if files::exists(&path)? {
        let existing: Marker =
            serde_json::from_slice(&crate::read_bounded_regular_file(&path, MAX_METADATA)?)?;
        if existing.identity != marker.identity
            || existing.identity_mac != marker.identity_mac
            || existing.phase > marker.phase
        {
            return Err(Error::StateRecoveryRequired);
        }
    }
    let bytes = serde_json::to_vec(marker)?;
    durable_metadata(
        &path,
        &bytes,
        &marker.identity.nonce,
        hook,
        "before-import-marker",
        "after-import-marker",
    )?;
    guard.import_nonce = Some(marker.identity.nonce);
    guard.import_marker_digest = Some(Sha256::digest(bytes).into());
    Ok(())
}
pub(super) fn write_locator(
    guard: &ClientStateMaintenanceGuard,
    marker: &Marker,
    hook: &mut Hook<'_>,
) -> Result<()> {
    let path = locator_path(&guard.base, &marker.identity.destination);
    if files::exists(&path)? {
        let old = Locator::decode(&path)?;
        if old.marker.identity != marker.identity || old.marker.identity_mac != marker.identity_mac
        {
            return Err(Error::StateRecoveryRequired);
        }
    }
    // Discovery remains the immutable Staging record even as native phases advance.
    let mut discovery = marker.clone();
    discovery.phase = Phase::Staging;
    let bytes = serde_json::to_vec(&Locator {
        kind: "import".into(),
        marker: discovery,
    })?;
    durable_metadata(
        &path,
        &bytes,
        &marker.identity.nonce,
        hook,
        "before-import-locator",
        "after-import-locator",
    )
}
pub(super) fn sync_parent(identity: &Identity, hook: &mut Hook<'_>) -> Result<()> {
    identity.verify_parent()?;
    hook("before-import-parent-sync")?;
    File::open(
        identity
            .destination
            .parent()
            .ok_or(Error::StatePathChanged)?,
    )?
    .sync_all()?;
    hook("after-import-parent-sync")
}
