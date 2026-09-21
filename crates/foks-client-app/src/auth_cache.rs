//! Read-path reuse of authentication outcomes.
//!
//! This crate defines only the shape of the caches; it never owns one. An
//! embedder that runs many read operations against the same profile (the
//! agent) supplies an implementation for read operations alone, so the CLI and
//! every other caller keep the previous behavior of authenticating inline.
//!
//! Two kinds of material are cached, both of which are treated as secret: an
//! [`AuthenticatedUserOutcome`], which carries the user's PUK seeds, and a team
//! view token, which is a server-side bearer capability. Neither is ever
//! logged, serialized or written to disk, and an implementation must drop its
//! entries on eviction so the seeds inside `Zeroizing` are cleared.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use foks_client::{AuthenticatedUserOutcome, DeviceCredential, PinnedHost, TeamViewGrant};

use foks_crypto::prefixed_hash;
use foks_proto::EntityId;

use crate::Result;

const AUTH_CACHE_BINDING_TYPE_ID: u64 = 0x4a1e_0c5b_9d73_2f16;

/// Identifies one authenticated user outcome.
///
/// `binding` digests everything about the acting credential that the outcome
/// depends on: its key kind, enrolled device identity, hybrid encryption
/// public key and mTLS certificate chain. A credential that differs in any of
/// those cannot collide with a cached outcome, and no key material of any kind
/// enters the digest.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AuthCacheKey {
    pub state_root: PathBuf,
    pub profile: String,
    pub host_id: Vec<u8>,
    pub uid: Vec<u8>,
    pub binding: [u8; 32],
}

impl AuthCacheKey {
    pub fn new(
        state_root: &Path,
        profile: &str,
        host_id: &EntityId,
        credential: &DeviceCredential,
    ) -> Result<Self> {
        let derived = credential.public_material()?;
        let mut binding = Vec::new();
        binding.extend_from_slice(&[match credential.key_kind {
            foks_crypto::SoftwareKeyKind::Device => 1,
            foks_crypto::SoftwareKeyKind::BotToken => 2,
        }]);
        binding.extend_from_slice(derived.id.as_bytes());
        binding.extend_from_slice(&derived.hepk.encoded()?);
        for certificate in &credential.certificate_chain {
            binding.extend_from_slice(&(certificate.len() as u64).to_be_bytes());
            binding.extend_from_slice(certificate);
        }
        Ok(Self {
            state_root: state_root.to_path_buf(),
            profile: profile.to_owned(),
            host_id: host_id.as_bytes().to_vec(),
            uid: credential.uid.as_bytes().to_vec(),
            binding: prefixed_hash(AUTH_CACHE_BINDING_TYPE_ID, &binding),
        })
    }
}

/// Identifies one activated team view.
///
/// The actor is the party the view was activated for, so a user actor and a
/// local-team actor never share an entry. The credential `binding` is the same
/// digest as in [`AuthCacheKey`]: the server accepts a view token only from the
/// device that activated it, so a token must never be replayed over another
/// credential's transport.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TeamViewCacheKey {
    pub state_root: PathBuf,
    pub profile: String,
    pub host_id: Vec<u8>,
    pub actor: Vec<u8>,
    pub team_id: Vec<u8>,
    pub binding: [u8; 32],
}

impl TeamViewCacheKey {
    pub fn for_user(user: &AuthCacheKey, team_id: &EntityId) -> Self {
        Self {
            state_root: user.state_root.clone(),
            profile: user.profile.clone(),
            host_id: user.host_id.clone(),
            actor: user.uid.clone(),
            team_id: team_id.as_bytes().to_vec(),
            binding: user.binding,
        }
    }
}

/// A bounded, expiring store of authenticated user outcomes.
///
/// Implementations must bound both the entry count and the entry lifetime, and
/// must serve only read operations: a mutating operation is never given a
/// cache, so it always authenticates against the current server state.
pub trait AuthenticatedUserCache: Send + Sync {
    fn get(&self, key: &AuthCacheKey) -> Option<Arc<AuthenticatedUserOutcome>>;
    fn put(&self, key: AuthCacheKey, outcome: Arc<AuthenticatedUserOutcome>);
    /// Drops every entry for one profile. Callers invalidate after any
    /// operation that could have changed the user's devices or PUKs.
    fn invalidate_profile(&self, state_root: &Path, profile: &str);
}

/// A bounded, expiring store of activated team views. The lifetime must stay
/// well inside the server's own view lifetime so a cached token is refused
/// only in the rare case of a role change or a team key rotation.
pub trait TeamViewTokenCache: Send + Sync {
    fn get(&self, key: &TeamViewCacheKey) -> Option<TeamViewGrant>;
    fn put(&self, key: TeamViewCacheKey, grant: TeamViewGrant);
    fn invalidate(&self, key: &TeamViewCacheKey);
    fn invalidate_profile(&self, state_root: &Path, profile: &str);
}

/// The caches a session may consult. Both are absent by default.
#[derive(Clone, Default)]
pub struct ReadCaches {
    pub authenticated_users: Option<Arc<dyn AuthenticatedUserCache>>,
    pub team_view_tokens: Option<Arc<dyn TeamViewTokenCache>>,
}

impl std::fmt::Debug for ReadCaches {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadCaches")
            .field("authenticated_users", &self.authenticated_users.is_some())
            .field("team_view_tokens", &self.team_view_tokens.is_some())
            .finish()
    }
}

impl crate::CheckedProfileSession<'_> {
    /// The authenticated user for a read operation.
    ///
    /// Without a cache this is exactly `authenticate_and_pin`. With one, the
    /// outcome is reused for the cache's lifetime, which removes the signed
    /// Merkle root, user chain and PUK parcel round trips from every read
    /// after the first. A reused outcome also skips that Merkle advance: a
    /// team load performs its own, but the personal KV catalog does not, so
    /// its freshness evidence can be as old as the cache's lifetime. Every
    /// payload is still opened with keys from the verified state and every
    /// call still presents the device credential, so a key or device
    /// superseded inside that window produces a failed read, never a wrong
    /// result.
    pub fn authenticated_user(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<Arc<AuthenticatedUserOutcome>> {
        self.authenticated_user_with_reuse(host, credential, true)
    }

    /// [`Self::authenticated_user`] that authenticates against the server even
    /// on a cache hit, and replaces the retained outcome with the result. It
    /// is used where a retained outcome has already been shown to be stale.
    pub fn authenticated_user_fresh(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<Arc<AuthenticatedUserOutcome>> {
        self.authenticated_user_with_reuse(host, credential, false)
    }

    fn authenticated_user_with_reuse(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        reuse: bool,
    ) -> Result<Arc<AuthenticatedUserOutcome>> {
        let Some(cache) = self.session.read_caches.authenticated_users.as_ref() else {
            return Ok(Arc::new(
                self.client.authenticate_and_pin(host, credential)?,
            ));
        };
        let key = AuthCacheKey::new(
            self.session.lease.root(),
            &self.session.profile.name,
            host.host_id(),
            credential,
        )?;
        if reuse {
            if let Some(outcome) = cache.get(&key) {
                return Ok(outcome);
            }
        }
        let outcome = Arc::new(self.client.authenticate_and_pin(host, credential)?);
        cache.put(key, Arc::clone(&outcome));
        Ok(outcome)
    }

    /// Loads a team for a read operation, reusing an activated view when one
    /// is retained for this actor and team. A retained view that the server or
    /// the local key material refuses is dropped and replaced by one fresh
    /// activation, so a role change or a team key rotation costs one extra
    /// load rather than a wrong result.
    pub(crate) fn load_team_for_read(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &AuthenticatedUserOutcome,
        team_id: &EntityId,
    ) -> Result<foks_client::AuthenticatedTeamOutcome> {
        let Some(cache) = self.session.read_caches.team_view_tokens.as_ref() else {
            return Ok(self.client.load_and_pin_team(
                host,
                credential,
                &user.verified,
                &user.puks,
                team_id,
            )?);
        };
        let key = TeamViewCacheKey::for_user(
            &AuthCacheKey::new(
                self.session.lease.root(),
                &self.session.profile.name,
                host.host_id(),
                credential,
            )?,
            team_id,
        );
        if let Some(grant) = cache.get(&key) {
            match self.client.load_and_pin_team_with_grant(
                host,
                credential,
                &user.verified,
                &user.puks,
                team_id,
                &grant,
            ) {
                Ok(team) => return Ok(team),
                Err(error) => {
                    cache.invalidate(&key);
                    // A cancelled or expired operation is not a refusal of the
                    // view. Activating a fresh one would only spend what is
                    // left of the caller's budget on the same answer.
                    if matches!(
                        error,
                        foks_client::Error::Cancelled | foks_client::Error::DeadlineExceeded
                    ) {
                        return Err(error.into());
                    }
                }
            }
        }
        let (team, grant) = self.client.load_and_pin_team_recording_view(
            host,
            credential,
            &user.verified,
            &user.puks,
            team_id,
        )?;
        cache.put(key, grant);
        Ok(team)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use foks_crypto::SoftwareKeyKind;
    use foks_proto::SecretSeed;

    fn entity(byte: u8) -> EntityId {
        let mut bytes = vec![foks_proto::ENTITY_USER];
        bytes.extend_from_slice(&[byte; 32]);
        EntityId::from_bytes(bytes).expect("entity identifier is well formed")
    }

    fn credential(seed: u8) -> DeviceCredential {
        DeviceCredential {
            key_kind: SoftwareKeyKind::Device,
            uid: entity(1),
            seed: SecretSeed::new([seed; 32]),
            certificate_chain: vec![vec![seed; 8]],
        }
    }

    fn key(
        state_root: &str,
        profile: &str,
        host: u8,
        credential: &DeviceCredential,
    ) -> AuthCacheKey {
        AuthCacheKey::new(Path::new(state_root), profile, &entity(host), credential)
            .expect("cache key is derived")
    }

    #[test]
    fn a_cache_key_is_stable_for_one_credential_and_profile() {
        assert_eq!(
            key("/state", "local", 9, &credential(3)),
            key("/state", "local", 9, &credential(3))
        );
    }

    #[test]
    fn a_cache_key_separates_every_binding_it_covers() {
        let base = key("/state", "local", 9, &credential(3));
        assert_ne!(base, key("/other", "local", 9, &credential(3)));
        assert_ne!(base, key("/state", "remote", 9, &credential(3)));
        assert_ne!(base, key("/state", "local", 8, &credential(3)));
        assert_ne!(base, key("/state", "local", 9, &credential(4)));

        let mut other_uid = credential(3);
        other_uid.uid = entity(2);
        assert_ne!(base, key("/state", "local", 9, &other_uid));

        let mut other_chain = credential(3);
        other_chain.certificate_chain = vec![vec![9; 8]];
        assert_ne!(base, key("/state", "local", 9, &other_chain));

        let mut other_kind = credential(3);
        other_kind.key_kind = SoftwareKeyKind::BotToken;
        assert_ne!(base, key("/state", "local", 9, &other_kind));
    }

    #[test]
    fn a_team_view_key_follows_its_user_key_and_its_team() {
        let user = key("/state", "local", 9, &credential(3));
        assert_eq!(
            TeamViewCacheKey::for_user(&user, &entity(5)),
            TeamViewCacheKey::for_user(&user, &entity(5))
        );
        assert_ne!(
            TeamViewCacheKey::for_user(&user, &entity(5)),
            TeamViewCacheKey::for_user(&user, &entity(6))
        );
        assert_ne!(
            TeamViewCacheKey::for_user(&user, &entity(5)),
            TeamViewCacheKey::for_user(&key("/state", "local", 9, &credential(4)), &entity(5))
        );
    }
}
