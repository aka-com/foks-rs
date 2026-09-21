//! Batched change marker for roster members' user chains.
//!
//! The team refresh pass compares each member's current shared-key generation
//! against the generation frozen in the team roster. That comparison needs the
//! member's current user state, which the pass used to obtain by reloading
//! every member's chain on every pass. The team chain's own sequence number is
//! not a usable marker: a member's key rotation advances that member's *user*
//! chain and leaves the team chain where it is.
//!
//! The marker used instead is the pair of absence proofs a user-chain response
//! must already carry before [`foks_verify::verify_user_chain`] will treat it
//! as terminal -- no link one past the pinned tail, and no further claim on the
//! pinned username's ownership sequence. Both keys are derivable from pinned
//! state alone, so a whole pass costs one batched Merkle lookup rather than one
//! root advance and one chain load per member.

use foks_proto::{
    EntityId, MerkleMultiLookupResponse, MerkleRoot, ENTITY_USER, MERKLE_ROOT_TYPE_ID,
};
use foks_verify::{
    classify_user_chain_tail, user_chain_next_link_key, user_chain_next_username_key,
    UserChainTail, VerifiedMerkleAdvance, VerifiedUserState,
};

use crate::{DeviceCredential, Error, FoksClient, PinnedHost, Result, YubiCredential};

/// Tails per batched lookup. Each contributes two keys and the protocol caps a
/// multi-lookup at 4096 keys, so any realistic roster is a single request; this
/// bound only keeps a pathological roster from being rejected outright instead
/// of probed in pieces.
const MAXIMUM_PROBE_TAILS: usize = 256;

/// The two absence keys that together prove a pinned user state is still what a
/// fresh load would return: no further chain link, and no further claim on the
/// username whose ownership sequence that state records.
struct ProbeKeys {
    chain: [u8; 32],
    username: [u8; 32],
}

impl ProbeKeys {
    fn derive(pinned: &VerifiedUserState) -> Result<Self> {
        Ok(Self {
            chain: user_chain_next_link_key(pinned)?,
            username: user_chain_next_username_key(pinned)?,
        })
    }
}

impl FoksClient {
    /// Classifies each supplied user-chain tail at `latest`'s epoch, with one
    /// batched Merkle lookup per 256 eligible tails.
    ///
    /// `latest` must be an advance this caller authenticated for this pass, and
    /// must not be older than the epoch the surrounding roster was read at, or
    /// a member could be reported current against a roster that already names a
    /// newer generation. The lookup is issued at its epoch and the returned
    /// root is re-checked against it, so the absence proofs are evidence under
    /// the same root the pass already trusts rather than under one the server
    /// chose.
    ///
    /// A tail reported [`UserChainTail::Unchanged`] is still that user's head
    /// at `latest`'s epoch, so the caller may reuse the pinned state with no
    /// chain load. Everything else is [`UserChainTail::Advanced`] and must be
    /// loaded: a tail already pinned past `latest` -- whose absence key would
    /// prove nothing about state the caller holds -- and any answer short of a
    /// proof. A server can therefore at worst cost the caller the work it would
    /// have done anyway.
    pub fn probe_user_chain_tails(
        &self,
        host: &PinnedHost,
        latest: &VerifiedMerkleAdvance,
        pinned: &[VerifiedUserState],
    ) -> Result<Vec<UserChainTail>> {
        let eligible = pinned
            .iter()
            .enumerate()
            .filter(|(_, state)| state.tree_root().epoch <= latest.root().epoch)
            .map(|(index, state)| Ok((index, ProbeKeys::derive(state)?)))
            .collect::<Result<Vec<_>>>()?;
        let mut outcome = vec![UserChainTail::Advanced; pinned.len()];
        for batch in eligible.chunks(MAXIMUM_PROBE_TAILS) {
            let query = batch
                .iter()
                .flat_map(|(_, keys)| [keys.chain, keys.username])
                .collect::<Vec<_>>();
            let response =
                self.merkle_multi_lookup(host, &query, false, Some(latest.root().epoch))?;
            let keys = batch.iter().map(|(_, keys)| keys);
            for ((index, _), mark) in batch
                .iter()
                .zip(classify_probe_response(latest, &response, keys)?)
            {
                outcome[*index] = mark;
            }
        }
        Ok(outcome)
    }

    /// Reports, for each target in order, the pinned user state that is
    /// provably still that user's chain head at `latest`'s epoch, or `None`
    /// when the caller must load the chain.
    ///
    /// A target with no replayable pinned tail, or one pinned against a root
    /// newer than `latest`, yields `None`: its absence keys would either be
    /// underivable or would prove nothing about state the caller already holds.
    /// Duplicate targets are probed once per position and reported at each.
    pub fn probe_pinned_user_chains(
        &self,
        host: &PinnedHost,
        latest: &VerifiedMerkleAdvance,
        targets: &[EntityId],
    ) -> Result<Vec<Option<VerifiedUserState>>> {
        let mut candidates = Vec::with_capacity(targets.len());
        for target in targets {
            target.clone().require_type(ENTITY_USER)?;
            // A tail whose persisted evidence no longer replays gives no
            // trustworthy location to build the absence key from, so fall back
            // to a full load exactly as the chain loader does for these errors.
            let candidate = match self.pinned_user(host, target) {
                Ok(state) => state,
                Err(Error::Verify(
                    foks_verify::Error::PersistedMerkleEvidence
                    | foks_verify::Error::UserChainContinuity,
                )) => None,
                Err(error) => return Err(error),
            };
            candidates.push(candidate);
        }
        let probed = candidates.iter().flatten().cloned().collect::<Vec<_>>();
        let mut marks = self
            .probe_user_chain_tails(host, latest, &probed)?
            .into_iter();
        candidates
            .into_iter()
            .map(|candidate| {
                let Some(state) = candidate else {
                    return Ok(None);
                };
                let mark = marks
                    .next()
                    .ok_or(Error::UserBinding("change-marker result is short"))?;
                Ok((mark == UserChainTail::Unchanged).then_some(state))
            })
            .collect()
    }

    /// Loads each target's user chain through an already activated team-view
    /// token, skipping the load for every target whose chain provably has not
    /// moved since it was pinned.
    ///
    /// One pass costs one Merkle root advance plus one batched lookup, then one
    /// chain load per member that actually moved, in place of a root advance
    /// and a chain load for every member.
    pub fn load_and_pin_users_as_local_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        targets: &[EntityId],
        team_view_token: &[u8; 16],
    ) -> Result<Vec<VerifiedUserState>> {
        self.load_marked_users(host, targets, |target| {
            self.load_and_pin_user_as_local_team(host, credential, target, team_view_token)
        })
    }

    /// Hardware-backed transport variant of
    /// [`Self::load_and_pin_users_as_local_team`].
    pub fn load_and_pin_users_as_local_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        targets: &[EntityId],
        team_view_token: &[u8; 16],
    ) -> Result<Vec<VerifiedUserState>> {
        self.load_marked_users(host, targets, |target| {
            self.load_and_pin_user_as_local_team_yubi(host, credential, target, team_view_token)
        })
    }

    fn load_marked_users(
        &self,
        host: &PinnedHost,
        targets: &[EntityId],
        load: impl Fn(&EntityId) -> Result<VerifiedUserState>,
    ) -> Result<Vec<VerifiedUserState>> {
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let (_, latest) = self.advance_merkle_root(host)?;
        let marked = match self.probe_pinned_user_chains(host, &latest, targets) {
            Ok(marked) => marked,
            // A host that cannot answer the probe -- an unimplemented method,
            // or an epoch it has already pruned -- must not wedge the refresh.
            // Loading every chain is what this did before the marker existed,
            // and each load authenticates itself. A forged root or a malformed
            // batch is not a service failure and is not absorbed here.
            Err(Error::Rpc(_)) => vec![None; targets.len()],
            Err(error) => return Err(error),
        };
        marked
            .into_iter()
            .zip(targets)
            .map(|(unchanged, target)| match unchanged {
                Some(state) => Ok(state),
                None => load(target),
            })
            .collect()
    }
}

/// Classifies one batched lookup against the epoch this pass authenticated.
///
/// The request names an epoch, but honouring it is the server's choice, and an
/// absence proof is evidence only under a root the client has independently
/// authenticated. Every path is therefore replayed against the root node of
/// `latest`, the signed root this pass verified, and only after the returned
/// root is checked to be that same root. Skipping the check would let a server
/// answer under a root of its own making, or replay a stale root from before a
/// member's chain advanced -- where the probed link genuinely was absent, so
/// the proof is well formed -- and have that read as evidence that a chain
/// which has since moved is still at its pinned tail.
fn classify_probe_response<'a>(
    latest: &VerifiedMerkleAdvance,
    response: &MerkleMultiLookupResponse,
    keys: impl ExactSizeIterator<Item = &'a ProbeKeys>,
) -> Result<Vec<UserChainTail>> {
    authenticate_probe_root(latest, &response.root)?;
    let expected = keys
        .len()
        .checked_mul(2)
        .ok_or(Error::UserBinding("change-marker key count overflow"))?;
    if response.paths.len() != expected {
        return Err(Error::UserBinding(
            "Merkle change-marker lookup returned the wrong number of paths",
        ));
    }
    Ok(response
        .paths
        .chunks_exact(2)
        .zip(keys)
        .map(|(paths, keys)| {
            // Both absences are required. Either one alone would leave a new
            // chain link or a new username claim unobserved, and the reused
            // state would then differ from what a load returns.
            let proved = [(&paths[0], &keys.chain), (&paths[1], &keys.username)]
                .into_iter()
                .all(|(path, key)| {
                    classify_user_chain_tail(path, key, &response.root.root_node)
                        == UserChainTail::Unchanged
                });
            if proved {
                UserChainTail::Unchanged
            } else {
                UserChainTail::Advanced
            }
        })
        .collect())
}

/// Requires the returned root to be the pass's own authenticated root.
///
/// Equality with `latest.root()` is the binding; the root-set lookup restates
/// the invariant a [`VerifiedMerkleAdvance`] carries, so a future change that
/// let its root drift from the set it authenticated fails here rather than
/// quietly widening what the marker treats as authenticated.
fn authenticate_probe_root(latest: &VerifiedMerkleAdvance, root: &MerkleRoot) -> Result<()> {
    let hash = foks_crypto::prefixed_hash_signable(MERKLE_ROOT_TYPE_ID, &root.encoded()?)?;
    if root != latest.root() || latest.authenticated_roots().root_hash(root.epoch) != Some(hash) {
        return Err(Error::UserBinding(
            "Merkle change-marker lookup returned an unauthenticated root",
        ));
    }
    Ok(())
}
