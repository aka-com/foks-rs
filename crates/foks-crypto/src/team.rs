//! Team creation, membership changes, removal proofs, and scoped token boxes.

use super::*;

/// Computes the exact v0.1.9 commitment authenticated by named-team member
/// links and removal-key boxes.
pub fn team_removal_key_commitment(removal_key: &SecretSeed) -> Result<[u8; 32]> {
    // Canonical Snowpack encodes a 32-byte blob as bin8(32). Construct it in
    // zeroizing fixed storage so commitment calculation makes no ordinary
    // heap copy of the removal key.
    let mut encoded = Zeroizing::new([0_u8; 34]);
    encoded[0] = 0xc4;
    encoded[1] = 32;
    encoded[2..].copy_from_slice(removal_key.as_bytes());
    prefixed_hash_signable(TEAM_REMOVAL_KEY_TYPE_ID, encoded.as_slice())
}

pub struct AdHocTeamInput<'a> {
    pub user: &'a EntityId,
    pub host: &'a EntityId,
    pub root: &'a TreeRoot,
    pub time: u64,
    pub owner_puk_generation: u64,
    pub membership_sequence: u64,
    pub membership_previous: Option<[u8; 32]>,
    pub next_tree_location: [u8; 32],
    pub subchain_tree_location: [u8; 32],
    pub membership_next_tree_location: [u8; 32],
}

pub struct AdHocTeamMaterial {
    pub team: EntityId,
    pub link: UserLink,
    pub membership_link: UserLink,
    pub ptks: Vec<SharedPublicMaterial>,
    pub next_tree_location: [u8; 32],
    pub subchain_tree_location: [u8; 32],
    pub membership_next_tree_location: [u8; 32],
}

pub struct NamedTeamInput<'a> {
    pub user: &'a EntityId,
    pub host: &'a EntityId,
    pub root: &'a TreeRoot,
    pub time: u64,
    pub owner_puk_generation: u64,
    pub membership_sequence: u64,
    pub membership_previous: Option<[u8; 32]>,
    pub normalized_name: &'a [u8],
    pub name_sequence: u64,
    pub team_name_commitment_key: [u8; 16],
    pub next_tree_location: [u8; 32],
    pub subchain_tree_location: [u8; 32],
    pub membership_next_tree_location: [u8; 32],
}

pub struct NamedTeamMaterial {
    pub team: EntityId,
    pub link: UserLink,
    pub membership_link: UserLink,
    pub ptks: Vec<SharedPublicMaterial>,
    pub removal_key_commitment: [u8; 32],
    pub next_tree_location: [u8; 32],
    pub subchain_tree_location: [u8; 32],
    pub membership_next_tree_location: [u8; 32],
}

pub struct AddLocalTeamMemberInput<'a> {
    pub actor: &'a EntityId,
    pub actor_source_role: Role,
    pub team: &'a EntityId,
    pub host: &'a EntityId,
    pub sequence: u64,
    pub previous: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub member: &'a EntityId,
    pub member_source_role: Role,
    pub member_destination_role: Role,
    pub member_generation: u64,
    pub member_public: &'a SharedPublicMaterial,
}

pub struct AddRemoteTeamMemberInput<'a> {
    pub actor: &'a EntityId,
    pub actor_source_role: Role,
    pub team: &'a EntityId,
    pub host: &'a EntityId,
    pub sequence: u64,
    pub previous: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub member: &'a EntityId,
    pub member_host: &'a EntityId,
    pub member_source_role: Role,
    pub member_destination_role: Role,
    pub member_generation: u64,
    pub member_public: &'a SharedPublicMaterial,
    pub member_index_range: Option<&'a foks_proto::RationalRange>,
}

pub struct AddLocalTeamMemberMaterial {
    pub link: UserLink,
    pub removal_key_commitment: [u8; 32],
    pub next_tree_location: [u8; 32],
}

pub struct TeamMetadataInput<'a> {
    pub actor: &'a EntityId,
    pub actor_source_role: Role,
    pub team: &'a EntityId,
    pub host: &'a EntityId,
    pub sequence: u64,
    pub previous: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub index_range: &'a foks_proto::RationalRange,
}

pub struct TeamMetadataMaterial {
    pub link: UserLink,
    pub next_tree_location: [u8; 32],
}

pub struct TeamPtkRotation<'a> {
    pub role: Role,
    pub generation: u64,
    pub seed: &'a SecretSeed,
}

pub struct RemoveLocalTeamMemberInput<'a> {
    pub actor: &'a EntityId,
    pub actor_source_role: Role,
    pub team: &'a EntityId,
    pub host: &'a EntityId,
    pub sequence: u64,
    pub previous: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub member: &'a EntityId,
    pub member_source_role: Role,
}

pub struct ChangeTeamMemberInput<'a> {
    pub actor: &'a EntityId,
    pub actor_source_role: Role,
    pub team: &'a EntityId,
    pub host: &'a EntityId,
    pub sequence: u64,
    pub previous: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub member: &'a EntityId,
    pub member_host: Option<&'a EntityId>,
    pub member_source_role: Role,
    pub destination_role: Role,
    pub member_generation: Option<u64>,
    pub member_public: Option<&'a SharedPublicMaterial>,
    pub member_index_range: Option<&'a foks_proto::RationalRange>,
}

pub struct ChangeTeamMemberEntryInput<'a> {
    pub member: &'a EntityId,
    pub member_host: Option<&'a EntityId>,
    pub member_source_role: Role,
    pub destination_role: Role,
    pub member_generation: Option<u64>,
    pub member_public: Option<&'a SharedPublicMaterial>,
    pub member_index_range: Option<&'a foks_proto::RationalRange>,
}

pub struct ChangeTeamMembersInput<'a> {
    pub actor: &'a EntityId,
    pub actor_source_role: Role,
    pub team: &'a EntityId,
    pub host: &'a EntityId,
    pub sequence: u64,
    pub previous: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub members: &'a [ChangeTeamMemberEntryInput<'a>],
}

pub struct RemoveLocalTeamMemberMaterial {
    pub link: UserLink,
    pub ptks: Vec<SharedPublicMaterial>,
    pub next_tree_location: [u8; 32],
}

/// Derives the permanent ad-hoc TeamID selected by FOKS from its admin PTK.
pub fn adhoc_team_id_from_admin_seed(seed: &SecretSeed) -> Result<EntityId> {
    let admin = derive_shared_public(seed, foks_proto::ENTITY_PTK_VERIFY)?;
    let mut team_bytes = admin.verify_key.as_bytes().to_vec();
    team_bytes[0] = foks_proto::ENTITY_AD_HOC_TEAM;
    Ok(EntityId::from_bytes(team_bytes)?)
}

/// Derives the permanent named TeamID selected by FOKS from its admin PTK.
pub fn named_team_id_from_admin_seed(seed: &SecretSeed) -> Result<EntityId> {
    let admin = derive_shared_public(seed, foks_proto::ENTITY_PTK_VERIFY)?;
    let mut team_bytes = admin.verify_key.as_bytes().to_vec();
    team_bytes[0] = foks_proto::ENTITY_NAMED_TEAM;
    Ok(EntityId::from_bytes(team_bytes)?)
}

/// Constructs the exact single-owner ad-hoc team and owner membership links.
/// PTK seeds are ordered member-min, member, admin, owner.
pub fn make_single_owner_adhoc_team(
    input: &AdHocTeamInput<'_>,
    device_seed: &SecretSeed,
    owner_puk_seed: &SecretSeed,
    ptk_seeds: [&SecretSeed; 4],
) -> Result<AdHocTeamMaterial> {
    let device = derive_device_public(device_seed)?;
    make_single_owner_adhoc_team_with_signer(
        input,
        &device.id,
        owner_puk_seed,
        ptk_seeds,
        |canonical_object| sign_seed_typed(device_seed, LINK_OUTER_V1_TYPE_ID, canonical_object),
    )
}

/// Hardware-backed variant of [`make_single_owner_adhoc_team`]. The Yubi
/// parent signs the creator's membership link; its delegated Ed25519 subkey is
/// intentionally not involved in chain signing.
pub fn make_single_owner_adhoc_team_yubi(
    input: &AdHocTeamInput<'_>,
    device: &dyn YubiDevice,
    owner_puk_seed: &SecretSeed,
    ptk_seeds: [&SecretSeed; 4],
) -> Result<AdHocTeamMaterial> {
    if device.entity_id().entity_type() != foks_proto::ENTITY_YUBI {
        return Err(Error::SignatureType);
    }
    make_single_owner_adhoc_team_with_signer(
        input,
        device.entity_id(),
        owner_puk_seed,
        ptk_seeds,
        |canonical_object| sign_yubi_typed(device, LINK_OUTER_V1_TYPE_ID, canonical_object),
    )
}

pub(super) fn make_single_owner_adhoc_team_with_signer(
    input: &AdHocTeamInput<'_>,
    device_id: &EntityId,
    owner_puk_seed: &SecretSeed,
    ptk_seeds: [&SecretSeed; 4],
    sign_membership: impl FnOnce(&[u8]) -> Result<Signature>,
) -> Result<AdHocTeamMaterial> {
    if input.owner_puk_generation == 0
        || input.membership_sequence == 0
        || (input.membership_sequence == 1) != input.membership_previous.is_none()
        || input.next_tree_location == input.subchain_tree_location
        || input.next_tree_location == input.membership_next_tree_location
        || input.subchain_tree_location == input.membership_next_tree_location
    {
        return Err(Error::AdHocTeamMaterial);
    }
    let owner_puk = derive_shared_public(owner_puk_seed, foks_proto::ENTITY_PUK_VERIFY)?;
    let mut signer_bytes = owner_puk.verify_key.as_bytes().to_vec();
    signer_bytes[0] = foks_proto::ENTITY_USER;
    let signer = EntityId::from_bytes(signer_bytes)?;
    let roles = [
        Role::member(-0x4000),
        Role::member(0),
        Role::ADMIN,
        Role::OWNER,
    ];
    let ptks = ptk_seeds
        .iter()
        .map(|seed| derive_shared_public(seed, foks_proto::ENTITY_PTK_VERIFY))
        .collect::<Result<Vec<_>>>()?;
    if ptks.iter().enumerate().any(|(index, ptk)| {
        ptks[index + 1..]
            .iter()
            .any(|other| other.verify_key == ptk.verify_key)
    }) {
        return Err(Error::AdHocTeamMaterial);
    }
    let team = adhoc_team_id_from_admin_seed(ptk_seeds[2])?;
    let shared_keys = roles
        .iter()
        .zip(&ptks)
        .map(|(role, ptk)| {
            Ok(UserSharedKey {
                generation: 1,
                role: *role,
                verify_key: ptk.verify_key.clone(),
                hepk_fingerprint: hepk_fingerprint(&ptk.hepk)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let change = TeamGroupChange {
        seqno: 1,
        previous: None,
        root: input.root.clone(),
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        team: team.clone(),
        host: input.host.clone(),
        signer,
        signer_owner: TeamKeyOwner {
            party: input.user.clone(),
            source_role: Role::OWNER,
        },
        changes: vec![TeamMemberChange {
            role: Role::OWNER,
            party: input.user.clone(),
            scoped_host: None,
            source_role: Role::OWNER,
            keys: Some(TeamMemberKeys {
                verify_key: owner_puk.verify_key.clone(),
                hepk_fingerprint: hepk_fingerprint(&owner_puk.hepk)?,
                generation: input.owner_puk_generation,
                removal_key_commitment: None,
                index_range: None,
            }),
        }],
        shared_keys,
        metadata: vec![
            ChangeMetadata::Eldest {
                subchain_location_commitment: tree_location_commitment(
                    &input.subchain_tree_location,
                )?,
            },
            ChangeMetadata::TeamIndexRange(foks_proto::RationalRange {
                low: foks_proto::Rational {
                    infinity: false,
                    base: vec![1],
                    exponent: 0,
                },
                high: foks_proto::Rational {
                    infinity: true,
                    base: Vec::new(),
                    exponent: 0,
                },
            }),
            ChangeMetadata::MemberLoadFloor(Role::member(0)),
        ],
    };
    let unsigned = UnsignedUserLink::team_group_change(&change)?;
    let mut signatures = Vec::with_capacity(5);
    for seed in ptk_seeds {
        signatures.push(sign_seed_typed(
            seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_seed_typed(
        owner_puk_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    let link = unsigned.finish(signatures)?;

    let membership_unsigned =
        UnsignedUserLink::approved_adhoc_membership(&AdHocMembershipLinkPublic {
            user: input.user,
            host: input.host,
            signer: device_id,
            sequence: input.membership_sequence,
            previous: input.membership_previous,
            root: input.root,
            time: 0,
            next_location_commitment: tree_location_commitment(
                &input.membership_next_tree_location,
            )?,
            team: &team,
            source_role: Role::OWNER,
            destination_role: Role::OWNER,
            team_sequence: 1,
        })?;
    let membership_signature = sign_membership(&membership_unsigned.signing_bytes(&[])?)?;
    let membership_link = membership_unsigned.finish(vec![membership_signature])?;
    Ok(AdHocTeamMaterial {
        team,
        link,
        membership_link,
        ptks,
        next_tree_location: input.next_tree_location,
        subchain_tree_location: input.subchain_tree_location,
        membership_next_tree_location: input.membership_next_tree_location,
    })
}

/// Constructs a named team's eldest link and its creator's approved
/// membership link. PTK seeds are ordered member-min, member, admin, owner.
pub fn make_single_owner_named_team(
    input: &NamedTeamInput<'_>,
    device_seed: &SecretSeed,
    owner_puk_seed: &SecretSeed,
    ptk_seeds: [&SecretSeed; 4],
    removal_key: &SecretSeed,
) -> Result<NamedTeamMaterial> {
    let device = derive_device_public(device_seed)?;
    make_single_owner_named_team_with_signer(
        input,
        &device.id,
        owner_puk_seed,
        ptk_seeds,
        removal_key,
        |canonical_object| sign_seed_typed(device_seed, LINK_OUTER_V1_TYPE_ID, canonical_object),
    )
}

/// Hardware-backed variant of [`make_single_owner_named_team`].
pub fn make_single_owner_named_team_yubi(
    input: &NamedTeamInput<'_>,
    device: &dyn YubiDevice,
    owner_puk_seed: &SecretSeed,
    ptk_seeds: [&SecretSeed; 4],
    removal_key: &SecretSeed,
) -> Result<NamedTeamMaterial> {
    if device.entity_id().entity_type() != foks_proto::ENTITY_YUBI {
        return Err(Error::SignatureType);
    }
    make_single_owner_named_team_with_signer(
        input,
        device.entity_id(),
        owner_puk_seed,
        ptk_seeds,
        removal_key,
        |canonical_object| sign_yubi_typed(device, LINK_OUTER_V1_TYPE_ID, canonical_object),
    )
}

pub(super) fn make_single_owner_named_team_with_signer(
    input: &NamedTeamInput<'_>,
    device_id: &EntityId,
    owner_puk_seed: &SecretSeed,
    ptk_seeds: [&SecretSeed; 4],
    removal_key: &SecretSeed,
    sign_membership: impl FnOnce(&[u8]) -> Result<Signature>,
) -> Result<NamedTeamMaterial> {
    if input.owner_puk_generation == 0
        || input.membership_sequence == 0
        || (input.membership_sequence == 1) != input.membership_previous.is_none()
        || input.normalized_name.is_empty()
        || input.name_sequence == 0
        || input.next_tree_location == input.subchain_tree_location
        || input.next_tree_location == input.membership_next_tree_location
        || input.subchain_tree_location == input.membership_next_tree_location
    {
        return Err(Error::NamedTeamMaterial);
    }
    let owner_puk = derive_shared_public(owner_puk_seed, foks_proto::ENTITY_PUK_VERIFY)?;
    let mut signer_bytes = owner_puk.verify_key.as_bytes().to_vec();
    signer_bytes[0] = foks_proto::ENTITY_USER;
    let signer = EntityId::from_bytes(signer_bytes)?;
    let roles = [
        Role::member(-0x4000),
        Role::member(0),
        Role::ADMIN,
        Role::OWNER,
    ];
    let ptks = ptk_seeds
        .iter()
        .map(|seed| derive_shared_public(seed, foks_proto::ENTITY_PTK_VERIFY))
        .collect::<Result<Vec<_>>>()?;
    if ptks.iter().enumerate().any(|(index, ptk)| {
        ptks[index + 1..]
            .iter()
            .any(|other| other.verify_key == ptk.verify_key)
    }) {
        return Err(Error::NamedTeamMaterial);
    }
    let team = named_team_id_from_admin_seed(ptk_seeds[2])?;
    let removal_key_commitment = team_removal_key_commitment(removal_key)?;
    let name_commitment = commitment(
        NAME_COMMITMENT_TYPE_ID,
        &encode(&Value::Array(vec![
            Value::Text(input.normalized_name.to_vec()),
            Value::Unsigned(input.name_sequence),
        ]))?,
        &input.team_name_commitment_key,
    );
    let shared_keys = roles
        .iter()
        .zip(&ptks)
        .map(|(role, ptk)| {
            Ok(UserSharedKey {
                generation: 1,
                role: *role,
                verify_key: ptk.verify_key.clone(),
                hepk_fingerprint: hepk_fingerprint(&ptk.hepk)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let change = TeamGroupChange {
        seqno: 1,
        previous: None,
        root: input.root.clone(),
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        team: team.clone(),
        host: input.host.clone(),
        signer,
        signer_owner: TeamKeyOwner {
            party: input.user.clone(),
            source_role: Role::OWNER,
        },
        changes: vec![TeamMemberChange {
            role: Role::OWNER,
            party: input.user.clone(),
            scoped_host: None,
            source_role: Role::OWNER,
            keys: Some(TeamMemberKeys {
                verify_key: owner_puk.verify_key.clone(),
                hepk_fingerprint: hepk_fingerprint(&owner_puk.hepk)?,
                generation: input.owner_puk_generation,
                removal_key_commitment: Some(removal_key_commitment),
                index_range: None,
            }),
        }],
        shared_keys,
        metadata: vec![
            ChangeMetadata::TeamName(name_commitment),
            ChangeMetadata::Eldest {
                subchain_location_commitment: tree_location_commitment(
                    &input.subchain_tree_location,
                )?,
            },
            ChangeMetadata::TeamIndexRange(foks_proto::RationalRange {
                low: foks_proto::Rational {
                    infinity: false,
                    base: vec![1],
                    exponent: 0,
                },
                high: foks_proto::Rational {
                    infinity: true,
                    base: Vec::new(),
                    exponent: 0,
                },
            }),
            ChangeMetadata::MemberLoadFloor(Role::member(0)),
        ],
    };
    let unsigned = UnsignedUserLink::team_group_change(&change)?;
    let mut signatures = Vec::with_capacity(5);
    for seed in ptk_seeds {
        signatures.push(sign_seed_typed(
            seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_seed_typed(
        owner_puk_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    let link = unsigned.finish(signatures)?;
    let membership_unsigned =
        UnsignedUserLink::approved_membership(&ApprovedMembershipLinkPublic {
            user: input.user,
            host: input.host,
            signer: device_id,
            sequence: input.membership_sequence,
            previous: input.membership_previous,
            root: input.root,
            time: 0,
            next_location_commitment: tree_location_commitment(
                &input.membership_next_tree_location,
            )?,
            team: &team,
            source_role: Role::OWNER,
            destination_role: Role::OWNER,
            team_sequence: 1,
            removal_key_commitment,
        })?;
    let membership_signature = sign_membership(&membership_unsigned.signing_bytes(&[])?)?;
    let membership_link = membership_unsigned.finish(vec![membership_signature])?;
    Ok(NamedTeamMaterial {
        team,
        link,
        membership_link,
        ptks,
        removal_key_commitment,
        next_tree_location: input.next_tree_location,
        subchain_tree_location: input.subchain_tree_location,
        membership_next_tree_location: input.membership_next_tree_location,
    })
}

/// Constructs one additive local-user named-team transition. Pure additions
/// reuse existing PTK generations; the caller separately boxes those keys and
/// the new member's removal key.
pub fn make_add_local_team_member_link(
    input: &AddLocalTeamMemberInput<'_>,
    actor_puk_seed: &SecretSeed,
    removal_key: &SecretSeed,
) -> Result<AddLocalTeamMemberMaterial> {
    input.actor.clone().require_type(foks_proto::ENTITY_USER)?;
    input.member.clone().require_type(foks_proto::ENTITY_USER)?;
    input
        .team
        .clone()
        .require_type(foks_proto::ENTITY_NAMED_TEAM)?;
    input.host.clone().require_type(foks_proto::ENTITY_HOST)?;
    if input.sequence < 2
        || input.actor_source_role == Role::NONE
        || input.member_source_role == Role::NONE
        || input.member_destination_role == Role::NONE
        || input.member_generation == 0
        || input.actor == input.member
    {
        return Err(Error::NamedTeamMaterial);
    }
    let actor_public = derive_shared_public(actor_puk_seed, foks_proto::ENTITY_PUK_VERIFY)?;
    let signer = actor_public.verify_key;
    let removal_key_commitment = team_removal_key_commitment(removal_key)?;
    let change = TeamGroupChange {
        seqno: input.sequence,
        previous: Some(input.previous),
        root: input.root.clone(),
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        team: input.team.clone(),
        host: input.host.clone(),
        signer,
        signer_owner: TeamKeyOwner {
            party: input.actor.clone(),
            source_role: input.actor_source_role,
        },
        changes: vec![TeamMemberChange {
            role: input.member_destination_role,
            party: input.member.clone(),
            scoped_host: None,
            source_role: input.member_source_role,
            keys: Some(TeamMemberKeys {
                verify_key: input.member_public.verify_key.clone(),
                hepk_fingerprint: hepk_fingerprint(&input.member_public.hepk)?,
                generation: input.member_generation,
                removal_key_commitment: Some(removal_key_commitment),
                index_range: None,
            }),
        }],
        shared_keys: Vec::new(),
        metadata: Vec::new(),
    };
    let unsigned = UnsignedUserLink::team_group_change(&change)?;
    let signature = sign_seed_typed(
        actor_puk_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&[])?,
    )?;
    Ok(AddLocalTeamMemberMaterial {
        link: unsigned.finish(vec![signature])?,
        removal_key_commitment,
        next_tree_location: input.next_tree_location,
    })
}

/// Constructs the signed roster transition for a remote user or team. The
/// remote host scope is part of the signed link and therefore cannot be
/// rewritten by the local server.
pub fn make_add_remote_team_member_link(
    input: &AddRemoteTeamMemberInput<'_>,
    actor_puk_seed: &SecretSeed,
    removal_key: &SecretSeed,
) -> Result<AddLocalTeamMemberMaterial> {
    if input.member_host == input.host {
        return Err(Error::NamedTeamMaterial);
    }
    make_add_scoped_team_member_link(input, actor_puk_seed, removal_key)
}

/// Shared local/remote party addition; local scope is encoded as nil.
pub fn make_add_scoped_team_member_link(
    input: &AddRemoteTeamMemberInput<'_>,
    actor_puk_seed: &SecretSeed,
    removal_key: &SecretSeed,
) -> Result<AddLocalTeamMemberMaterial> {
    input.actor.clone().require_type(foks_proto::ENTITY_USER)?;
    if !matches!(
        input.member.entity_type(),
        foks_proto::ENTITY_USER | foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
    ) {
        return Err(Error::NamedTeamMaterial);
    }
    input
        .team
        .clone()
        .require_type(foks_proto::ENTITY_NAMED_TEAM)?;
    input.host.clone().require_type(foks_proto::ENTITY_HOST)?;
    input
        .member_host
        .clone()
        .require_type(foks_proto::ENTITY_HOST)?;
    if input.sequence < 2
        || input.actor_source_role == Role::NONE
        || input.member_source_role == Role::NONE
        || input.member_destination_role == Role::NONE
        || input.member_generation == 0
        || input.actor == input.member
        || matches!(
            input.member.entity_type(),
            foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
        ) != input.member_index_range.is_some()
    {
        return Err(Error::NamedTeamMaterial);
    }
    let actor_public = derive_shared_public(actor_puk_seed, foks_proto::ENTITY_PUK_VERIFY)?;
    let removal_key_commitment = team_removal_key_commitment(removal_key)?;
    let change = TeamGroupChange {
        seqno: input.sequence,
        previous: Some(input.previous),
        root: input.root.clone(),
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        team: input.team.clone(),
        host: input.host.clone(),
        signer: actor_public.verify_key,
        signer_owner: TeamKeyOwner {
            party: input.actor.clone(),
            source_role: input.actor_source_role,
        },
        changes: vec![TeamMemberChange {
            role: input.member_destination_role,
            party: input.member.clone(),
            scoped_host: (input.member_host != input.host).then(|| input.member_host.clone()),
            source_role: input.member_source_role,
            keys: Some(TeamMemberKeys {
                verify_key: input.member_public.verify_key.clone(),
                hepk_fingerprint: hepk_fingerprint(&input.member_public.hepk)?,
                generation: input.member_generation,
                removal_key_commitment: Some(removal_key_commitment),
                index_range: input.member_index_range.cloned(),
            }),
        }],
        shared_keys: Vec::new(),
        metadata: Vec::new(),
    };
    let unsigned = UnsignedUserLink::team_group_change(&change)?;
    let signature = sign_seed_typed(
        actor_puk_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&[])?,
    )?;
    Ok(AddLocalTeamMemberMaterial {
        link: unsigned.finish(vec![signature])?,
        removal_key_commitment,
        next_tree_location: input.next_tree_location,
    })
}

/// Constructs one signed, metadata-only named-team transition. The caller must
/// derive `index_range` from the authenticated current team state; chain replay
/// enforces that the new range is a strict narrowing.
pub fn make_team_index_range_link(
    input: &TeamMetadataInput<'_>,
    actor_puk_seed: &SecretSeed,
) -> Result<TeamMetadataMaterial> {
    input.actor.clone().require_type(foks_proto::ENTITY_USER)?;
    input
        .team
        .clone()
        .require_type(foks_proto::ENTITY_NAMED_TEAM)?;
    input.host.clone().require_type(foks_proto::ENTITY_HOST)?;
    if input.sequence < 2 || input.actor_source_role == Role::NONE {
        return Err(Error::NamedTeamMaterial);
    }
    let actor_public = derive_shared_public(actor_puk_seed, foks_proto::ENTITY_PUK_VERIFY)?;
    let change = TeamGroupChange {
        seqno: input.sequence,
        previous: Some(input.previous),
        root: input.root.clone(),
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        team: input.team.clone(),
        host: input.host.clone(),
        signer: actor_public.verify_key,
        signer_owner: TeamKeyOwner {
            party: input.actor.clone(),
            source_role: input.actor_source_role,
        },
        changes: Vec::new(),
        shared_keys: Vec::new(),
        metadata: vec![ChangeMetadata::TeamIndexRange(input.index_range.clone())],
    };
    let unsigned = UnsignedUserLink::team_group_change(&change)?;
    let signature = sign_seed_typed(
        actor_puk_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&[])?,
    )?;
    Ok(TeamMetadataMaterial {
        link: unsigned.finish(vec![signature])?,
        next_tree_location: input.next_tree_location,
    })
}

/// Constructs and stacked-signs one local-user removal and its exact PTK
/// generation advances. The caller derives the required role set from the
/// authenticated pre-transition roster and key schedule.
pub fn make_remove_local_team_member_link(
    input: &RemoveLocalTeamMemberInput<'_>,
    actor_puk_seed: &SecretSeed,
    rotations: &[TeamPtkRotation<'_>],
) -> Result<RemoveLocalTeamMemberMaterial> {
    make_change_team_member_link(
        &ChangeTeamMemberInput {
            actor: input.actor,
            actor_source_role: input.actor_source_role,
            team: input.team,
            host: input.host,
            sequence: input.sequence,
            previous: input.previous,
            root: input.root,
            time: input.time,
            next_tree_location: input.next_tree_location,
            member: input.member,
            member_host: None,
            member_source_role: input.member_source_role,
            destination_role: Role::NONE,
            member_generation: None,
            member_public: None,
            member_index_range: None,
        },
        actor_puk_seed,
        rotations,
    )
}

/// Constructs and stacked-signs a removal, role change, or member credential
/// generation advance together with its exact PTK introductions or rotations.
pub fn make_change_team_member_link(
    input: &ChangeTeamMemberInput<'_>,
    actor_puk_seed: &SecretSeed,
    rotations: &[TeamPtkRotation<'_>],
) -> Result<RemoveLocalTeamMemberMaterial> {
    make_change_team_members_link(
        &ChangeTeamMembersInput {
            actor: input.actor,
            actor_source_role: input.actor_source_role,
            team: input.team,
            host: input.host,
            sequence: input.sequence,
            previous: input.previous,
            root: input.root,
            time: input.time,
            next_tree_location: input.next_tree_location,
            members: &[ChangeTeamMemberEntryInput {
                member: input.member,
                member_host: input.member_host,
                member_source_role: input.member_source_role,
                destination_role: input.destination_role,
                member_generation: input.member_generation,
                member_public: input.member_public,
                member_index_range: input.member_index_range,
            }],
        },
        actor_puk_seed,
        rotations,
    )
}

/// Constructs and stacked-signs one atomic group change covering multiple
/// roster transitions and their union PTK rotation schedule.
pub fn make_change_team_members_link(
    input: &ChangeTeamMembersInput<'_>,
    actor_shared_key_seed: &SecretSeed,
    rotations: &[TeamPtkRotation<'_>],
) -> Result<RemoveLocalTeamMemberMaterial> {
    let actor_key_type = match input.actor.entity_type() {
        foks_proto::ENTITY_USER => foks_proto::ENTITY_PUK_VERIFY,
        foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM => {
            foks_proto::ENTITY_PTK_VERIFY
        }
        _ => return Err(Error::NamedTeamMaterial),
    };
    input
        .team
        .clone()
        .require_type(foks_proto::ENTITY_NAMED_TEAM)?;
    input.host.clone().require_type(foks_proto::ENTITY_HOST)?;
    if input.sequence < 2 || input.actor_source_role == Role::NONE || input.members.is_empty() {
        return Err(Error::NamedTeamMaterial);
    }
    let mut member_bindings = std::collections::BTreeSet::new();
    for member in input.members {
        if !matches!(
            member.member.entity_type(),
            foks_proto::ENTITY_USER
                | foks_proto::ENTITY_NAMED_TEAM
                | foks_proto::ENTITY_AD_HOC_TEAM
        ) || member.member_source_role == Role::NONE
            || (member.destination_role == Role::NONE)
                != (member.member_generation.is_none()
                    && member.member_public.is_none()
                    && member.member_index_range.is_none())
            || (member.destination_role != Role::NONE
                && matches!(
                    member.member.entity_type(),
                    foks_proto::ENTITY_NAMED_TEAM | foks_proto::ENTITY_AD_HOC_TEAM
                ) != member.member_index_range.is_some())
            || member
                .member_host
                .is_some_and(|host| host.entity_type() != foks_proto::ENTITY_HOST)
            || !member_bindings.insert((
                member.member.as_bytes().to_vec(),
                member.member_host.map(|host| host.as_bytes().to_vec()),
                member.member_source_role,
            ))
        {
            return Err(Error::NamedTeamMaterial);
        }
    }
    let actor = derive_shared_public(actor_shared_key_seed, actor_key_type)?;
    let mut prior_role = None;
    let mut verify_keys = std::collections::BTreeSet::new();
    let mut ptks = Vec::with_capacity(rotations.len());
    let mut shared_keys = Vec::with_capacity(rotations.len());
    for rotation in rotations {
        if rotation.role == Role::NONE
            || rotation.generation == 0
            || prior_role.is_some_and(|role| role >= rotation.role)
        {
            return Err(Error::NamedTeamMaterial);
        }
        let public = derive_shared_public(rotation.seed, foks_proto::ENTITY_PTK_VERIFY)?;
        if !verify_keys.insert(public.verify_key.as_bytes().to_vec()) {
            return Err(Error::NamedTeamMaterial);
        }
        shared_keys.push(UserSharedKey {
            generation: rotation.generation,
            role: rotation.role,
            verify_key: public.verify_key.clone(),
            hepk_fingerprint: hepk_fingerprint(&public.hepk)?,
        });
        ptks.push(public);
        prior_role = Some(rotation.role);
    }
    let change = TeamGroupChange {
        seqno: input.sequence,
        previous: Some(input.previous),
        root: input.root.clone(),
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        team: input.team.clone(),
        host: input.host.clone(),
        signer: actor.verify_key,
        signer_owner: TeamKeyOwner {
            party: input.actor.clone(),
            source_role: input.actor_source_role,
        },
        changes: input
            .members
            .iter()
            .map(|member| {
                Ok(TeamMemberChange {
                    role: member.destination_role,
                    party: member.member.clone(),
                    scoped_host: member.member_host.cloned(),
                    source_role: member.member_source_role,
                    keys: match (member.member_generation, member.member_public) {
                        (Some(generation), Some(public)) if generation > 0 => {
                            Some(TeamMemberKeys {
                                verify_key: public.verify_key.clone(),
                                hepk_fingerprint: hepk_fingerprint(&public.hepk)?,
                                generation,
                                removal_key_commitment: None,
                                index_range: member.member_index_range.cloned(),
                            })
                        }
                        (None, None) => None,
                        _ => return Err(Error::NamedTeamMaterial),
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?,
        shared_keys,
        metadata: Vec::new(),
    };
    let unsigned = UnsignedUserLink::team_group_change(&change)?;
    let mut signatures = Vec::with_capacity(rotations.len() + 1);
    for rotation in rotations {
        signatures.push(sign_seed_typed(
            rotation.seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_seed_typed(
        actor_shared_key_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    Ok(RemoveLocalTeamMemberMaterial {
        link: unsigned.finish(signatures)?,
        ptks,
        next_tree_location: input.next_tree_location,
    })
}

/// Authenticates a removal against the member's committed removal key and
/// the exact Merkle root used by the edit.
pub fn make_team_removal_proof(
    removal_key: &SecretSeed,
    payload: TeamRemovalMacPayload,
) -> Result<TeamRemovalAndCommitment> {
    let encoded = Zeroizing::new(payload.encoded()?);
    let mut mac = <Hmac<Sha512_256> as Mac>::new_from_slice(removal_key.as_slice())
        .map_err(|_| Error::NamedTeamMaterial)?;
    mac.update(&TEAM_REMOVAL_MAC_PAYLOAD_TYPE_ID.to_be_bytes());
    mac.update(encoded.as_slice());
    Ok(TeamRemovalAndCommitment {
        removal: TeamRemovalProof {
            mac: mac.finalize().into_bytes().into(),
            payload,
        },
        commitment: team_removal_key_commitment(removal_key)?,
    })
}

/// Boxes one team-removal key both to the team's admin PTK and to the member's
/// PUK/PTK. The same authenticated metadata is included in each independent
/// hybrid box.
#[allow(clippy::too_many_arguments)]
pub fn seal_team_removal_key(
    sender_seed: &SecretSeed,
    sender_hepk: &Hepk,
    team_receiver_hepk: &Hepk,
    team_receiver_role: Role,
    team_receiver_generation: u64,
    member_receiver_hepk: &Hepk,
    member_receiver_role: Role,
    member_receiver_generation: u64,
    removal_key: &SecretSeed,
    metadata: TeamRemovalKeyMetadata,
    randomness: [PukBoxRandomness; 2],
) -> Result<TeamRemovalBoxData> {
    if team_receiver_generation == 0 || member_receiver_generation == 0 {
        return Err(Error::NamedTeamMaterial);
    }
    let metadata_bytes = metadata.encoded()?;
    let mut payload = Zeroizing::new(Vec::with_capacity(35 + metadata_bytes.len()));
    payload.extend_from_slice(&[0x92, 0xc4, 32]);
    payload.extend_from_slice(removal_key.as_bytes());
    payload.extend_from_slice(&metadata_bytes);
    let team_box = TeamRemovalKeyBox {
        hybrid: seal_hybrid_payload(
            sender_seed,
            sender_hepk,
            team_receiver_hepk,
            TEAM_REMOVAL_KEY_BOX_PAYLOAD_TYPE_ID,
            payload.as_slice(),
            &randomness[0],
            true,
        )?,
        role: team_receiver_role,
        generation: team_receiver_generation,
    };
    let member_box = TeamRemovalKeyBox {
        hybrid: seal_hybrid_payload(
            sender_seed,
            sender_hepk,
            member_receiver_hepk,
            TEAM_REMOVAL_KEY_BOX_PAYLOAD_TYPE_ID,
            payload.as_slice(),
            &randomness[1],
            true,
        )?,
        role: member_receiver_role,
        generation: member_receiver_generation,
    };
    let commitment = team_removal_key_commitment(removal_key)?;
    Ok(TeamRemovalBoxData {
        commitment,
        team_box,
        member_box,
        metadata,
    })
}

/// Opens either side of a team-removal key box and binds the plaintext to its
/// authenticated chain commitment and exact metadata.
pub fn open_team_removal_key(
    boxed: &TeamRemovalKeyBox,
    receiver: &dyn HybridSecretDecapsulator,
    expected_role: Role,
    expected_generation: u64,
    expected_commitment: &[u8; 32],
    expected_metadata: &TeamRemovalKeyMetadata,
) -> Result<SecretSeed> {
    if boxed.role != expected_role || boxed.generation != expected_generation {
        return Err(Error::PukBinding);
    }
    let sender_dh = boxed.hybrid.sender_dh.as_ref().ok_or(Error::HybridBox)?;
    let cleartext = open_hybrid_box(
        &boxed.hybrid,
        receiver,
        sender_dh,
        TEAM_REMOVAL_KEY_BOX_PAYLOAD_TYPE_ID,
    )?;
    let payload = TeamRemovalKeyPayload::decode(&cleartext)?;
    if payload.metadata != *expected_metadata {
        return Err(Error::PukBinding);
    }
    let key = payload.into_key();
    let commitment = team_removal_key_commitment(&key)?;
    if &commitment != expected_commitment {
        return Err(Error::PukBinding);
    }
    Ok(key)
}

/// Opens an administrator copy of a member removal key when the original
/// destination role and addition sequence are known only inside the box.
/// The stable member identity fields and the chain commitment remain exact,
/// while authenticated destination metadata is returned to the caller.
pub struct TeamRemovalKeyExpectation<'a> {
    pub commitment: &'a [u8; 32],
    pub team: &'a EntityId,
    pub host: &'a EntityId,
    pub member: &'a EntityId,
    pub member_host: &'a EntityId,
    pub source_role: Role,
}

pub fn open_team_removal_key_for_member(
    boxed: &TeamRemovalKeyBox,
    receiver: &dyn HybridSecretDecapsulator,
    expected: &TeamRemovalKeyExpectation<'_>,
) -> Result<(SecretSeed, TeamRemovalKeyMetadata)> {
    let sender_dh = boxed.hybrid.sender_dh.as_ref().ok_or(Error::HybridBox)?;
    let cleartext = open_hybrid_box(
        &boxed.hybrid,
        receiver,
        sender_dh,
        TEAM_REMOVAL_KEY_BOX_PAYLOAD_TYPE_ID,
    )?;
    let payload = TeamRemovalKeyPayload::decode(&cleartext)?;
    if payload.metadata.team != *expected.team
        || payload.metadata.host != *expected.host
        || payload.metadata.member != *expected.member
        || payload.metadata.member_host != *expected.member_host
        || payload.metadata.source_role != expected.source_role
    {
        return Err(Error::PukBinding);
    }
    let metadata = payload.metadata.clone();
    let key = payload.into_key();
    if &team_removal_key_commitment(&key)? != expected.commitment {
        return Err(Error::PukBinding);
    }
    Ok((key, metadata))
}

/// Seals a federation bearer token to the target team's member-load-floor
/// PTK using the exact v0.1.9 typed SecretBox construction.
pub fn seal_team_remote_member_view_token(
    ptk_seed: &SecretSeed,
    payload: &foks_proto::TeamRemoteMemberViewTokenBoxPayload,
    nonce: [u8; 16],
) -> Result<SecretBox> {
    let key = derive_key(ptk_seed, 2, None)?;
    let cleartext = Zeroizing::new(payload.encoded()?);
    Ok(SecretBox {
        nonce,
        ciphertext: seal_typed_secretbox(
            key.as_bytes(),
            foks_proto::TEAM_REMOTE_MEMBER_VIEW_TOKEN_BOX_PAYLOAD_TYPE_ID,
            &nonce,
            cleartext.as_slice(),
            false,
        )?,
    })
}

/// Opens and authenticates a federation token box. Callers must additionally
/// bind the cleartext party and the outer PTK metadata to verified team state.
pub fn open_team_remote_member_view_token(
    ptk_seed: &SecretSeed,
    boxed: &SecretBox,
) -> Result<foks_proto::TeamRemoteMemberViewTokenBoxPayload> {
    let key = derive_key(ptk_seed, 2, None)?;
    let cleartext = open_typed_secretbox(
        key.as_bytes(),
        foks_proto::TEAM_REMOTE_MEMBER_VIEW_TOKEN_BOX_PAYLOAD_TYPE_ID,
        &boxed.nonce,
        &boxed.ciphertext,
    )?;
    foks_proto::TeamRemoteMemberViewTokenBoxPayload::decode(&cleartext).map_err(Into::into)
}
