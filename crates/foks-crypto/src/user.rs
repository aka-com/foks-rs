//! User eldest links, device provisioning, revocation, and PUK rotation links.

use super::*;

/// Complete public result of constructing a software-device eldest link.
pub struct SoftwareEldestMaterial {
    pub uid: EntityId,
    pub device: DevicePublicMaterial,
    pub puk: SharedPublicMaterial,
    pub link: UserLink,
}

/// Complete public result of constructing a Yubi-parent eldest link.
pub struct YubiEldestMaterial {
    pub uid: EntityId,
    pub device: DevicePublicMaterial,
    pub subkey: DevicePublicMaterial,
    pub puk: SharedPublicMaterial,
    pub link: UserLink,
}

pub struct UserMutationBase<'a> {
    pub uid: &'a EntityId,
    pub host: &'a EntityId,
    pub seqno: u64,
    pub previous: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
}

pub struct SoftwareProvisionInput<'a> {
    pub base: UserMutationBase<'a>,
    pub role: Role,
    pub device_label: &'a foks_proto::DeviceLabel,
    pub device_name_commitment_key: [u8; 16],
}

pub struct SoftwareProvisionMaterial {
    pub link: UserLink,
    pub device: DevicePublicMaterial,
    pub introduced_puk: Option<SharedPublicMaterial>,
    pub device_name_commitment_key: [u8; 16],
    pub next_tree_location: [u8; 32],
}

pub struct YubiProvisionMaterial {
    pub link: UserLink,
    pub device: DevicePublicMaterial,
    pub subkey: DevicePublicMaterial,
    pub introduced_puk: Option<SharedPublicMaterial>,
    pub device_name_commitment_key: [u8; 16],
    pub next_tree_location: [u8; 32],
}

/// Constructs a complete software-device provision link. If the requested
/// role has no PUK yet, `introduced_puk` supplies generation 1 and signs first.
/// The new device countersigns before the existing owner device.
pub fn make_software_provision_link(
    input: &SoftwareProvisionInput<'_>,
    existing_device_seed: &SecretSeed,
    new_device_seed: &SecretSeed,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    make_provision_link(
        input,
        existing_device_seed,
        foks_proto::ENTITY_DEVICE,
        foks_proto::ENTITY_DEVICE,
        new_device_seed,
        introduced_puk,
    )
}

/// Constructs a provisioning link for a hardware Yubi parent and its
/// delegated Ed25519 mTLS subkey. The new PUK (when present), subkey, parent,
/// and existing software owner sign the stack in that order.
pub fn make_yubi_provision_link(
    input: &SoftwareProvisionInput<'_>,
    existing_device_seed: &SecretSeed,
    parent: &dyn YubiDevice,
    subkey_seed: &SecretSeed,
    introduced_puk: Option<&SecretSeed>,
) -> Result<YubiProvisionMaterial> {
    if input.device_label.device_type != foks_proto::DeviceType::YubiKey
        || input.role == Role::NONE
        || parent.entity_id().entity_type() != foks_proto::ENTITY_YUBI
        || parent.hepk().p256().is_none()
    {
        return Err(Error::DeviceKey);
    }
    let existing = derive_device_public(existing_device_seed)?;
    let device = DevicePublicMaterial {
        id: parent.entity_id().clone(),
        hepk: parent.hepk().clone(),
    };
    let subkey = derive_public_material(subkey_seed, foks_proto::ENTITY_SUBKEY)?;
    let introduced = introduced_puk
        .map(|seed| derive_shared_public(seed, foks_proto::ENTITY_PUK_VERIFY))
        .transpose()?;
    let label = input.device_label;
    let label_bytes = encode(&Value::Array(vec![
        Value::Unsigned(label.device_type.protocol_value()),
        Value::Text(label.normalized_name.clone()),
        Value::Unsigned(label.serial),
    ]))?;
    let shared_keys = match introduced.as_ref() {
        Some(key) => vec![UserSharedKey {
            generation: 1,
            role: input.role,
            verify_key: key.verify_key.clone(),
            hepk_fingerprint: hepk_fingerprint(&key.hepk)?,
        }],
        None => Vec::new(),
    };
    let change = UserGroupChange {
        seqno: input.base.seqno,
        previous: Some(input.base.previous),
        root: input.base.root.clone(),
        time: input.base.time,
        next_location_commitment: tree_location_commitment(&input.base.next_tree_location)?,
        uid: input.base.uid.clone(),
        host: input.base.host.clone(),
        signer: existing.id,
        changes: vec![UserMemberChange {
            role: input.role,
            entity: device.id.clone(),
            scoped_host: None,
            source_role: Role::NONE,
            keys: UserMemberKeys::User {
                hepk_fingerprint: hepk_fingerprint(&device.hepk)?,
                subkey: Some(subkey.id.clone()),
            },
        }],
        shared_keys,
        metadata: vec![ChangeMetadata::DeviceName(commitment(
            DEVICE_LABEL_TYPE_ID,
            &label_bytes,
            &input.device_name_commitment_key,
        ))],
    };
    let unsigned = UnsignedUserLink::user_group_change(&change)?;
    let mut signatures = Vec::with_capacity(if introduced_puk.is_some() { 4 } else { 3 });
    if let Some(seed) = introduced_puk {
        signatures.push(sign_seed_typed(
            seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_seed_typed(
        subkey_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    signatures.push(sign_yubi_typed(
        parent,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    signatures.push(sign_seed_typed(
        existing_device_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    Ok(YubiProvisionMaterial {
        link: unsigned.finish(signatures)?,
        device,
        subkey,
        introduced_puk: introduced,
        device_name_commitment_key: input.device_name_commitment_key,
        next_tree_location: input.base.next_tree_location,
    })
}

/// Constructs the same provision link with a FOKS backup key as the new
/// member. Backup keys have a distinct EntityID type but otherwise use the
/// exact Curve25519/ML-KEM software suite.
pub fn make_backup_provision_link(
    input: &SoftwareProvisionInput<'_>,
    existing_device_seed: &SecretSeed,
    backup: &BackupKey,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    let backup_seed = backup.derived_seed();
    make_provision_link(
        input,
        existing_device_seed,
        foks_proto::ENTITY_DEVICE,
        foks_proto::ENTITY_BACKUP_KEY,
        &backup_seed,
        introduced_puk,
    )
}

/// Provisions a permanent software device while an ephemeral backup key is
/// the authenticated existing member and final countersigner.
pub fn make_software_provision_link_from_backup(
    input: &SoftwareProvisionInput<'_>,
    backup: &BackupKey,
    new_device_seed: &SecretSeed,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    let backup_seed = backup.derived_seed();
    make_software_provision_link_from_backup_seed(
        input,
        &backup_seed,
        new_device_seed,
        introduced_puk,
    )
}

pub(super) fn make_software_provision_link_from_backup_seed(
    input: &SoftwareProvisionInput<'_>,
    backup_seed: &SecretSeed,
    new_device_seed: &SecretSeed,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    make_provision_link(
        input,
        backup_seed,
        foks_proto::ENTITY_BACKUP_KEY,
        foks_proto::ENTITY_DEVICE,
        new_device_seed,
        introduced_puk,
    )
}

pub fn make_software_provision_link_from_backup_credential(
    input: &SoftwareProvisionInput<'_>,
    backup: &BackupKeyMaterial,
    new_device_seed: &SecretSeed,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    make_software_provision_link_from_backup_seed(
        input,
        &backup.seed,
        new_device_seed,
        introduced_puk,
    )
}

pub(super) fn key_provision_unsigned(
    input: &SoftwareProvisionInput<'_>,
    existing: &EntityId,
    device: &DevicePublicMaterial,
    introduced: Option<&SharedPublicMaterial>,
) -> Result<UnsignedUserLink> {
    let label = input.device_label;
    let label_bytes = encode(&Value::Array(vec![
        Value::Unsigned(label.device_type.protocol_value()),
        Value::Text(label.normalized_name.clone()),
        Value::Unsigned(label.serial),
    ]))?;
    let shared_keys = match introduced {
        Some(key) => {
            vec![UserSharedKey {
                generation: 1,
                role: input.role,
                verify_key: key.verify_key.clone(),
                hepk_fingerprint: hepk_fingerprint(&key.hepk)?,
            }]
        }
        None => Vec::new(),
    };
    let change = UserGroupChange {
        seqno: input.base.seqno,
        previous: Some(input.base.previous),
        root: input.base.root.clone(),
        time: input.base.time,
        next_location_commitment: tree_location_commitment(&input.base.next_tree_location)?,
        uid: input.base.uid.clone(),
        host: input.base.host.clone(),
        signer: existing.clone(),
        changes: vec![UserMemberChange {
            role: input.role,
            entity: device.id.clone(),
            scoped_host: None,
            source_role: Role::NONE,
            keys: UserMemberKeys::User {
                hepk_fingerprint: hepk_fingerprint(&device.hepk)?,
                subkey: None,
            },
        }],
        shared_keys,
        metadata: vec![ChangeMetadata::DeviceName(commitment(
            DEVICE_LABEL_TYPE_ID,
            &label_bytes,
            &input.device_name_commitment_key,
        ))],
    };
    UnsignedUserLink::user_group_change(&change).map_err(Into::into)
}

/// Software token/device provisioning shares the exact group-change construction.
pub fn make_software_key_provision_link(
    input: &SoftwareProvisionInput<'_>,
    existing_seed: &SecretSeed,
    existing_kind: SoftwareKeyKind,
    new_seed: &SecretSeed,
    new_kind: SoftwareKeyKind,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    make_provision_link(
        input,
        existing_seed,
        existing_kind.entity_type(),
        new_kind.entity_type(),
        new_seed,
        introduced_puk,
    )
}

/// A hardware owner countersigns enrollment of a software bot key.
pub fn make_bot_provision_link_from_yubi(
    input: &SoftwareProvisionInput<'_>,
    parent: &dyn YubiDevice,
    bot: &BotToken,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    let device = bot.public_material()?;
    let introduced = introduced_puk
        .map(|s| derive_shared_public(s, foks_proto::ENTITY_PUK_VERIFY))
        .transpose()?;
    let unsigned = key_provision_unsigned(input, parent.entity_id(), &device, introduced.as_ref())?;
    let mut signatures = Vec::new();
    if let Some(seed) = introduced_puk {
        signatures.push(sign_seed_typed(
            seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_seed_typed(
        &bot.derived_seed(),
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    signatures.push(sign_yubi_typed(
        parent,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    Ok(SoftwareProvisionMaterial {
        link: unsigned.finish(signatures)?,
        device,
        introduced_puk: introduced,
        device_name_commitment_key: input.device_name_commitment_key,
        next_tree_location: input.base.next_tree_location,
    })
}

pub(super) fn make_provision_link(
    input: &SoftwareProvisionInput<'_>,
    existing_device_seed: &SecretSeed,
    existing_entity_type: u8,
    new_entity_type: u8,
    new_device_seed: &SecretSeed,
    introduced_puk: Option<&SecretSeed>,
) -> Result<SoftwareProvisionMaterial> {
    let existing = derive_public_material(existing_device_seed, existing_entity_type)?;
    let device = derive_public_material(new_device_seed, new_entity_type)?;
    let introduced = introduced_puk
        .map(|seed| derive_shared_public(seed, foks_proto::ENTITY_PUK_VERIFY))
        .transpose()?;
    let unsigned = key_provision_unsigned(input, &existing.id, &device, introduced.as_ref())?;
    let mut signatures = Vec::new();
    if let Some(seed) = introduced_puk {
        signatures.push(sign_seed_typed(
            seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_seed_typed(
        new_device_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    signatures.push(sign_seed_typed(
        existing_device_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    Ok(SoftwareProvisionMaterial {
        link: unsigned.finish(signatures)?,
        device,
        introduced_puk: introduced,
        device_name_commitment_key: input.device_name_commitment_key,
        next_tree_location: input.base.next_tree_location,
    })
}

pub struct PukRotation<'a> {
    pub role: Role,
    pub generation: u64,
    pub seed: &'a SecretSeed,
}

pub fn make_software_revoke_link(
    base: &UserMutationBase<'_>,
    signing_device_seed: &SecretSeed,
    target: &EntityId,
    rotations: &[PukRotation<'_>],
) -> Result<UserLink> {
    make_software_puk_change_link(base, signing_device_seed, Some(target), rotations)
}

/// Constructs a standalone PUK-rotation link. FOKS uses the revoke RPC for
/// this operation but authenticates an empty member-change list.
pub fn make_software_puk_rotation_link(
    base: &UserMutationBase<'_>,
    signing_device_seed: &SecretSeed,
    rotations: &[PukRotation<'_>],
) -> Result<UserLink> {
    if rotations.is_empty() {
        return Err(Error::PukBinding);
    }
    make_software_puk_change_link(base, signing_device_seed, None, rotations)
}

/// Constructs a standalone PUK-rotation link signed by an enrolled Yubi
/// parent. Each rotated PUK signs the accumulated stack first, in shared-key
/// order, and the P-256 parent signs last as required by user-chain replay.
pub fn make_yubi_puk_rotation_link(
    base: &UserMutationBase<'_>,
    parent: &dyn YubiDevice,
    rotations: &[PukRotation<'_>],
) -> Result<UserLink> {
    make_yubi_puk_change_link(base, parent, None, rotations)
}
pub fn make_yubi_revoke_link(
    base: &UserMutationBase<'_>,
    parent: &dyn YubiDevice,
    target: &EntityId,
    rotations: &[PukRotation<'_>],
) -> Result<UserLink> {
    make_yubi_puk_change_link(base, parent, Some(target), rotations)
}
pub(super) fn make_yubi_puk_change_link(
    base: &UserMutationBase<'_>,
    parent: &dyn YubiDevice,
    target: Option<&EntityId>,
    rotations: &[PukRotation<'_>],
) -> Result<UserLink> {
    if rotations.is_empty() {
        return Err(Error::PukBinding);
    }
    if parent.entity_id().entity_type() != foks_proto::ENTITY_YUBI
        || parent.entity_id().p256_key().ok().as_ref() != parent.hepk().p256()
    {
        return Err(Error::DeviceKey);
    }
    let mut shared_keys = Vec::with_capacity(rotations.len());
    for rotation in rotations {
        let public = derive_shared_public(rotation.seed, foks_proto::ENTITY_PUK_VERIFY)?;
        shared_keys.push(UserSharedKey {
            generation: rotation.generation,
            role: rotation.role,
            verify_key: public.verify_key,
            hepk_fingerprint: hepk_fingerprint(&public.hepk)?,
        });
    }
    let change = UserGroupChange {
        seqno: base.seqno,
        previous: Some(base.previous),
        root: base.root.clone(),
        time: base.time,
        next_location_commitment: tree_location_commitment(&base.next_tree_location)?,
        uid: base.uid.clone(),
        host: base.host.clone(),
        signer: parent.entity_id().clone(),
        changes: target
            .map(|target| {
                vec![UserMemberChange {
                    role: Role::NONE,
                    entity: target.clone(),
                    scoped_host: None,
                    source_role: Role::NONE,
                    keys: UserMemberKeys::None,
                }]
            })
            .unwrap_or_default(),
        shared_keys,
        metadata: Vec::new(),
    };
    let unsigned = UnsignedUserLink::user_group_change(&change)?;
    let mut signatures = Vec::with_capacity(rotations.len() + 1);
    for rotation in rotations {
        signatures.push(sign_seed_typed(
            rotation.seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_yubi_typed(
        parent,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    unsigned.finish(signatures).map_err(Into::into)
}

pub(super) fn make_software_puk_change_link(
    base: &UserMutationBase<'_>,
    signing_device_seed: &SecretSeed,
    target: Option<&EntityId>,
    rotations: &[PukRotation<'_>],
) -> Result<UserLink> {
    let signer = derive_device_public(signing_device_seed)?;
    let mut shared_keys = Vec::with_capacity(rotations.len());
    for rotation in rotations {
        let public = derive_shared_public(rotation.seed, foks_proto::ENTITY_PUK_VERIFY)?;
        shared_keys.push(UserSharedKey {
            generation: rotation.generation,
            role: rotation.role,
            verify_key: public.verify_key,
            hepk_fingerprint: hepk_fingerprint(&public.hepk)?,
        });
    }
    let change = UserGroupChange {
        seqno: base.seqno,
        previous: Some(base.previous),
        root: base.root.clone(),
        time: base.time,
        next_location_commitment: tree_location_commitment(&base.next_tree_location)?,
        uid: base.uid.clone(),
        host: base.host.clone(),
        signer: signer.id,
        changes: target
            .map(|target| {
                vec![UserMemberChange {
                    role: Role::NONE,
                    entity: target.clone(),
                    scoped_host: None,
                    source_role: Role::NONE,
                    keys: UserMemberKeys::None,
                }]
            })
            .unwrap_or_default(),
        shared_keys,
        metadata: Vec::new(),
    };
    let unsigned = UnsignedUserLink::user_group_change(&change)?;
    let mut signatures = Vec::with_capacity(rotations.len() + 1);
    for rotation in rotations {
        signatures.push(sign_seed_typed(
            rotation.seed,
            LINK_OUTER_V1_TYPE_ID,
            &unsigned.signing_bytes(&signatures)?,
        )?);
    }
    signatures.push(sign_seed_typed(
        signing_device_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&signatures)?,
    )?);
    unsigned.finish(signatures).map_err(Into::into)
}

/// Caller-controlled inputs that are intentionally retained after signup.
/// The two seeds and permission token are not part of this structure and must
/// already be durable in an encrypted credential store before submission.
pub struct SoftwareEldestInput<'a> {
    pub host: &'a EntityId,
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub subchain_tree_location: [u8; 32],
    pub normalized_username: &'a [u8],
    pub username_sequence: u64,
    pub username_commitment_key: [u8; 16],
    pub device_name: &'a DeviceLabelNameAndCommitmentKey,
}

/// Constructs and stacked-signs the exact v0.1.9 eldest link for one software
/// owner device and its first owner PUK.
pub fn make_software_eldest_link(
    input: &SoftwareEldestInput<'_>,
    device_seed: &SecretSeed,
    puk_seed: &SecretSeed,
) -> Result<SoftwareEldestMaterial> {
    if input.normalized_username.is_empty()
        || !input.normalized_username.is_ascii()
        || input.username_sequence == 0
    {
        return Err(Error::DeviceKey);
    }
    let host = input.host.clone().require_type(foks_proto::ENTITY_HOST)?;
    let device = derive_device_public(device_seed)?;
    let puk = derive_shared_public(puk_seed, foks_proto::ENTITY_PUK_VERIFY)?;
    let mut uid_bytes = puk.verify_key.as_bytes().to_vec();
    uid_bytes[0] = foks_proto::ENTITY_USER;
    let uid = EntityId::from_bytes(uid_bytes)?;

    let username_object = encode(&Value::Array(vec![
        Value::Text(input.normalized_username.to_vec()),
        Value::Unsigned(input.username_sequence),
    ]))?;
    let label = &input.device_name.label;
    let device_label_object = encode(&Value::Array(vec![
        Value::Unsigned(label.device_type.protocol_value()),
        Value::Text(label.normalized_name.clone()),
        Value::Unsigned(label.serial),
    ]))?;
    let device_hepk_fingerprint = hepk_fingerprint(&device.hepk)?;
    let puk_hepk_fingerprint = hepk_fingerprint(&puk.hepk)?;
    let public = SoftwareEldestPublic {
        host: &host,
        uid: &uid,
        device: &device.id,
        device_hepk_fingerprint,
        puk_verify_key: &puk.verify_key,
        puk_hepk_fingerprint,
        root: input.root,
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        username_commitment: commitment(
            NAME_COMMITMENT_TYPE_ID,
            &username_object,
            &input.username_commitment_key,
        ),
        device_name_commitment: commitment(
            DEVICE_LABEL_TYPE_ID,
            &device_label_object,
            &input.device_name.commitment_key,
        ),
        subchain_location_commitment: tree_location_commitment(&input.subchain_tree_location)?,
    };
    let unsigned = UnsignedUserLink::software_eldest(&public)?;
    let puk_signature = sign_seed_typed(
        puk_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&[])?,
    )?;
    let device_signature = sign_seed_typed(
        device_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(std::slice::from_ref(&puk_signature))?,
    )?;
    let link = unsigned.finish(vec![puk_signature, device_signature])?;
    Ok(SoftwareEldestMaterial {
        uid,
        device,
        puk,
        link,
    })
}

/// Constructs the exact v0.1.9 eldest stack for a Yubi parent: owner PUK,
/// delegated Ed25519 subkey, then the P-256 parent signature.
pub fn make_yubi_eldest_link(
    input: &SoftwareEldestInput<'_>,
    device: &dyn YubiDevice,
    subkey_seed: &SecretSeed,
    puk_seed: &SecretSeed,
) -> Result<YubiEldestMaterial> {
    if input.normalized_username.is_empty()
        || !input.normalized_username.is_ascii()
        || input.username_sequence == 0
        || device.entity_id().entity_type() != foks_proto::ENTITY_YUBI
        || device.hepk().p256().is_none()
    {
        return Err(Error::DeviceKey);
    }
    let host = input.host.clone().require_type(foks_proto::ENTITY_HOST)?;
    let parent = DevicePublicMaterial {
        id: device.entity_id().clone(),
        hepk: device.hepk().clone(),
    };
    let subkey = derive_public_material(subkey_seed, foks_proto::ENTITY_SUBKEY)?;
    let puk = derive_shared_public(puk_seed, foks_proto::ENTITY_PUK_VERIFY)?;
    let mut uid_bytes = puk.verify_key.as_bytes().to_vec();
    uid_bytes[0] = foks_proto::ENTITY_USER;
    let uid = EntityId::from_bytes(uid_bytes)?;
    let username_object = encode(&Value::Array(vec![
        Value::Text(input.normalized_username.to_vec()),
        Value::Unsigned(input.username_sequence),
    ]))?;
    let label = &input.device_name.label;
    let device_label_object = encode(&Value::Array(vec![
        Value::Unsigned(label.device_type.protocol_value()),
        Value::Text(label.normalized_name.clone()),
        Value::Unsigned(label.serial),
    ]))?;
    let unsigned = UnsignedUserLink::yubi_eldest(&YubiEldestPublic {
        host: &host,
        uid: &uid,
        device: &parent.id,
        subkey: &subkey.id,
        device_hepk_fingerprint: hepk_fingerprint(&parent.hepk)?,
        puk_verify_key: &puk.verify_key,
        puk_hepk_fingerprint: hepk_fingerprint(&puk.hepk)?,
        root: input.root,
        time: input.time,
        next_location_commitment: tree_location_commitment(&input.next_tree_location)?,
        username_commitment: commitment(
            NAME_COMMITMENT_TYPE_ID,
            &username_object,
            &input.username_commitment_key,
        ),
        device_name_commitment: commitment(
            DEVICE_LABEL_TYPE_ID,
            &device_label_object,
            &input.device_name.commitment_key,
        ),
        subchain_location_commitment: tree_location_commitment(&input.subchain_tree_location)?,
    })?;
    let puk_signature = sign_seed_typed(
        puk_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&[])?,
    )?;
    let subkey_signature = sign_seed_typed(
        subkey_seed,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(std::slice::from_ref(&puk_signature))?,
    )?;
    let prefix = [puk_signature, subkey_signature];
    let parent_signature = sign_yubi_typed(
        device,
        LINK_OUTER_V1_TYPE_ID,
        &unsigned.signing_bytes(&prefix)?,
    )?;
    Ok(YubiEldestMaterial {
        uid,
        device: parent,
        subkey,
        puk,
        link: unsigned.finish(vec![prefix[0].clone(), prefix[1].clone(), parent_signature])?,
    })
}
