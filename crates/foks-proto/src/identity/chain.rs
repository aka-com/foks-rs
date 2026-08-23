//! Exact user/team outer links, membership changes, roles, and metadata.

use crate::{
    array, binary, decode, device_entity, encode, entity, expect_unsigned, fixed_blob, integer,
    list, option, signature, type_error, unsigned, variant, EntityId, Error, Result, Role,
    Signature, TreeRoot, Value, ENTITY_AD_HOC_TEAM, ENTITY_DEVICE, ENTITY_HOST, ENTITY_NAMED_TEAM,
    ENTITY_PTK_VERIFY, ENTITY_PUK_VERIFY, ENTITY_USER, ENTITY_YUBI,
};

/// Exact v0.1.9 user-chain outer link. The inner blob is retained verbatim
/// because both stacked signatures and the Merkle leaf commit to these bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserLink {
    inner: Vec<u8>,
    signatures: Vec<Signature>,
    exact: Vec<u8>,
}

/// Public inputs committed by a software-device user eldest link.
///
/// Commitment values are supplied rather than recomputed here so this exact
/// wire builder remains independent of cryptographic policy. Callers should
/// use `foks-crypto`'s high-level constructor for standard account creation.
pub struct SoftwareEldestPublic<'a> {
    pub host: &'a EntityId,
    pub uid: &'a EntityId,
    pub device: &'a EntityId,
    pub device_hepk_fingerprint: [u8; 32],
    pub puk_verify_key: &'a EntityId,
    pub puk_hepk_fingerprint: [u8; 32],
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_location_commitment: [u8; 32],
    pub username_commitment: [u8; 32],
    pub device_name_commitment: [u8; 32],
    pub subchain_location_commitment: [u8; 32],
}

/// Exact unsigned `LinkOuterV1` state used by FOKS stacked signatures.
pub struct UnsignedUserLink {
    inner: Vec<u8>,
}

impl UnsignedUserLink {
    pub fn software_eldest(input: &SoftwareEldestPublic<'_>) -> Result<Self> {
        input.host.clone().require_type(ENTITY_HOST)?;
        input.uid.clone().require_type(ENTITY_USER)?;
        input.device.clone().require_type(ENTITY_DEVICE)?;
        input
            .puk_verify_key
            .clone()
            .require_type(ENTITY_PUK_VERIFY)?;

        let member = Value::Array(vec![
            Value::Array(vec![
                Value::Binary(input.device.as_bytes().to_vec()),
                Value::Null,
            ]),
            Role::NONE.to_value(),
            Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((
                    b"1".to_vec(),
                    Box::new(Value::Array(vec![
                        Value::Binary(input.device_hepk_fingerprint.to_vec()),
                        Value::Null,
                    ])),
                ))),
            ]),
        ]);
        let group_change = Value::Array(vec![
            Value::Array(vec![
                Value::Array(vec![
                    Value::Unsigned(1),
                    Value::Null,
                    Value::Array(vec![
                        Value::Unsigned(input.root.epoch),
                        Value::Binary(input.root.hash.to_vec()),
                    ]),
                    Value::Unsigned(input.time),
                ]),
                Value::Binary(input.next_location_commitment.to_vec()),
            ]),
            Value::Array(vec![
                Value::Binary(input.uid.as_bytes().to_vec()),
                Value::Binary(input.host.as_bytes().to_vec()),
            ]),
            Value::Array(vec![
                Value::Binary(input.device.as_bytes().to_vec()),
                Value::Null,
            ]),
            Value::Array(vec![Value::Array(vec![Role::OWNER.to_value(), member])]),
            Value::Null,
            Value::Array(vec![Value::Array(vec![
                Value::Unsigned(1),
                Role::OWNER.to_value(),
                Value::Binary(input.puk_verify_key.as_bytes().to_vec()),
                Value::Binary(input.puk_hepk_fingerprint.to_vec()),
            ])]),
            Value::Array(vec![
                Value::Array(vec![
                    Value::Unsigned(1),
                    Value::Variant(Some((
                        b"0".to_vec(),
                        Box::new(Value::Binary(input.username_commitment.to_vec())),
                    ))),
                ]),
                Value::Array(vec![
                    Value::Unsigned(0),
                    Value::Variant(Some((
                        b"0".to_vec(),
                        Box::new(Value::Binary(input.device_name_commitment.to_vec())),
                    ))),
                ]),
                Value::Array(vec![
                    Value::Unsigned(2),
                    Value::Variant(Some((
                        b"1".to_vec(),
                        Box::new(Value::Array(vec![Value::Binary(
                            input.subchain_location_commitment.to_vec(),
                        )])),
                    ))),
                ]),
            ]),
            Value::Null,
        ]);
        Ok(Self {
            inner: encode(&Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((b"0".to_vec(), Box::new(group_change)))),
            ]))?,
        })
    }

    /// Builds the exact canonical v0.1.9 user group-change payload used for
    /// provisioning, revocation, and standalone PUK rotation.
    pub fn user_group_change(change: &UserGroupChange) -> Result<Self> {
        change.uid.clone().require_type(ENTITY_USER)?;
        change.host.clone().require_type(ENTITY_HOST)?;
        if change.seqno < 2 || change.previous.is_none() {
            return Err(Error::IntegerRange("non-eldest user sequence"));
        }
        let option_blob =
            |value: Option<&[u8]>| value.map_or(Value::Null, |bytes| Value::Binary(bytes.to_vec()));
        let group_change = Value::Array(vec![
            Value::Array(vec![
                Value::Array(vec![
                    Value::Unsigned(change.seqno),
                    option_blob(change.previous.as_ref().map(<[u8; 32]>::as_slice)),
                    Value::Array(vec![
                        Value::Unsigned(change.root.epoch),
                        Value::Binary(change.root.hash.to_vec()),
                    ]),
                    Value::Unsigned(change.time),
                ]),
                Value::Binary(change.next_location_commitment.to_vec()),
            ]),
            Value::Array(vec![
                Value::Binary(change.uid.as_bytes().to_vec()),
                Value::Binary(change.host.as_bytes().to_vec()),
            ]),
            Value::Array(vec![
                Value::Binary(change.signer.as_bytes().to_vec()),
                Value::Null,
            ]),
            list_or_null(change.changes.iter().map(user_member_change_value)),
            Value::Null,
            list_or_null(change.shared_keys.iter().map(user_shared_key_value)),
            list_or_null(change.metadata.iter().map(change_metadata_value)),
            Value::Null,
        ]);
        Ok(Self {
            inner: encode(&Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((b"0".to_vec(), Box::new(group_change)))),
            ]))?,
        })
    }

    pub fn team_group_change(change: &TeamGroupChange) -> Result<Self> {
        change.host.clone().require_type(ENTITY_HOST)?;
        if !matches!(
            change.team.entity_type(),
            ENTITY_NAMED_TEAM | ENTITY_AD_HOC_TEAM
        ) {
            return Err(Error::WrongEntityType {
                expected: ENTITY_NAMED_TEAM,
                found: change.team.entity_type(),
            });
        }
        if change.seqno == 0 || (change.seqno == 1) != change.previous.is_none() {
            return Err(Error::IntegerRange("team chain sequence"));
        }
        let group_change = Value::Array(vec![
            Value::Array(vec![
                Value::Array(vec![
                    Value::Unsigned(change.seqno),
                    change
                        .previous
                        .map_or(Value::Null, |hash| Value::Binary(hash.to_vec())),
                    Value::Array(vec![
                        Value::Unsigned(change.root.epoch),
                        Value::Binary(change.root.hash.to_vec()),
                    ]),
                    Value::Unsigned(change.time),
                ]),
                Value::Binary(change.next_location_commitment.to_vec()),
            ]),
            Value::Array(vec![
                Value::Binary(change.team.as_bytes().to_vec()),
                Value::Binary(change.host.as_bytes().to_vec()),
            ]),
            Value::Array(vec![
                Value::Binary(change.signer.as_bytes().to_vec()),
                Value::Array(vec![
                    Value::Binary(change.signer_owner.party.as_bytes().to_vec()),
                    change.signer_owner.source_role.to_value(),
                ]),
            ]),
            list_or_null(change.changes.iter().map(team_member_change_value)),
            Value::Null,
            list_or_null(change.shared_keys.iter().map(user_shared_key_value)),
            list_or_null(change.metadata.iter().map(change_metadata_value)),
            Value::Null,
        ]);
        Ok(Self {
            inner: encode(&Value::Array(vec![
                Value::Unsigned(1),
                Value::Variant(Some((b"0".to_vec(), Box::new(group_change)))),
            ]))?,
        })
    }

    pub fn approved_adhoc_membership(input: &AdHocMembershipLinkPublic<'_>) -> Result<Self> {
        input.user.clone().require_type(ENTITY_USER)?;
        input.host.clone().require_type(ENTITY_HOST)?;
        if !matches!(input.signer.entity_type(), ENTITY_DEVICE | ENTITY_YUBI) {
            return Err(Error::WrongEntityType {
                expected: ENTITY_DEVICE,
                found: input.signer.entity_type(),
            });
        }
        input.team.clone().require_type(ENTITY_AD_HOC_TEAM)?;
        if input.sequence != 1 || input.previous.is_some() || input.team_sequence != 1 {
            return Err(Error::IntegerRange("ad-hoc membership sequence"));
        }
        let membership = Value::Array(vec![
            Value::Array(vec![
                Value::Binary(input.team.as_bytes().to_vec()),
                Value::Binary(input.host.as_bytes().to_vec()),
            ]),
            input.source_role.to_value(),
            Value::Array(vec![
                Value::Unsigned(4),
                Value::Variant(Some((
                    b"2".to_vec(),
                    Box::new(Value::Array(vec![
                        input.destination_role.to_value(),
                        Value::Unsigned(input.team_sequence),
                    ])),
                ))),
            ]),
        ]);
        let generic = Value::Array(vec![
            Value::Array(vec![
                Value::Array(vec![
                    Value::Unsigned(input.sequence),
                    input
                        .previous
                        .map_or(Value::Null, |hash| Value::Binary(hash.to_vec())),
                    Value::Array(vec![
                        Value::Unsigned(input.root.epoch),
                        Value::Binary(input.root.hash.to_vec()),
                    ]),
                    Value::Unsigned(input.time),
                ]),
                Value::Binary(input.next_location_commitment.to_vec()),
            ]),
            Value::Array(vec![
                Value::Binary(input.user.as_bytes().to_vec()),
                Value::Binary(input.host.as_bytes().to_vec()),
            ]),
            Value::Array(vec![
                Value::Binary(input.signer.as_bytes().to_vec()),
                Value::Null,
            ]),
            Value::Array(vec![
                Value::Unsigned(4),
                Value::Variant(Some((b"1".to_vec(), Box::new(membership)))),
            ]),
        ]);
        Ok(Self {
            inner: encode(&Value::Array(vec![
                Value::Unsigned(2),
                Value::Variant(Some((b"1".to_vec(), Box::new(generic)))),
            ]))?,
        })
    }

    pub fn signing_bytes(&self, signatures: &[Signature]) -> Result<Vec<u8>> {
        let signatures = if signatures.is_empty() {
            Value::Null
        } else {
            Value::Array(signatures.iter().map(Signature::to_value).collect())
        };
        Ok(encode(&Value::Array(vec![
            Value::Binary(self.inner.clone()),
            signatures,
        ]))?)
    }

    pub fn finish(self, signatures: Vec<Signature>) -> Result<UserLink> {
        if signatures.is_empty() {
            return Err(Error::FieldCount {
                expected: 1,
                found: 0,
            });
        }
        let exact = encode(&Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"1".to_vec(),
                Box::new(Value::Array(vec![
                    Value::Binary(self.inner.clone()),
                    Value::Array(signatures.iter().map(Signature::to_value).collect()),
                ])),
            ))),
        ]))?;
        Ok(UserLink {
            inner: self.inner,
            signatures,
            exact,
        })
    }
}

pub struct AdHocMembershipLinkPublic<'a> {
    pub user: &'a EntityId,
    pub host: &'a EntityId,
    pub signer: &'a EntityId,
    pub sequence: u64,
    pub previous: Option<[u8; 32]>,
    pub root: &'a TreeRoot,
    pub time: u64,
    pub next_location_commitment: [u8; 32],
    pub team: &'a EntityId,
    pub source_role: Role,
    pub destination_role: Role,
    pub team_sequence: u64,
}

pub(crate) fn list_or_null(values: impl Iterator<Item = Value>) -> Value {
    let values = values.collect::<Vec<_>>();
    if values.is_empty() {
        Value::Null
    } else {
        Value::Array(values)
    }
}

fn user_member_change_value(change: &UserMemberChange) -> Value {
    let scoped_host = change
        .scoped_host
        .as_ref()
        .map_or(Value::Null, |host| Value::Binary(host.as_bytes().to_vec()));
    let keys = match &change.keys {
        UserMemberKeys::None => Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]),
        UserMemberKeys::User {
            hepk_fingerprint,
            subkey,
        } => Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"1".to_vec(),
                Box::new(Value::Array(vec![
                    Value::Binary(hepk_fingerprint.to_vec()),
                    subkey
                        .as_ref()
                        .map_or(Value::Null, |key| Value::Binary(key.as_bytes().to_vec())),
                ])),
            ))),
        ]),
    };
    Value::Array(vec![
        change.role.to_value(),
        Value::Array(vec![
            Value::Array(vec![
                Value::Binary(change.entity.as_bytes().to_vec()),
                scoped_host,
            ]),
            change.source_role.to_value(),
            keys,
        ]),
    ])
}

fn team_member_change_value(change: &TeamMemberChange) -> Value {
    let keys = change.keys.as_ref().map_or(
        Value::Array(vec![Value::Unsigned(0), Value::Variant(None)]),
        |keys| {
            Value::Array(vec![
                Value::Unsigned(2),
                Value::Variant(Some((
                    b"2".to_vec(),
                    Box::new(Value::Array(vec![
                        Value::Binary(keys.verify_key.as_bytes().to_vec()),
                        Value::Binary(keys.hepk_fingerprint.to_vec()),
                        Value::Unsigned(keys.generation),
                        keys.removal_key_commitment
                            .map_or(Value::Null, |value| Value::Binary(value.to_vec())),
                        keys.index_range
                            .as_ref()
                            .map_or(Value::Null, rational_range_value),
                    ])),
                ))),
            ])
        },
    );
    Value::Array(vec![
        change.role.to_value(),
        Value::Array(vec![
            Value::Array(vec![
                Value::Binary(change.party.as_bytes().to_vec()),
                change
                    .scoped_host
                    .as_ref()
                    .map_or(Value::Null, |host| Value::Binary(host.as_bytes().to_vec())),
            ]),
            change.source_role.to_value(),
            keys,
        ]),
    ])
}

fn user_shared_key_value(key: &UserSharedKey) -> Value {
    Value::Array(vec![
        Value::Unsigned(key.generation),
        key.role.to_value(),
        Value::Binary(key.verify_key.as_bytes().to_vec()),
        Value::Binary(key.hepk_fingerprint.to_vec()),
    ])
}

fn change_metadata_value(metadata: &ChangeMetadata) -> Value {
    match metadata {
        ChangeMetadata::DeviceName(value) => Value::Array(vec![
            Value::Unsigned(0),
            Value::Variant(Some((
                b"0".to_vec(),
                Box::new(Value::Binary(value.to_vec())),
            ))),
        ]),
        ChangeMetadata::Username(value) => Value::Array(vec![
            Value::Unsigned(1),
            Value::Variant(Some((
                b"0".to_vec(),
                Box::new(Value::Binary(value.to_vec())),
            ))),
        ]),
        ChangeMetadata::Eldest {
            subchain_location_commitment,
        } => Value::Array(vec![
            Value::Unsigned(2),
            Value::Variant(Some((
                b"1".to_vec(),
                Box::new(Value::Array(vec![Value::Binary(
                    subchain_location_commitment.to_vec(),
                )])),
            ))),
        ]),
        ChangeMetadata::TeamName(value) => Value::Array(vec![
            Value::Unsigned(3),
            Value::Variant(Some((
                b"0".to_vec(),
                Box::new(Value::Binary(value.to_vec())),
            ))),
        ]),
        ChangeMetadata::TeamIndexRange(range) => Value::Array(vec![
            Value::Unsigned(4),
            Value::Variant(Some((b"2".to_vec(), Box::new(rational_range_value(range))))),
        ]),
        ChangeMetadata::MemberLoadFloor(role) => Value::Array(vec![
            Value::Unsigned(5),
            Value::Variant(Some((b"3".to_vec(), Box::new(role.to_value())))),
        ]),
    }
}

fn rational_range_value(range: &RationalRange) -> Value {
    Value::Array(vec![
        rational_value(&range.low),
        rational_value(&range.high),
    ])
}

fn rational_value(value: &Rational) -> Value {
    Value::Array(vec![
        Value::Bool(value.infinity),
        if value.base.is_empty() {
            Value::Null
        } else {
            Value::Binary(value.base.clone())
        },
        if value.exponent < 0 {
            Value::Negative(value.exponent)
        } else {
            Value::Unsigned(value.exponent as u64)
        },
    ])
}

impl UserLink {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        user_link(&decode(bytes)?)
    }

    pub fn encoded(&self) -> Result<Vec<u8>> {
        Ok(self.exact.clone())
    }

    pub fn signatures(&self) -> &[Signature] {
        &self.signatures
    }

    pub fn signing_bytes(&self, signature_count: usize) -> Result<Vec<u8>> {
        if signature_count > self.signatures.len() {
            return Err(Error::Length {
                kind: "signature prefix",
                expected: self.signatures.len(),
                found: signature_count,
            });
        }
        let signatures = if signature_count == 0 {
            Value::Null
        } else {
            Value::Array(
                self.signatures[..signature_count]
                    .iter()
                    .map(Signature::to_value)
                    .collect(),
            )
        };
        Ok(encode(&Value::Array(vec![
            Value::Binary(self.inner.clone()),
            signatures,
        ]))?)
    }

    /// Decodes the security-critical fields of an eldest group-change link.
    pub fn decode_eldest(&self) -> Result<UserEldest> {
        let inner = decode(&self.inner)?;
        let inner_fields = array(&inner, 2)?;
        expect_unsigned(&inner_fields[0], "link inner version", 1)?;
        let group = array(variant(&inner_fields[1], "0")?, 8)?;
        let hiding = array(&group[0], 2)?;
        let chainer = array(&hiding[0], 4)?;
        let root = array(&chainer[2], 2)?;
        let fq_user = array(&group[1], 2)?;
        let signer = array(&group[2], 2)?;
        let changes = list_values(&group[3])?;
        let shared_keys = list_values(&group[5])?;
        if changes.len() != 1 || shared_keys.len() != 1 {
            return Err(Error::FieldCount {
                expected: 1,
                found: changes.len().max(shared_keys.len()),
            });
        }
        let member_role = array(changes[0], 2)?;
        let owner_role = role(&member_role[0])?;
        if owner_role != Role::OWNER {
            return Err(Error::UnknownEnum {
                kind: "eldest member role",
                value: owner_role.protocol_value(),
            });
        }
        let member = array(&member_role[1], 3)?;
        let member_fqe = array(&member[0], 2)?;
        let member_keys = array(&member[2], 2)?;
        expect_unsigned(&member_keys[0], "member key version", 1)?;
        let member_key_v1 = array(variant(&member_keys[1], "1")?, 2)?;
        let shared = array(shared_keys[0], 4)?;
        let shared_role = role(&shared[1])?;
        if shared_role != Role::OWNER {
            return Err(Error::UnknownEnum {
                kind: "eldest shared-key role",
                value: shared_role.protocol_value(),
            });
        }
        Ok(UserEldest {
            seqno: unsigned(&chainer[0])?,
            previous: option(&chainer[1], |v| fixed_blob(v, "previous link hash"))?,
            root: TreeRoot {
                epoch: unsigned(&root[0])?,
                hash: fixed_blob(&root[1], "Merkle root hash")?,
            },
            time: unsigned(&chainer[3])?,
            next_tree_location: fixed_blob(&hiding[1], "next tree location")?,
            uid: entity(&fq_user[0])?.require_type(ENTITY_USER)?,
            host: entity(&fq_user[1])?.require_type(ENTITY_HOST)?,
            signer: device_entity(&signer[0])?,
            member: device_entity(&member_fqe[0])?,
            member_verify_key: device_entity(&member_fqe[0])?,
            member_role: owner_role,
            member_source_role: role(&member[1])?,
            member_scoped_host: option(&member_fqe[1], |value| {
                entity(value)?.require_type(ENTITY_HOST)
            })?,
            member_hepk_fingerprint: fixed_blob(&member_key_v1[0], "HEPK fingerprint")?,
            member_subkey: option(&member_key_v1[1], |value| entity(value)?.require_type(13))?,
            puk_generation: unsigned(&shared[0])?,
            puk_verify_key: entity(&shared[2])?.require_type(ENTITY_PUK_VERIFY)?,
            puk_hepk_fingerprint: fixed_blob(&shared[3], "PUK HEPK fingerprint")?,
            metadata: list(&group[6], change_metadata)?,
        })
    }

    pub fn decode_group_change(&self) -> Result<UserGroupChange> {
        let inner = decode(&self.inner)?;
        let inner_fields = array(&inner, 2)?;
        expect_unsigned(&inner_fields[0], "link inner version", 1)?;
        let group = array(variant(&inner_fields[1], "0")?, 8)?;
        let hiding = array(&group[0], 2)?;
        let chainer = array(&hiding[0], 4)?;
        let root = array(&chainer[2], 2)?;
        let fq_user = array(&group[1], 2)?;
        let signer = array(&group[2], 2)?;
        if signer[1] != Value::Null {
            return Err(type_error("empty user-chain key owner", &signer[1]));
        }
        Ok(UserGroupChange {
            seqno: unsigned(&chainer[0])?,
            previous: option(&chainer[1], |value| fixed_blob(value, "previous link hash"))?,
            root: TreeRoot {
                epoch: unsigned(&root[0])?,
                hash: fixed_blob(&root[1], "Merkle root hash")?,
            },
            time: unsigned(&chainer[3])?,
            next_location_commitment: fixed_blob(&hiding[1], "next tree location commitment")?,
            uid: entity(&fq_user[0])?.require_type(ENTITY_USER)?,
            host: entity(&fq_user[1])?.require_type(ENTITY_HOST)?,
            signer: device_entity(&signer[0])?,
            changes: list(&group[3], user_member_change)?,
            shared_keys: list(&group[5], user_shared_key)?,
            metadata: list(&group[6], change_metadata)?,
        })
    }

    pub fn decode_team_group_change(&self) -> Result<TeamGroupChange> {
        let inner = decode(&self.inner)?;
        let inner_fields = array(&inner, 2)?;
        expect_unsigned(&inner_fields[0], "link inner version", 1)?;
        let group = array(variant(&inner_fields[1], "0")?, 8)?;
        let hiding = array(&group[0], 2)?;
        let chainer = array(&hiding[0], 4)?;
        let root = array(&chainer[2], 2)?;
        let fq_team = array(&group[1], 2)?;
        let team = entity(&fq_team[0])?;
        if !matches!(team.entity_type(), ENTITY_NAMED_TEAM | ENTITY_AD_HOC_TEAM) {
            return Err(Error::WrongEntityType {
                expected: ENTITY_NAMED_TEAM,
                found: team.entity_type(),
            });
        }
        let signer = array(&group[2], 2)?;
        let owner = option(&signer[1], key_owner)?
            .ok_or_else(|| type_error("team key owner", &signer[1]))?;
        Ok(TeamGroupChange {
            seqno: unsigned(&chainer[0])?,
            previous: option(&chainer[1], |value| fixed_blob(value, "previous link hash"))?,
            root: TreeRoot {
                epoch: unsigned(&root[0])?,
                hash: fixed_blob(&root[1], "Merkle root hash")?,
            },
            time: unsigned(&chainer[3])?,
            next_location_commitment: fixed_blob(&hiding[1], "next tree location commitment")?,
            team,
            host: entity(&fq_team[1])?.require_type(ENTITY_HOST)?,
            signer: entity(&signer[0])?,
            signer_owner: owner,
            changes: list(&group[3], team_member_change)?,
            shared_keys: list(&group[5], team_shared_key)?,
            metadata: list(&group[6], change_metadata)?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamKeyOwner {
    pub party: EntityId,
    pub source_role: Role,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamMemberKeys {
    pub verify_key: EntityId,
    pub hepk_fingerprint: [u8; 32],
    pub generation: u64,
    pub removal_key_commitment: Option<[u8; 32]>,
    pub index_range: Option<RationalRange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamMemberChange {
    pub role: Role,
    pub party: EntityId,
    pub scoped_host: Option<EntityId>,
    pub source_role: Role,
    pub keys: Option<TeamMemberKeys>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamGroupChange {
    pub seqno: u64,
    pub previous: Option<[u8; 32]>,
    pub root: TreeRoot,
    pub time: u64,
    pub next_location_commitment: [u8; 32],
    pub team: EntityId,
    pub host: EntityId,
    pub signer: EntityId,
    pub signer_owner: TeamKeyOwner,
    pub changes: Vec<TeamMemberChange>,
    pub shared_keys: Vec<UserSharedKey>,
    pub metadata: Vec<ChangeMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserGroupChange {
    pub seqno: u64,
    pub previous: Option<[u8; 32]>,
    pub root: TreeRoot,
    pub time: u64,
    pub next_location_commitment: [u8; 32],
    pub uid: EntityId,
    pub host: EntityId,
    pub signer: EntityId,
    pub changes: Vec<UserMemberChange>,
    pub shared_keys: Vec<UserSharedKey>,
    pub metadata: Vec<ChangeMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChangeMetadata {
    DeviceName([u8; 32]),
    Username([u8; 32]),
    Eldest {
        subchain_location_commitment: [u8; 32],
    },
    TeamName([u8; 32]),
    TeamIndexRange(RationalRange),
    MemberLoadFloor(Role),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RationalRange {
    pub low: Rational,
    pub high: Rational,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rational {
    pub infinity: bool,
    pub base: Vec<u8>,
    pub exponent: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserMemberChange {
    pub role: Role,
    pub entity: EntityId,
    pub scoped_host: Option<EntityId>,
    pub source_role: Role,
    pub keys: UserMemberKeys,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserMemberKeys {
    None,
    User {
        hepk_fingerprint: [u8; 32],
        subkey: Option<EntityId>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserSharedKey {
    pub generation: u64,
    pub role: Role,
    pub verify_key: EntityId,
    pub hepk_fingerprint: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserEldest {
    pub seqno: u64,
    pub previous: Option<[u8; 32]>,
    pub root: TreeRoot,
    pub time: u64,
    pub next_tree_location: [u8; 32],
    pub uid: EntityId,
    pub host: EntityId,
    pub signer: EntityId,
    pub member: EntityId,
    pub member_verify_key: EntityId,
    pub member_role: Role,
    pub member_source_role: Role,
    pub member_scoped_host: Option<EntityId>,
    pub member_hepk_fingerprint: [u8; 32],
    pub member_subkey: Option<EntityId>,
    pub puk_generation: u64,
    pub puk_verify_key: EntityId,
    pub puk_hepk_fingerprint: [u8; 32],
    pub metadata: Vec<ChangeMetadata>,
}

pub(crate) fn user_link(value: &Value) -> Result<UserLink> {
    let fields = array(value, 2)?;
    expect_unsigned(&fields[0], "link outer version", 1)?;
    let v1 = array(variant(&fields[1], "1")?, 2)?;
    Ok(UserLink {
        inner: binary(&v1[0])?.to_vec(),
        signatures: list(&v1[1], signature)?,
        exact: encode(value)?,
    })
}

pub(crate) fn role(value: &Value) -> Result<Role> {
    let fields = array(value, 2)?;
    let role_type = unsigned(&fields[0])?;
    match role_type {
        0 | 2 | 3 => {
            if fields[1] != Value::Variant(None) {
                return Err(type_error("empty role variant", &fields[1]));
            }
            Ok(match role_type {
                0 => Role::NONE,
                2 => Role::ADMIN,
                3 => Role::OWNER,
                _ => unreachable!(),
            })
        }
        1 => {
            let visibility = integer(variant(&fields[1], "0")?)?;
            let visibility = i16::try_from(visibility).map_err(|_| Error::IntegerRange("i16"))?;
            Ok(Role::member(visibility))
        }
        value => Err(Error::UnknownEnum {
            kind: "role type",
            value,
        }),
    }
}

fn change_metadata(value: &Value) -> Result<ChangeMetadata> {
    let fields = array(value, 2)?;
    let change_type = unsigned(&fields[0])?;
    match change_type {
        0 => Ok(ChangeMetadata::DeviceName(fixed_blob(
            variant(&fields[1], "0")?,
            "device-name commitment",
        )?)),
        1 => Ok(ChangeMetadata::Username(fixed_blob(
            variant(&fields[1], "0")?,
            "username commitment",
        )?)),
        2 => {
            let eldest = array(variant(&fields[1], "1")?, 1)?;
            Ok(ChangeMetadata::Eldest {
                subchain_location_commitment: fixed_blob(
                    &eldest[0],
                    "subchain location commitment",
                )?,
            })
        }
        3 => Ok(ChangeMetadata::TeamName(fixed_blob(
            variant(&fields[1], "0")?,
            "team-name commitment",
        )?)),
        4 => Ok(ChangeMetadata::TeamIndexRange(rational_range(variant(
            &fields[1], "2",
        )?)?)),
        5 => Ok(ChangeMetadata::MemberLoadFloor(role(variant(
            &fields[1], "3",
        )?)?)),
        value => Err(Error::UnknownEnum {
            kind: "change metadata type",
            value,
        }),
    }
}

fn rational_range(value: &Value) -> Result<RationalRange> {
    let fields = array(value, 2)?;
    Ok(RationalRange {
        low: rational(&fields[0])?,
        high: rational(&fields[1])?,
    })
}

fn rational(value: &Value) -> Result<Rational> {
    let fields = array(value, 3)?;
    let Value::Bool(infinity) = fields[0] else {
        return Err(type_error("boolean", &fields[0]));
    };
    Ok(Rational {
        infinity,
        base: match &fields[1] {
            Value::Null => Vec::new(),
            value => binary(value)?.to_vec(),
        },
        exponent: integer(&fields[2])?,
    })
}

fn user_member_change(value: &Value) -> Result<UserMemberChange> {
    let fields = array(value, 2)?;
    let member = array(&fields[1], 3)?;
    let scoped = array(&member[0], 2)?;
    let key_fields = array(&member[2], 2)?;
    let key_type = unsigned(&key_fields[0])?;
    let keys = match key_type {
        0 => {
            if key_fields[1] != Value::Variant(None) {
                return Err(type_error("empty member-key variant", &key_fields[1]));
            }
            UserMemberKeys::None
        }
        1 => {
            let user = array(variant(&key_fields[1], "1")?, 2)?;
            UserMemberKeys::User {
                hepk_fingerprint: fixed_blob(&user[0], "device HEPK fingerprint")?,
                subkey: option(&user[1], entity)?,
            }
        }
        value => {
            return Err(Error::UnknownEnum {
                kind: "user member keys",
                value,
            })
        }
    };
    Ok(UserMemberChange {
        role: role(&fields[0])?,
        entity: device_entity(&scoped[0])?,
        scoped_host: option(&scoped[1], |value| entity(value)?.require_type(ENTITY_HOST))?,
        source_role: role(&member[1])?,
        keys,
    })
}

fn key_owner(value: &Value) -> Result<TeamKeyOwner> {
    let fields = array(value, 2)?;
    Ok(TeamKeyOwner {
        party: entity(&fields[0])?,
        source_role: role(&fields[1])?,
    })
}

fn team_member_change(value: &Value) -> Result<TeamMemberChange> {
    let fields = array(value, 2)?;
    let member = array(&fields[1], 3)?;
    let scoped = array(&member[0], 2)?;
    let key_fields = array(&member[2], 2)?;
    let keys = match unsigned(&key_fields[0])? {
        0 => {
            if key_fields[1] != Value::Variant(None) {
                return Err(type_error("empty member-key variant", &key_fields[1]));
            }
            None
        }
        2 => {
            let team = array(variant(&key_fields[1], "2")?, 5)?;
            let verify_key = entity(&team[0])?;
            if !matches!(
                verify_key.entity_type(),
                ENTITY_PUK_VERIFY | ENTITY_PTK_VERIFY
            ) {
                return Err(Error::WrongEntityType {
                    expected: ENTITY_PUK_VERIFY,
                    found: verify_key.entity_type(),
                });
            }
            Some(TeamMemberKeys {
                verify_key,
                hepk_fingerprint: fixed_blob(&team[1], "member HEPK fingerprint")?,
                generation: unsigned(&team[2])?,
                removal_key_commitment: option(&team[3], |value| {
                    fixed_blob(value, "removal-key commitment")
                })?,
                index_range: option(&team[4], rational_range)?,
            })
        }
        value => {
            return Err(Error::UnknownEnum {
                kind: "team member keys",
                value,
            });
        }
    };
    Ok(TeamMemberChange {
        role: role(&fields[0])?,
        party: entity(&scoped[0])?,
        scoped_host: option(&scoped[1], |value| entity(value)?.require_type(ENTITY_HOST))?,
        source_role: role(&member[1])?,
        keys,
    })
}

fn user_shared_key(value: &Value) -> Result<UserSharedKey> {
    let fields = array(value, 4)?;
    Ok(UserSharedKey {
        generation: unsigned(&fields[0])?,
        role: role(&fields[1])?,
        verify_key: entity(&fields[2])?.require_type(ENTITY_PUK_VERIFY)?,
        hepk_fingerprint: fixed_blob(&fields[3], "PUK HEPK fingerprint")?,
    })
}

fn team_shared_key(value: &Value) -> Result<UserSharedKey> {
    let fields = array(value, 4)?;
    Ok(UserSharedKey {
        generation: unsigned(&fields[0])?,
        role: role(&fields[1])?,
        verify_key: entity(&fields[2])?.require_type(ENTITY_PTK_VERIFY)?,
        hepk_fingerprint: fixed_blob(&fields[3], "PTK HEPK fingerprint")?,
    })
}

pub(crate) fn array_any(value: &Value) -> Result<&[Value]> {
    match value {
        Value::Null => Ok(&[]),
        Value::Array(values) => Ok(values),
        _ => Err(type_error("array or null", value)),
    }
}

pub(crate) fn list_values(value: &Value) -> Result<Vec<&Value>> {
    Ok(array_any(value)?.iter().collect())
}
