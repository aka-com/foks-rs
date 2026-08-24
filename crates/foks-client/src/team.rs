//! Team loading, PTK recovery, ad-hoc creation, and reconciliation.

mod bearer;
mod membership;
mod named;
mod rotation;

pub use membership::*;
pub use named::*;
pub use rotation::*;

use std::time::Duration;

use foks_client_db::{Acceptance, AdHocTeamOperation, AdHocTeamOperationState, HardStateStore};
use foks_crypto::{
    adhoc_team_id_from_admin_seed, derive_device_public, derive_subkey_id, hepk_fingerprint,
    make_single_owner_adhoc_team, make_single_owner_adhoc_team_yubi, open_shared_key_parcel_with,
    open_shared_key_seed_chain, prefixed_hash, seal_shared_key_boxes, sign_shared_key_typed,
    AdHocTeamInput, AdHocTeamMaterial, PukBoxRandomness, SharedKeyBoxInput, SharedKeyDecapsulator,
};
use foks_proto::{
    ActivatedTeamView, AdHocTeamCreateArgument, EntityId, HostConfig, Role, SecretSeed, TeamChain,
    TeamViewChallenge, TeamViewRequest, ViewershipMode, ENTITY_PTK_VERIFY, ENTITY_PUK_VERIFY,
    TEAM_VIEW_CHALLENGE_TYPE_ID,
};
use foks_rpc::{
    encode_activate_team_view_request, encode_create_adhoc_team_request,
    encode_get_host_config_request, encode_load_team_chain_request_from,
    encode_team_view_challenge_request, STATUS_TX_RETRY_ERROR,
};
use foks_verify::{
    verify_team_chain, verify_team_chain_increment, VerifiedTeamState, VerifiedUserState,
};

use crate::{
    current_owner_puk, now_microseconds, random_bytes, user_key_for_seed, AuthenticatedUserOutcome,
    DeviceCredential, Error, FoksClient, PinnedHost, Result, UserPrivateKey, YubiCredential,
    ADHOC_TEAM_OPERATION_ID_TYPE_ID, ADHOC_TEAM_REQUEST_HASH_TYPE_ID,
};

pub struct TeamPrivateKey {
    pub role: Role,
    pub generation: u64,
    pub seed: SecretSeed,
}

pub struct AuthenticatedTeamOutcome {
    pub merkle_acceptance: Acceptance,
    pub acceptance: Acceptance,
    pub verified: VerifiedTeamState,
    pub ptks: Vec<TeamPrivateKey>,
    pub view_token: [u8; 16],
}

/// Caller-durable, role-keyed PTK seeds for a single-owner ad-hoc team.
pub struct AdHocTeamSecrets {
    pub member_min: SecretSeed,
    pub member: SecretSeed,
    pub admin: SecretSeed,
    pub owner: SecretSeed,
}

impl AdHocTeamSecrets {
    fn ordered(&self) -> [&SecretSeed; 4] {
        [&self.member_min, &self.member, &self.admin, &self.owner]
    }

    /// Returns the stable TeamID before any network mutation is attempted.
    pub fn team_id(&self) -> Result<EntityId> {
        Ok(adhoc_team_id_from_admin_seed(&self.admin)?)
    }

    /// Stable public journal identifier recoverable from retained PTKs even
    /// when the process loses the create call's return value.
    pub fn operation_id(&self) -> Result<[u8; 16]> {
        let hash = prefixed_hash(ADHOC_TEAM_OPERATION_ID_TYPE_ID, self.team_id()?.as_bytes());
        Ok(hash[..16].try_into().expect("hash prefix has fixed length"))
    }
}

pub struct CreatedAdHocTeam {
    pub operation_id: [u8; 16],
    pub team: EntityId,
    pub authenticated: AuthenticatedTeamOutcome,
}

impl FoksClient {
    /// Acquires a short-lived view token with the current device-role PUK,
    /// loads and verifies a team chain, unboxes exactly the PTKs visible to
    /// this member, and atomically seals the public team projection in SQLite.
    pub fn load_and_pin_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
        team: &EntityId,
    ) -> Result<AuthenticatedTeamOutcome> {
        let puk_seed = &puks
            .last()
            .ok_or(Error::KeyBinding("no user private key is available"))?
            .seed;
        self.load_and_pin_team_with_material(
            host,
            &credential.uid,
            &credential.seed,
            &credential.certificate_chain,
            user,
            puk_seed,
            team,
        )
    }

    /// Yubi variant of [`Self::load_and_pin_team`]. The delegated software
    /// subkey is used only for mTLS; the already-unboxed PUK signs the team
    /// view challenge and decrypts returned PTKs.
    pub fn load_and_pin_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        user: &VerifiedUserState,
        puks: &[UserPrivateKey],
        team: &EntityId,
    ) -> Result<AuthenticatedTeamOutcome> {
        let subkey = derive_subkey_id(&credential.subkey_seed)?;
        if !user.devices().iter().any(|device| {
            device.id == *credential.parent.entity_id()
                && device.hepk == *credential.parent.hepk()
                && device.subkey.as_ref() == Some(&subkey)
        }) {
            return Err(Error::CredentialBinding(
                "Yubi credential is not enrolled in the supplied user state",
            ));
        }
        let puk_seed = &puks
            .last()
            .ok_or(Error::KeyBinding("no user private key is available"))?
            .seed;
        self.load_and_pin_team_with_material(
            host,
            &credential.uid,
            &credential.subkey_seed,
            &credential.certificate_chain,
            user,
            puk_seed,
            team,
        )
    }

    /// Creates a FOKS v0.1.9 single-owner ad-hoc team and then reconciles the
    /// predicted TeamID through the ordinary verified team-loading path.
    ///
    /// `secrets` must already be committed to the caller's encrypted
    /// credential store. They remain caller-owned if posting or reconciliation
    /// fails, so the operation can be resumed by loading the TeamID derived
    /// from the admin PTK.
    pub fn create_single_owner_adhoc_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        secrets: &AdHocTeamSecrets,
    ) -> Result<CreatedAdHocTeam> {
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        self.require_open_user_viewership(host, &credential.seed, &credential.certificate_chain)?;
        let owner = current_owner_puk(&authenticated_user)?;
        let device_id = derive_device_public(&credential.seed)?.id;
        let device = authenticated_user
            .verified
            .devices()
            .iter()
            .find(|device| device.id == device_id)
            .ok_or(Error::CredentialBinding(
                "team creator device is not enrolled",
            ))?;
        if device.role != Role::OWNER {
            return Err(Error::CredentialBinding(
                "team creator device is not an owner",
            ));
        }

        let ordered_seeds = secrets.ordered();
        let material = make_single_owner_adhoc_team(
            &AdHocTeamInput {
                user: &credential.uid,
                host: host.host_id(),
                root: &authenticated_user.verified.tree_root(),
                time: now_microseconds()?,
                owner_puk_generation: owner.generation,
                next_tree_location: random_bytes()?,
                subchain_tree_location: random_bytes()?,
                membership_next_tree_location: random_bytes()?,
            },
            &credential.seed,
            &owner.seed,
            ordered_seeds,
        )?;
        self.finish_single_owner_adhoc_team_creation(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            material,
            secrets,
        )
    }

    /// Hardware-backed variant of [`Self::create_single_owner_adhoc_team`].
    /// The Yubi parent signs the membership chain while the delegated software
    /// subkey is used only for mTLS.
    pub fn create_single_owner_adhoc_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        secrets: &AdHocTeamSecrets,
    ) -> Result<CreatedAdHocTeam> {
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        self.require_open_user_viewership(
            host,
            &credential.subkey_seed,
            &credential.certificate_chain,
        )?;
        let owner = current_owner_puk(&authenticated_user)?;
        let subkey = derive_subkey_id(&credential.subkey_seed)?;
        let device = authenticated_user
            .verified
            .devices()
            .iter()
            .find(|device| {
                device.id == *credential.parent.entity_id()
                    && device.hepk == *credential.parent.hepk()
                    && device.subkey.as_ref() == Some(&subkey)
            })
            .ok_or(Error::CredentialBinding(
                "Yubi team creator is not enrolled",
            ))?;
        if device.role != Role::OWNER {
            return Err(Error::CredentialBinding(
                "Yubi team creator is not an owner",
            ));
        }
        let ordered_seeds = secrets.ordered();
        let material = make_single_owner_adhoc_team_yubi(
            &AdHocTeamInput {
                user: &credential.uid,
                host: host.host_id(),
                root: &authenticated_user.verified.tree_root(),
                time: now_microseconds()?,
                owner_puk_generation: owner.generation,
                next_tree_location: random_bytes()?,
                subchain_tree_location: random_bytes()?,
                membership_next_tree_location: random_bytes()?,
            },
            credential.parent,
            &owner.seed,
            ordered_seeds,
        )?;
        self.finish_single_owner_adhoc_team_creation(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_user,
            owner,
            material,
            secrets,
        )
    }

    fn require_open_user_viewership(
        &self,
        host: &PinnedHost,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
    ) -> Result<()> {
        let host_config = HostConfig::decode(&self.call_with_material(
            host,
            &host.user,
            &encode_get_host_config_request()?,
            auth_seed,
            certificate_chain,
        )?)?;
        if host_config.user_viewership != ViewershipMode::Open {
            return Err(Error::TeamRequest(
                "host does not allow open user viewership",
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_single_owner_adhoc_team_creation(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        authenticated_user: &AuthenticatedUserOutcome,
        owner: &UserPrivateKey,
        material: AdHocTeamMaterial,
        secrets: &AdHocTeamSecrets,
    ) -> Result<CreatedAdHocTeam> {
        let ordered_seeds = secrets.ordered();
        let owner_public = foks_crypto::derive_shared_public(&owner.seed, ENTITY_PUK_VERIFY)?;
        let box_inputs = ordered_seeds
            .into_iter()
            .zip([
                Role::member(-0x4000),
                Role::member(0),
                Role::ADMIN,
                Role::OWNER,
            ])
            .map(|(seed, role)| SharedKeyBoxInput {
                seed,
                generation: 1,
                role,
                receiver_id: uid,
                receiver_host: None,
                receiver_hepk: &owner_public.hepk,
                receiver_role: Role::OWNER,
                receiver_generation: owner.generation,
            })
            .collect::<Vec<_>>();
        let randomness = (0..box_inputs.len())
            .map(|_| {
                Ok(PukBoxRandomness {
                    kem_message: random_bytes()?,
                    nonce: random_bytes()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let boxes = seal_shared_key_boxes(
            host.host_id(),
            &owner.seed,
            &owner_public.hepk,
            random_bytes()?,
            &box_inputs,
            &randomness,
        )?;
        let hepks = material
            .ptks
            .iter()
            .map(|ptk| ptk.hepk.clone())
            .collect::<Vec<_>>();
        let request = encode_create_adhoc_team_request(&AdHocTeamCreateArgument {
            link: &material.link,
            next_tree_location: material.next_tree_location,
            ptk_boxes: &boxes,
            hepks: &hepks,
            subchain_tree_location: material.subchain_tree_location,
            membership_link: &material.membership_link,
            membership_next_tree_location: material.membership_next_tree_location,
        })?;
        let operation_id = secrets.operation_id()?;
        let created_at = now_microseconds()?;
        let operation = AdHocTeamOperation {
            operation_id,
            host_id: host.host_id().as_bytes().to_vec(),
            uid: uid.as_bytes().to_vec(),
            device_id: device_id.as_bytes().to_vec(),
            team_id: material.team.as_bytes().to_vec(),
            request_hash: prefixed_hash(ADHOC_TEAM_REQUEST_HASH_TYPE_ID, &request),
            state: AdHocTeamOperationState::Prepared,
            created_at,
            updated_at: created_at,
        };
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        hard_store.record_adhoc_team_operation(&operation)?;
        let post = || {
            self.call_void_with_material(host, &host.user, &request, auth_seed, certificate_chain)
        };
        let post_error = match post() {
            Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
                code: STATUS_TX_RETRY_ERROR,
                ..
            })) => post().err(),
            Err(error) => Some(error),
            Ok(()) => None,
        };
        if post_error.is_none() {
            hard_store.advance_adhoc_team_operation(
                &operation_id,
                AdHocTeamOperationState::Submitted,
                now_microseconds()?,
            )?;
        }
        if matches!(
            post_error.as_ref(),
            Some(Error::Rpc(foks_rpc::Error::RemoteStatus { .. }))
        ) {
            return Err(post_error.expect("matched above"));
        }
        let authenticated = match self.wait_for_adhoc_team_with_material(
            host,
            uid,
            auth_seed,
            certificate_chain,
            authenticated_user,
            &owner.seed,
            &material.team,
            secrets,
        ) {
            Ok(authenticated) => authenticated,
            Err(_) if post_error.is_some() => return Err(post_error.expect("checked above")),
            Err(error) => return Err(error),
        };
        let persisted =
            hard_store
                .adhoc_team_operation(&operation_id)?
                .ok_or(Error::OperationBinding(
                    "ad-hoc team operation disappeared after submission",
                ))?;
        if persisted.state == AdHocTeamOperationState::Prepared {
            hard_store.advance_adhoc_team_operation(
                &operation_id,
                AdHocTeamOperationState::Submitted,
                now_microseconds()?,
            )?;
        }
        hard_store.advance_adhoc_team_operation(
            &operation_id,
            AdHocTeamOperationState::Verified,
            now_microseconds()?,
        )?;
        Ok(CreatedAdHocTeam {
            operation_id,
            team: material.team,
            authenticated,
        })
    }

    /// Reconciles a journaled ad-hoc creation without replaying the mutation.
    pub fn resume_single_owner_adhoc_team(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
        secrets: &AdHocTeamSecrets,
    ) -> Result<CreatedAdHocTeam> {
        let device_id = derive_device_public(&credential.seed)?.id;
        self.ensure_adhoc_team_operation_binding(host, &credential.uid, &device_id, secrets)?;
        let authenticated_user = self.authenticate_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        self.resume_single_owner_adhoc_team_with_material(
            host,
            &credential.uid,
            &device_id,
            &credential.seed,
            &credential.certificate_chain,
            &authenticated_user,
            &owner.seed,
            secrets,
        )
    }

    /// Hardware-backed variant of [`Self::resume_single_owner_adhoc_team`].
    /// It performs verified reconciliation only and never reposts the mutation.
    pub fn resume_single_owner_adhoc_team_yubi(
        &self,
        host: &PinnedHost,
        credential: &YubiCredential<'_>,
        secrets: &AdHocTeamSecrets,
    ) -> Result<CreatedAdHocTeam> {
        self.ensure_adhoc_team_operation_binding(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            secrets,
        )?;
        let authenticated_user = self.authenticate_yubi_and_pin(host, credential)?;
        let owner = current_owner_puk(&authenticated_user)?;
        self.resume_single_owner_adhoc_team_with_material(
            host,
            &credential.uid,
            credential.parent.entity_id(),
            &credential.subkey_seed,
            &credential.certificate_chain,
            &authenticated_user,
            &owner.seed,
            secrets,
        )
    }

    fn ensure_adhoc_team_operation_binding(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        secrets: &AdHocTeamSecrets,
    ) -> Result<()> {
        let operation_id = secrets.operation_id()?;
        let operation = HardStateStore::open(&host.database_path)?
            .adhoc_team_operation(&operation_id)?
            .ok_or(Error::AccountRequest(
                "ad-hoc team operation is not recorded",
            ))?;
        let team = secrets.team_id()?;
        if operation.host_id != host.host_id().as_bytes()
            || operation.uid != uid.as_bytes()
            || operation.device_id != device_id.as_bytes()
            || operation.team_id != team.as_bytes()
        {
            return Err(Error::OperationBinding(
                "ad-hoc team operation does not match supplied identities",
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn resume_single_owner_adhoc_team_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        device_id: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        authenticated_user: &AuthenticatedUserOutcome,
        owner_seed: &SecretSeed,
        secrets: &AdHocTeamSecrets,
    ) -> Result<CreatedAdHocTeam> {
        let operation_id = secrets.operation_id()?;
        let mut hard_store = HardStateStore::open(&host.database_path)?;
        let operation =
            hard_store
                .adhoc_team_operation(&operation_id)?
                .ok_or(Error::AccountRequest(
                    "ad-hoc team operation is not recorded",
                ))?;
        let team = secrets.team_id()?;
        if operation.host_id != host.host_id().as_bytes()
            || operation.uid != uid.as_bytes()
            || operation.device_id != device_id.as_bytes()
            || operation.team_id != team.as_bytes()
        {
            return Err(Error::OperationBinding(
                "ad-hoc team operation does not match supplied identities",
            ));
        }
        let authenticated = self.wait_for_adhoc_team_with_material(
            host,
            uid,
            auth_seed,
            certificate_chain,
            authenticated_user,
            owner_seed,
            &team,
            secrets,
        )?;
        if operation.state == AdHocTeamOperationState::Prepared {
            hard_store.advance_adhoc_team_operation(
                &operation_id,
                AdHocTeamOperationState::Submitted,
                now_microseconds()?,
            )?;
        }
        if operation.state != AdHocTeamOperationState::Verified {
            hard_store.advance_adhoc_team_operation(
                &operation_id,
                AdHocTeamOperationState::Verified,
                now_microseconds()?,
            )?;
        }
        Ok(CreatedAdHocTeam {
            operation_id,
            team,
            authenticated,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn wait_for_adhoc_team_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        user: &AuthenticatedUserOutcome,
        owner_seed: &SecretSeed,
        team: &EntityId,
        secrets: &AdHocTeamSecrets,
    ) -> Result<AuthenticatedTeamOutcome> {
        let roles_and_seeds = [
            (Role::member(-0x4000), &secrets.member_min),
            (Role::member(0), &secrets.member),
            (Role::ADMIN, &secrets.admin),
            (Role::OWNER, &secrets.owner),
        ];
        let mut last_error = None;
        for attempt in 0..40 {
            match self.load_and_pin_team_with_material(
                host,
                uid,
                auth_seed,
                certificate_chain,
                &user.verified,
                owner_seed,
                team,
            ) {
                Ok(authenticated)
                    if authenticated.ptks.len() == roles_and_seeds.len()
                        && roles_and_seeds.iter().all(|(role, seed)| {
                            authenticated.ptks.iter().any(|key| {
                                key.role == *role && key.generation == 1 && key.seed == **seed
                            })
                        }) =>
                {
                    return Ok(authenticated);
                }
                Ok(_) => {
                    last_error = Some(Error::TransitionNotObserved(
                        "ad-hoc team keys do not match prepared secrets",
                    ));
                }
                Err(error) => last_error = Some(error),
            }
            if attempt != 39 {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        Err(last_error.expect("team transition loop executes at least once"))
    }

    #[allow(clippy::too_many_arguments)]
    fn load_and_pin_team_with_material(
        &self,
        host: &PinnedHost,
        uid: &EntityId,
        auth_seed: &SecretSeed,
        certificate_chain: &[Vec<u8>],
        user: &VerifiedUserState,
        puk_seed: &SecretSeed,
        team: &EntityId,
    ) -> Result<AuthenticatedTeamOutcome> {
        if user.uid() != uid || user.host() != host.host_id() {
            return Err(Error::UserBinding(
                "user state does not match the credential and pinned host",
            ));
        }
        let source = user_key_for_seed(user, puk_seed)?;
        let view_request = TeamViewRequest {
            team: team.clone(),
            host: host.host_id.clone(),
            member: uid.clone(),
            member_host: host.host_id.clone(),
            source_role: source.role,
            generation: source.generation,
        };
        let challenge_bytes = self.call_with_material(
            host,
            &host.user,
            &encode_team_view_challenge_request(&view_request)?,
            auth_seed,
            certificate_chain,
        )?;
        let mut challenge = TeamViewChallenge::decode(&challenge_bytes)?;
        // Never sign server-selected authorization fields. FOKS v0.1.9's Go
        // client likewise overwrites the echoed request before signing.
        challenge.request = view_request;
        let signature =
            sign_shared_key_typed(puk_seed, TEAM_VIEW_CHALLENGE_TYPE_ID, &challenge.encoded()?)?;
        let activated_bytes = self.call_with_material(
            host,
            &host.user,
            &encode_activate_team_view_request(&challenge, &signature)?,
            auth_seed,
            certificate_chain,
        )?;
        let activated = ActivatedTeamView::decode(&activated_bytes)?;
        if activated.team != *team || activated.token != challenge.token {
            return Err(Error::TeamBinding(
                "activated view does not match its challenge",
            ));
        }

        let (merkle_acceptance, merkle) = self.advance_merkle_root(host)?;
        let prior = self.pinned_team(host, team)?;
        let (start, name) = match prior.as_ref() {
            Some(prior) => (
                prior
                    .chain_seqno()
                    .checked_add(1)
                    .ok_or(Error::TeamBinding("team chain sequence overflow"))?,
                Some((
                    prior.team_name(),
                    prior
                        .team_name_sequence()
                        .checked_add(1)
                        .ok_or(Error::TeamBinding("team name sequence overflow"))?,
                )),
            ),
            None => (1, None),
        };
        let chain_bytes = self.call_with_material(
            host,
            &host.user,
            &encode_load_team_chain_request_from(
                team,
                host.host_id(),
                &activated.token,
                start,
                name,
            )?,
            auth_seed,
            certificate_chain,
        )?;
        let verified = match prior.as_ref() {
            Some(prior) => verify_team_chain_increment(
                &chain_bytes,
                prior,
                team,
                host.host_id(),
                merkle.authenticated_roots(),
                &merkle.root().hostchain,
            )?,
            None => verify_team_chain(
                &chain_bytes,
                team,
                host.host_id(),
                merkle.authenticated_roots(),
                &merkle.root().hostchain,
            )?,
        };
        let member = verified
            .members()
            .iter()
            .find(|member| {
                member.party == *uid
                    && member.source_role == source.role
                    && member
                        .scoped_host
                        .as_ref()
                        .is_none_or(|scope| scope == host.host_id())
            })
            .ok_or(Error::TeamBinding(
                "requesting user is not in the verified team roster",
            ))?;
        if member.verify_key.as_bytes()[1..] != source.verify_key.as_bytes()[1..]
            || member.generation != source.generation
        {
            return Err(Error::TeamBinding(
                "team membership key does not match the requesting PUK",
            ));
        }
        let chain = TeamChain::decode(&chain_bytes)?;
        let receiver = SharedKeyDecapsulator::new(puk_seed, uid.clone())?;
        let expected_keys = verified
            .shared_keys()
            .iter()
            .filter(|key| key.role <= member.role)
            .collect::<Vec<_>>();
        if chain.boxes.len() != expected_keys.len() {
            return Err(Error::TeamBinding(
                "team key boxes do not match visible PTK roles",
            ));
        }
        let mut ptks = Vec::with_capacity(expected_keys.len());
        for key in expected_keys {
            let parcels = chain
                .boxes
                .iter()
                .filter(|parcel| parcel.role == key.role)
                .collect::<Vec<_>>();
            let [parcel] = parcels.as_slice() else {
                return Err(Error::TeamBinding(
                    "team PTK role has a missing or duplicate parcel",
                ));
            };
            let sender_hepk =
                team_parcel_sender_hepk(parcel, source, verified.members(), &chain.hepks)?;
            let clear = open_shared_key_parcel_with(
                parcel,
                &receiver,
                sender_hepk,
                &key.verify_key,
                host.host_id(),
                source.role,
                source.generation,
                key.role,
                ENTITY_PTK_VERIFY,
            )?;
            ptks.extend(
                open_shared_key_seed_chain(clear, parcel, uid, host.host_id())?
                    .into_iter()
                    .map(|key| TeamPrivateKey {
                        role: key.role,
                        generation: key.generation,
                        seed: key.into_seed(),
                    }),
            );
        }
        let mut store = HardStateStore::open(&host.database_path)?;
        let acceptance = store.accept_verified_team(&verified.hard_state_snapshot()?)?;
        Ok(AuthenticatedTeamOutcome {
            merkle_acceptance,
            acceptance,
            verified,
            ptks,
            view_token: activated.token,
        })
    }
}

fn team_parcel_sender_hepk<'a>(
    parcel: &foks_proto::PukParcel,
    receiver: &'a foks_verify::VerifiedSharedKey,
    members: &'a [foks_verify::VerifiedTeamMemberState],
    hepks: &'a [foks_proto::Hepk],
) -> Result<&'a foks_proto::Hepk> {
    if parcel.sender == receiver.verify_key {
        return Ok(&receiver.hepk);
    }
    let mut senders = members
        .iter()
        .filter(|member| member.verify_key == parcel.sender);
    let sender = senders.next().ok_or(Error::TeamBinding(
        "team PTK parcel sender is not in the verified roster",
    ))?;
    if senders.next().is_some() {
        return Err(Error::TeamBinding(
            "team PTK parcel sender is ambiguous in the verified roster",
        ));
    }
    let mut matched = None;
    for candidate in hepks {
        if hepk_fingerprint(candidate)? != sender.hepk_fingerprint {
            continue;
        }
        if matched.replace(candidate).is_some() {
            return Err(Error::TeamBinding(
                "team PTK parcel sender HEPK is ambiguous",
            ));
        }
    }
    matched.ok_or(Error::TeamBinding(
        "team PTK parcel sender HEPK is unavailable",
    ))
}

#[cfg(test)]
mod parcel_sender_tests {
    use foks_crypto::{derive_shared_public, hepk_fingerprint};
    use foks_proto::{EntityId, PukParcel, Role, SecretSeed, SharedKeyBoxSet, UserLink};
    use foks_snowpack::{decode, Value};
    use foks_verify::{VerifiedSharedKey, VerifiedTeamMemberState};

    use super::team_parcel_sender_hepk;

    const MUTATION_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user-mutations";
    const USER_DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user";

    fn fixture(directory: &str, name: &str) -> Vec<u8> {
        std::fs::read(format!("{directory}/{name}")).unwrap()
    }

    fn entity_fixture(name: &str) -> EntityId {
        match decode(&fixture(MUTATION_DIR, name)).unwrap() {
            Value::Binary(bytes) => EntityId::from_bytes(bytes).unwrap(),
            other => panic!("expected entity fixture, got {other:?}"),
        }
    }

    #[test]
    fn cross_user_ptk_sender_is_resolved_from_authenticated_roster() {
        let eldest = UserLink::decode(&fixture(MUTATION_DIR, "named-team-link.snowp")).unwrap();
        let owner_change = eldest.decode_team_group_change().unwrap().changes.remove(0);
        let actor_seed = SecretSeed::new(fixture(USER_DIR, "puk-seed.bin").try_into().unwrap());
        let actor = derive_shared_public(&actor_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
        let target_seed = SecretSeed::new(
            fixture(MUTATION_DIR, "add-member-target-puk-seed.bin")
                .try_into()
                .unwrap(),
        );
        let target = derive_shared_public(&target_seed, foks_proto::ENTITY_PUK_VERIFY).unwrap();
        let member = VerifiedTeamMemberState {
            party: owner_change.party,
            scoped_host: None,
            source_role: Role::OWNER,
            role: Role::OWNER,
            generation: owner_change.keys.as_ref().unwrap().generation,
            verify_key: actor.verify_key.clone(),
            hepk_fingerprint: hepk_fingerprint(&actor.hepk).unwrap(),
            removal_key_commitment: owner_change.keys.as_ref().unwrap().removal_key_commitment,
        };
        let receiver = VerifiedSharedKey {
            role: Role::OWNER,
            generation: 3,
            verify_key: target.verify_key,
            hepk: target.hepk,
        };
        let boxes =
            SharedKeyBoxSet::decode(&fixture(MUTATION_DIR, "add-member-ptk-boxes.snowp")).unwrap();
        let boxed = boxes.boxes.first().unwrap();
        let parcel = PukParcel {
            generation: boxed.generation,
            role: boxed.role,
            hybrid: boxed.hybrid.clone(),
            target: entity_fixture("add-member-target-uid.snowp"),
            target_host: None,
            target_role: boxed.target.role,
            target_generation: boxed.target.generation,
            sender: actor.verify_key,
            box_id: boxes.box_id,
            temp_dh_key: boxes.temp_dh_key,
            seed_chain: Vec::new(),
        };
        assert_eq!(
            team_parcel_sender_hepk(
                &parcel,
                &receiver,
                std::slice::from_ref(&member),
                std::slice::from_ref(&actor.hepk),
            )
            .unwrap(),
            &actor.hepk
        );
        assert!(
            team_parcel_sender_hepk(&parcel, &receiver, std::slice::from_ref(&member), &[],)
                .is_err()
        );
    }
}
