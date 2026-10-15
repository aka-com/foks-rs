//! Selector tables over the typed foks-proto schema decoders and the strict
//! server argument decoders. The first input byte selects one table entry and
//! the remaining bytes are its input. The fuzz targets, the seed generator and
//! the seed tests share these tables, so seed selectors follow table order.
#![allow(dead_code)]
use foks_proto::*;
use foks_rpc::arguments;

pub enum Outcome {
    Rejected,
    Accepted,
    /// The decoded value re-encoded to these bytes, and decoding them again
    /// reproduced the same encoding.
    Encoded(Vec<u8>),
}

pub type Decoder = (&'static str, fn(&[u8]) -> Outcome);

/// Splits a fuzz input into its selected decoder and the decoder input.
pub fn select<'a>(
    table: &'static [Decoder],
    input: &'a [u8],
) -> Option<(&'static Decoder, &'a [u8])> {
    let (&selector, rest) = input.split_first()?;
    Some((&table[usize::from(selector) % table.len()], rest))
}

/// The selector byte that reaches `table[index]`.
pub fn selector(table: &[Decoder], index: usize) -> u8 {
    assert!(
        table.len() <= 256,
        "a selector byte addresses at most 256 decoders"
    );
    u8::try_from(index).unwrap()
}

/// Normalizes the three encoder return shapes: fallible, infallible, and the
/// exact bytes retained from decoding.
trait Encoded {
    fn bytes(self) -> Option<Vec<u8>>;
}
impl<E> Encoded for Result<Vec<u8>, E> {
    fn bytes(self) -> Option<Vec<u8>> {
        self.ok()
    }
}
impl Encoded for Vec<u8> {
    fn bytes(self) -> Option<Vec<u8>> {
        Some(self)
    }
}
impl Encoded for &[u8] {
    fn bytes(self) -> Option<Vec<u8>> {
        Some(self.to_vec())
    }
}

fn accepted(ok: bool) -> Outcome {
    if ok {
        Outcome::Accepted
    } else {
        Outcome::Rejected
    }
}

/// Decode only, for types without an encoder.
macro_rules! decode {
    ($ty:ty) => {
        decode!($ty, decode)
    };
    ($ty:ty, $decode:ident) => {
        (stringify!($ty), |bytes: &[u8]| {
            accepted(<$ty>::$decode(bytes).is_ok())
        })
    };
}

/// Decode, re-encode, and require the encoding to be a fixed point. An
/// encoder may refuse a value its decoder accepted; such inputs only count as
/// accepted.
macro_rules! canonical {
    ($ty:ty) => {
        canonical!($ty, decode, encoded)
    };
    ($ty:ty, $decode:ident, $encode:ident) => {
        (stringify!($ty), |bytes: &[u8]| {
            let Ok(value) = <$ty>::$decode(bytes) else {
                return Outcome::Rejected;
            };
            let Some(encoded) = value.$encode().bytes() else {
                return Outcome::Accepted;
            };
            let again = <$ty>::$decode(&encoded)
                .unwrap_or_else(|_| panic!("{} rejects its own encoding", stringify!($ty)));
            assert!(
                again.$encode().bytes().as_deref() == Some(&encoded[..]),
                "{} encoding is not a fixed point",
                stringify!($ty)
            );
            Outcome::Encoded(encoded)
        })
    };
}

/// Types that retain the exact decoded bytes must hand back their input.
macro_rules! exact {
    ($ty:ty) => {
        (stringify!($ty), |bytes: &[u8]| match <$ty>::decode(bytes) {
            Ok(value) => {
                assert!(
                    value.encoded() == bytes,
                    "{} does not retain its input",
                    stringify!($ty)
                );
                Outcome::Encoded(bytes.to_vec())
            }
            Err(_) => Outcome::Rejected,
        })
    };
}

macro_rules! realtime {
    ($ty:ty) => {
        (stringify!($ty), |bytes: &[u8]| {
            let Ok(value) = <$ty as RealtimeWire>::decode(bytes) else {
                return Outcome::Rejected;
            };
            let Ok(encoded) = RealtimeWire::encoded(&value) else {
                return Outcome::Accepted;
            };
            let again = <$ty as RealtimeWire>::decode(&encoded)
                .unwrap_or_else(|_| panic!("{} rejects its own encoding", stringify!($ty)));
            assert!(
                RealtimeWire::encoded(&again).ok().as_ref() == Some(&encoded),
                "{} encoding is not a fixed point",
                stringify!($ty)
            );
            Outcome::Encoded(encoded)
        })
    };
}

fn host() -> EntityId {
    let mut id = [0x5a; 33];
    id[0] = ENTITY_HOST;
    EntityId::from_bytes(id.to_vec()).unwrap()
}

/// Every public foks-proto decoder that takes network or stored bytes.
pub static PROTO: &[Decoder] = &[
    canonical!(ChangeUsernameArgument),
    canonical!(ClientVersionExt),
    canonical!(ServerClientVersionInfo),
    canonical!(DeviceNagInfo),
    decode!(Signature),
    canonical!(SignedBlob),
    canonical!(PermissionToken),
    canonical!(FqParty),
    canonical!(FqTeam),
    canonical!(TeamRemoteMemberViewTokenBoxPayload),
    canonical!(TeamRemoteMemberViewTokenInner),
    canonical!(TeamRemoteMemberViewToken),
    canonical!(TeamRemoteViewTokenSet),
    canonical!(RemoteViewPermissionPayload),
    ("BeaconHint", |bytes| {
        accepted(BeaconHint::decode_address(host(), bytes).is_ok())
    }),
    canonical!(ProbeResponse),
    ("decode_hostchain", |bytes| match decode_hostchain(bytes) {
        Ok(links) => {
            for link in &links {
                let _ = link.encoded();
                if let Ok(change) = link.decode_change() {
                    let _ = change.encoded();
                }
            }
            Outcome::Accepted
        }
        Err(_) => Outcome::Rejected,
    }),
    canonical!(TreeRoot),
    canonical!(PublicZone),
    canonical!(MerkleRoot),
    ("decode_merkle_back_pointers", |bytes| {
        accepted(decode_merkle_back_pointers(bytes).is_ok())
    }),
    canonical!(HistoricalMerkleRoots),
    canonical!(IdentityClaim),
    canonical!(IdentityChallenge),
    canonical!(IdentityProof),
    canonical!(IdentityStatus),
    canonical!(TeamInvite),
    canonical!(TeamCertificatePayload),
    canonical!(TeamCertificate),
    canonical!(LocalViewPermissionPayload),
    canonical!(TeamCertificateAndMetadata),
    canonical!(RemoteJoinRequest),
    canonical!(RemoteJoinPayload),
    canonical!(TeamRsvp),
    canonical!(InboxPagination),
    ("decode_team_inbox", |bytes| {
        accepted(decode_team_inbox(bytes).is_ok())
    }),
    canonical!(LocalInviteAcceptance),
    canonical!(KexWrapperMessage),
    canonical!(KexSendArgument),
    canonical!(KexReceiveArgument),
    canonical!(KexCleartext),
    canonical!(HybridBox),
    canonical!(TeamRemovalKeyBox),
    canonical!(SharedKeyBoxSet),
    canonical!(SeedChainBox),
    canonical!(PukParcel),
    canonical!(SecretBox),
    decode!(SubkeySeed),
    decode!(SharedKeySeed),
    decode!(KvPathVersionVector),
    exact!(KvRoot),
    decode!(KvDirectory),
    exact!(KvDirectoryPair),
    exact!(KvDirent),
    exact!(KvGetResponse),
    exact!(KvListResponse),
    decode!(KvSmallFileBox),
    decode!(KvLargeFileMetadata),
    decode!(KvUploadChunk),
    canonical!(KvNode),
    decode!(KvUsage),
    decode!(KvEncryptedChunk),
    ("KvDirentName", |bytes| match foks_snowpack::decode(bytes) {
        Ok(value) => accepted(KvDirentName::decode_value(value).is_ok()),
        Err(_) => Outcome::Rejected,
    }),
    (
        "KvSmallFilePlaintext",
        |bytes| match foks_snowpack::decode(bytes) {
            Ok(value) => accepted(KvSmallFilePlaintext::decode_value(value).is_ok()),
            Err(_) => Outcome::Rejected,
        },
    ),
    canonical!(PpePassphraseBox),
    canonical!(PpePukBox),
    canonical!(PpeParcel),
    canonical!(PassphraseUpdateArgument, decode_set, encoded_set),
    ("PassphraseUpdateArgument::decode_change", |bytes| {
        let Ok(value) = PassphraseUpdateArgument::decode_change(bytes, [7; 16]) else {
            return Outcome::Rejected;
        };
        let Ok(encoded) = value.encoded_change() else {
            return Outcome::Accepted;
        };
        let again = PassphraseUpdateArgument::decode_change(&encoded, [7; 16])
            .expect("PassphraseUpdateArgument rejects its own encoding");
        assert!(again.encoded_change().ok().as_ref() == Some(&encoded));
        Outcome::Encoded(encoded)
    }),
    canonical!(PassphraseLoginResult),
    canonical!(SkmwkList),
    canonical!(PpePassphraseBoxPayload),
    canonical!(PpePukBoxPayload),
    ("decode_salt", |bytes| accepted(decode_salt(bytes).is_ok())),
    ("decode_generation", |bytes| {
        accepted(decode_generation(bytes).is_ok())
    }),
    ("decode_stretch_version", |bytes| {
        accepted(decode_stretch_version(bytes).is_ok())
    }),
    canonical!(Role),
    canonical!(SsoConfig),
    canonical!(OAuth2Binding),
    canonical!(OAuth2TokenSet),
    canonical!(OAuth2PollResult),
    canonical!(OAuth2IdTokenBindingPayload),
    canonical!(OAuth2IdTokenBinding),
    canonical!(RegSsoArgs),
    canonical!(InitOAuth2SessionArgument),
    canonical!(PollOAuth2SessionArgument),
    canonical!(SsoLoginArgument),
    canonical!(YubiSlotAndPqKeyId),
    canonical!(YubiManagementKeyBoxPayload),
    canonical!(YubiEncryptedManagementKey),
    canonical!(PostGenericLinkArgument),
    ("UserLink", |bytes| {
        let Ok(link) = UserLink::decode(bytes) else {
            return Outcome::Rejected;
        };
        // The inner payload decoders run on signed bytes the outer decoder
        // does not interpret.
        let _ = link.decode_approved_membership();
        let _ = link.decode_eldest();
        let _ = link.decode_group_change();
        let _ = link.decode_team_group_change();
        let _ = link.decode_generic();
        let Ok(encoded) = link.encoded() else {
            return Outcome::Accepted;
        };
        let again = UserLink::decode(&encoded).expect("UserLink rejects its own encoding");
        assert!(again.encoded().ok().as_ref() == Some(&encoded));
        Outcome::Encoded(encoded)
    }),
    canonical!(RationalRange),
    decode!(GenericChain),
    ("decode_local_team_list", |bytes| {
        accepted(decode_local_team_list(bytes).is_ok())
    }),
    canonical!(Hepk),
    canonical!(DeviceLabelNameAndCommitmentKey),
    canonical!(MerklePathCompressed),
    canonical!(MerkleLookupResponse),
    canonical!(MerkleMultiLookupResponse),
    canonical!(MerkleExistsResponse),
    decode!(UserMerklePaths),
    canonical!(TeamBearerTokenChallenge, decode_payload, encoded_payload),
    decode!(TeamRemovalKeyPayload),
    canonical!(TeamRemovalBoxData),
    canonical!(UsernameReservation),
    canonical!(InviteCode),
    ("decode_base62_strict", |bytes| {
        match std::str::from_utf8(bytes) {
            Ok(text) => accepted(decode_base62_strict(text).is_ok()),
            Err(_) => Outcome::Rejected,
        }
    }),
    decode!(DecodedSignupArgument),
    decode!(DecodedProvisionDeviceArgument),
    decode!(DecodedAdHocTeamCreateArgument),
    decode!(DecodedNamedTeamCreateArgument),
    decode!(DecodedTeamEditArgument),
    canonical!(TeamRemovalProof),
    canonical!(TeamRemovalAndKeyBox),
    canonical!(TeamRemovalAndCommitment),
    canonical!(TeamConfig),
    canonical!(TeamEditResult),
    canonical!(RegServerConfig),
    canonical!(HostConfig),
    decode!(DecodedRevokeDeviceArgument),
    canonical!(RegistrationChallenge),
    canonical!(LookupUserResult),
    decode!(TeamChain),
    canonical!(TeamViewRequest),
    canonical!(TeamViewChallenge),
    canonical!(ActivatedTeamView),
    decode!(UserChain),
    realtime!(RtChatCapabilitiesArgument),
    realtime!(RtChatCapabilities),
    realtime!(RtChatContext),
    realtime!(RtChatPurpose),
    realtime!(RtCiphertext),
    realtime!(RtMessageBody),
    realtime!(RtMessageWrapper),
    realtime!(RtText),
    realtime!(RtBox),
    realtime!(RtMessageBox),
    realtime!(RtRolePair),
    realtime!(RtMessageMetadata),
    realtime!(RtMessageNoncer),
    realtime!(RtLastMessage),
    realtime!(RtChannelMetadata),
    realtime!(RtChannelSet),
    realtime!(RtInboxKey),
    realtime!(RtChangedThreads),
    realtime!(RtReadThrough),
    realtime!(RtPollInbox),
    realtime!(RtInboxPollResult),
    realtime!(RtInboxChannel),
    realtime!(RtInboxDelta),
    realtime!(RtSend),
    realtime!(ChatSendReceipt),
    realtime!(RtThreadRange),
    realtime!(RtThreadQuery),
    realtime!(RtMessage),
    realtime!(RtMessageList),
    realtime!(RtThreadPage),
];

macro_rules! argument {
    ($decode:ident) => {
        (stringify!($decode), |bytes: &[u8]| {
            accepted(arguments::$decode(bytes).is_ok())
        })
    };
}

macro_rules! realtime_request {
    ($position:ident) => {
        (stringify!($position), |bytes: &[u8]| {
            let Ok(request) =
                foks_rpc::RealtimeRequest::decode_argument(foks_rpc::$position, bytes)
            else {
                return Outcome::Rejected;
            };
            assert_eq!(request.position(), foks_rpc::$position);
            let Ok(encoded) = request.argument() else {
                return Outcome::Accepted;
            };
            let again = foks_rpc::RealtimeRequest::decode_argument(request.position(), &encoded)
                .expect("realtime request rejects its own encoding");
            assert!(
                again == request,
                "realtime request encoding is not a fixed point"
            );
            Outcome::Encoded(encoded)
        })
    };
}

/// Every server-side argument decoder in `foks_rpc::arguments`, then the
/// realtime request decoder at each supported method position.
pub static SERVER: &[Decoder] = &[
    argument!(decode_client_version_info),
    argument!(decode_clear_device_nag),
    argument!(decode_post_generic_link),
    argument!(decode_load_generic_chain),
    argument!(decode_check_name_exists),
    argument!(decode_resolve_username),
    argument!(decode_probe_key_exists),
    argument!(decode_check_invite_code),
    argument!(decode_log_send_init_file),
    argument!(decode_log_send_upload_block),
    argument!(decode_merkle_lookup),
    argument!(decode_merkle_multi_lookup),
    argument!(decode_merkle_check_key),
    argument!(decode_merkle_optional_host),
    argument!(decode_login_challenge),
    argument!(decode_passphrase_login),
    argument!(decode_set_passphrase),
    ("decode_change_passphrase", |bytes| {
        accepted(arguments::decode_change_passphrase(bytes, [7; 16]).is_ok())
    }),
    argument!(decode_void),
    argument!(decode_uid_lookup_challenge),
    argument!(decode_lookup_uid_by_device),
    argument!(decode_check_team_bearer_token),
    argument!(decode_post_team_membership_link),
    argument!(decode_team_name_reservation_request),
    argument!(decode_named_team_create),
    argument!(decode_adhoc_team_create),
    argument!(decode_team_edit),
    argument!(decode_make_team_bearer_token),
    argument!(decode_activate_team_bearer_token),
    argument!(decode_load_removal_key_box),
    argument!(decode_check_team_view),
    argument!(decode_load_team_membership_chain),
    argument!(decode_load_removal_for_member),
    argument!(decode_team_view_request),
    argument!(decode_activate_team_view),
    argument!(decode_load_team_chain),
    argument!(decode_provision_device),
    argument!(decode_revoke_device),
    argument!(decode_join_waitlist),
    argument!(decode_subkey_box_challenge),
    argument!(decode_load_subkey_box),
    argument!(decode_put_yubi_management_key),
    argument!(decode_get_yubi_management_key),
    argument!(decode_load_user_chain_argument),
    argument!(decode_grant_remote_view_permission_for_user),
    argument!(decode_grant_remote_view_permission_for_team),
    argument!(decode_load_team_remote_view_tokens),
    realtime_request!(RT_CHAT_CAPABILITIES_METHOD_POSITION),
    realtime_request!(RT_NEW_CHANNEL_METHOD_POSITION),
    realtime_request!(RT_LIST_CHANNELS_METHOD_POSITION),
    realtime_request!(RT_SEND_METHOD_POSITION),
    realtime_request!(RT_GET_THREAD_METHOD_POSITION),
    realtime_request!(RT_GET_INBOX_VERSION_METHOD_POSITION),
    realtime_request!(RT_GET_CHANGED_THREADS_METHOD_POSITION),
    realtime_request!(RT_READ_THROUGH_METHOD_POSITION),
    realtime_request!(RT_POLL_INBOX_METHOD_POSITION),
    realtime_request!(RT_SELECT_VHOST_METHOD_POSITION),
    realtime_request!(RT_RECENTS_METHOD_POSITION),
];

/// The corpus file name for a fixture seeded through a named decoder.
pub fn seed_name(fixture: &str, decoder: &str) -> String {
    let decoder: String = decoder
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{fixture}.{decoder}")
}
