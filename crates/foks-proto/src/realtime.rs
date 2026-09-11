//! Pinned v0.1.9 realtime wire values. Authorization belongs to the service/client.
use crate::{
    array, binary, boolean, entity, fixed_blob, integer, list, option, unsigned, variant, Error,
    FqParty, Result, Role, RoleAndGeneration, SecretBox, Value,
};
use foks_snowpack::{decode_sensitive, encode_ref, ValueRef};
use zeroize::{Zeroize, Zeroizing};

pub const RT_KEY_DERIVATION_TYPE_ID: u64 = 0xee6956bd3980334a;
pub const RT_MSG_NONCER_TYPE_ID: u64 = 0xd45941000217cf8a;
pub const RT_MSG_BODY_TYPE_ID: u64 = 0xc830111a77ab24f6;
pub const RT_CHANNEL_NAME_TYPE_ID: u64 = 0xbe4f7ec6ba0b1393;
pub const RT_CHANNEL_DESC_TYPE_ID: u64 = 0xd14f88f5ae7aaecb;
pub const RT_MAX_WIRE_BYTES: usize = 16 * 1024 * 1024;
pub const RT_MAX_COLLECTION: usize = 4096;
pub const RT_MAX_REQUEST_BYTES: usize = 1024 * 1024;
// Reserve 1 KiB for the largest supported send metadata (including attribution,
// u64 sequence/time fields, role/generation), wrapper, and RPC envelope.
pub const RT_SEND_OVERHEAD_BYTES: usize = 1024;
pub const RT_MAX_BODY_BYTES: usize = RT_MAX_REQUEST_BYTES - RT_SEND_OVERHEAD_BYTES;
// Body union/string header plus the NaCl authentication tag fit within 32 bytes.
pub const RT_MAX_CIPHERTEXT_BYTES: usize = RT_MAX_BODY_BYTES + 32;

/// Canonical schema conversion for realtime values (not an authorization witness).
pub trait RealtimeWire: Sized {
    fn to_value(&self) -> Value;
    fn from_value(value: &Value) -> Result<Self>;
    /// Validate fields before constructing the encoded tree. Composite and
    /// sensitive types override this scalar fallback.
    fn validate(&self) -> Result<()> {
        Self::from_value(&self.to_value()).map(|_| ())
    }
    fn to_value_ref(&self) -> ValueRef<'_> {
        ValueRef::Owned(self.to_value())
    }
    fn encoded(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut bytes = Zeroizing::new(encode_ref(&self.to_value_ref())?);
        if bytes.len() > RT_MAX_WIRE_BYTES {
            return Err(Error::IntegerRange("realtime frame limit"));
        }
        Ok(std::mem::take(&mut *bytes))
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > RT_MAX_WIRE_BYTES {
            return Err(Error::IntegerRange("realtime frame limit"));
        }
        Self::from_value(&*decode_sensitive(bytes)?)
    }
}
impl RealtimeWire for u64 {
    fn to_value(&self) -> Value {
        Value::Unsigned(*self)
    }
    fn from_value(v: &Value) -> Result<Self> {
        unsigned(v)
    }
}
impl RealtimeWire for bool {
    fn to_value(&self) -> Value {
        Value::Bool(*self)
    }
    fn from_value(v: &Value) -> Result<Self> {
        boolean(v)
    }
}
impl<T: RealtimeWire> RealtimeWire for Option<T> {
    fn validate(&self) -> Result<()> {
        self.as_ref().map_or(Ok(()), RealtimeWire::validate)
    }
    fn to_value_ref(&self) -> ValueRef<'_> {
        self.as_ref()
            .map_or(ValueRef::Null, RealtimeWire::to_value_ref)
    }
    fn to_value(&self) -> Value {
        self.as_ref().map_or(Value::Null, RealtimeWire::to_value)
    }
    fn from_value(v: &Value) -> Result<Self> {
        option(v, T::from_value)
    }
}
impl<T: RealtimeWire> RealtimeWire for Vec<T> {
    fn validate(&self) -> Result<()> {
        if self.len() > RT_MAX_COLLECTION {
            return Err(Error::IntegerRange("realtime collection limit"));
        }
        self.iter().try_for_each(RealtimeWire::validate)
    }
    fn to_value_ref(&self) -> ValueRef<'_> {
        if self.is_empty() {
            ValueRef::Null
        } else {
            ValueRef::Array(self.iter().map(RealtimeWire::to_value_ref).collect())
        }
    }
    fn to_value(&self) -> Value {
        if self.is_empty() {
            Value::Null
        } else {
            Value::Array(self.iter().map(RealtimeWire::to_value).collect())
        }
    }
    fn from_value(v: &Value) -> Result<Self> {
        if matches!(v, Value::Array(a) if a.len() > RT_MAX_COLLECTION) {
            return Err(Error::IntegerRange("realtime collection limit"));
        }
        list(v, T::from_value)
    }
}
impl RealtimeWire for Role {
    fn to_value(&self) -> Value {
        Role::to_value(*self)
    }
    fn from_value(v: &Value) -> Result<Self> {
        crate::identity::role(v)
    }
}
impl RealtimeWire for RoleAndGeneration {
    fn to_value(&self) -> Value {
        RoleAndGeneration::to_value(*self)
    }
    fn from_value(v: &Value) -> Result<Self> {
        let f = array(v, 2)?;
        Ok(Self {
            role: Role::from_value(&f[0])?,
            generation: unsigned(&f[1])?,
        })
    }
}
impl RealtimeWire for SecretBox {
    fn to_value(&self) -> Value {
        SecretBox::to_value(self)
    }
    fn from_value(v: &Value) -> Result<Self> {
        crate::kv::secret_box(v)
    }
}
impl RealtimeWire for FqParty {
    fn to_value(&self) -> Value {
        FqParty::to_value(self)
    }
    fn from_value(v: &Value) -> Result<Self> {
        FqParty::from_value(v)
    }
}

macro_rules! rt_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
        pub struct $name(pub [u8; 16]);
        impl RealtimeWire for $name {
            fn to_value(&self) -> Value {
                Value::Binary(self.0.to_vec())
            }
            fn from_value(v: &Value) -> Result<Self> {
                Ok(Self(fixed_blob(v, stringify!($name))?))
            }
        }
    };
}
rt_id!(RtChannelId);
rt_id!(RtMessageId);
impl RtChannelId {
    pub fn short(self) -> RtChannelIdShort {
        RtChannelIdShort(
            (u64::from_be_bytes(self.0[..8].try_into().expect("eight bytes")) & i64::MAX as u64)
                as i64,
        )
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RtChannelIdShort(i64);
impl RtChannelIdShort {
    pub fn new(value: i64) -> Result<Self> {
        if value < 0 {
            Err(Error::IntegerRange("nonnegative channel ID"))
        } else {
            Ok(Self(value))
        }
    }
    pub fn get(self) -> i64 {
        self.0
    }
}
impl RealtimeWire for RtChannelIdShort {
    fn to_value(&self) -> Value {
        Value::Unsigned(self.0 as u64)
    }
    fn from_value(v: &Value) -> Result<Self> {
        Self::new(integer(v)?)
    }
}
macro_rules! rt_enum {
    ($name:ident { $($case:ident = $n:literal),+ $(,)? }) => {
        #[derive(Clone,Copy,Debug,Eq,PartialEq)]
        #[repr(u64)]
        pub enum $name { $($case = $n),+ }
        impl RealtimeWire for $name {
            fn to_value(&self)->Value {Value::Unsigned(*self as u64)}
            fn from_value(v:&Value)->Result<Self> {match unsigned(v)? { $($n=>Ok(Self::$case),)+ value=>Err(Error::UnknownEnum{kind:stringify!($name),value}) }}
        }
    }
}
rt_enum!(RtAppId { None=0, Chat=1, Crdt=2, Notification=3 });
rt_enum!(RtMessageType { None=0, Basic=1, Edit=2, Delete=3, Reaction=4, Attachment=5, Reply=6, System=7, Join=8, Leave=9 });
rt_enum!(RtChannelTier { None=0, Bottom=1, Admin=2 });
rt_enum!(RtKeyType { ChannelName=1, ChannelDescription=2, Data=3 });

/// Context-specific entity wrappers prevent mixing a host/team/user on the wire.
macro_rules! rt_entity {
    ($name:ident, $($kind:pat_param)|+) => {
        #[derive(Clone,Debug,Eq,PartialEq)]
        pub struct $name(crate::EntityId);
        impl $name {
            pub fn new(id:crate::EntityId)->Result<Self>{if matches!(id.entity_type(),$($kind)|+) {Ok(Self(id))} else {Err(Error::EntityType(id.entity_type()))}}
            pub fn entity(&self)->&crate::EntityId {&self.0}
        }
        impl RealtimeWire for $name {
            fn to_value(&self)->Value{Value::Binary(self.0.as_bytes().to_vec())}
            fn from_value(v:&Value)->Result<Self>{Self::new(entity(v)?)}
        }
    }
}
rt_entity!(RtHostId, crate::ENTITY_HOST);
rt_entity!(
    RtTeamId,
    crate::ENTITY_NAMED_TEAM | crate::ENTITY_AD_HOC_TEAM
);
rt_entity!(RtUserId, crate::ENTITY_USER);
rt_entity!(
    RtPartyId,
    crate::ENTITY_USER | crate::ENTITY_NAMED_TEAM | crate::ENTITY_AD_HOC_TEAM
);

macro_rules! rt_struct {
    ($name:ident {$($field:ident : $ty:ty),+ $(,)?}) => {
        #[derive(Clone,Debug,Eq,PartialEq)]
        pub struct $name {$(pub $field:$ty),+}
        impl RealtimeWire for $name {
            fn validate(&self)->Result<()> { $(self.$field.validate()?;)+ Ok(()) }
            fn to_value_ref(&self)->ValueRef<'_>{ValueRef::Array(vec![$(self.$field.to_value_ref()),+])}
            fn to_value(&self)->Value{Value::Array(vec![$(RealtimeWire::to_value(&self.$field)),+])}
            fn from_value(v:&Value)->Result<Self>{
                let fields=array(v,[$(stringify!($field)),+].len())?;
                let mut fields=fields.iter();
                Ok(Self{$($field:<$ty>::from_value(fields.next().expect("validated field count"))?),+})
            }
        }
    }
}

fn arm(n: u64, tag: &str, v: Value) -> Value {
    Value::Array(vec![
        Value::Unsigned(n),
        Value::Variant(Some((tag.as_bytes().to_vec(), Box::new(v)))),
    ])
}
fn blob(v: &Value) -> Result<Vec<u8>> {
    let b = binary(v)?;
    if b.len() > RT_MAX_BODY_BYTES {
        return Err(Error::IntegerRange("realtime body limit"));
    }
    Ok(b.to_vec())
}

/// Plaintext is redacted from diagnostics and erased on drop.
#[derive(Clone, Eq, PartialEq)]
pub enum RtMessageBody {
    Basic(Vec<u8>),
    Pegged {
        kind: RtMessageType,
        body: Vec<u8>,
        reply_to: RtMessageId,
    },
}
impl std::fmt::Debug for RtMessageBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RtMessageBody([REDACTED])")
    }
}
impl Drop for RtMessageBody {
    fn drop(&mut self) {
        match self {
            Self::Basic(b) | Self::Pegged { body: b, .. } => b.zeroize(),
        }
    }
}
impl RealtimeWire for RtMessageBody {
    fn validate(&self) -> Result<()> {
        let body = match self {
            Self::Basic(body) => body,
            Self::Pegged { kind, body, .. } => {
                if !matches!(
                    kind,
                    RtMessageType::Edit | RtMessageType::Reaction | RtMessageType::Reply
                ) {
                    return Err(Error::UnknownEnum {
                        kind: "supported realtime body",
                        value: *kind as u64,
                    });
                }
                body
            }
        };
        check_body_length(body.len())
    }
    fn to_value_ref(&self) -> ValueRef<'_> {
        match self {
            Self::Basic(body) => borrowed_arm(1, b"1", ValueRef::Binary(body)),
            Self::Pegged {
                kind,
                body,
                reply_to,
            } => borrowed_arm(
                *kind as u64,
                b"2",
                ValueRef::Array(vec![ValueRef::Binary(body), ValueRef::Binary(&reply_to.0)]),
            ),
        }
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        decode_plaintext(bytes)
    }

    fn to_value(&self) -> Value {
        match self {
            Self::Basic(b) => arm(1, "1", Value::Binary(b.clone())),
            Self::Pegged {
                kind,
                body,
                reply_to,
            } => arm(
                *kind as u64,
                "2",
                Value::Array(vec![Value::Binary(body.clone()), reply_to.to_value()]),
            ),
        }
    }
    fn from_value(v: &Value) -> Result<Self> {
        let f = array(v, 2)?;
        let kind = RtMessageType::from_value(&f[0])?;
        match kind {
            RtMessageType::Basic => Ok(Self::Basic(blob(variant(&f[1], "1")?)?)),
            RtMessageType::Edit | RtMessageType::Reaction | RtMessageType::Reply => {
                let p = array(variant(&f[1], "2")?, 2)?;
                Ok(Self::Pegged {
                    kind,
                    reply_to: RtMessageId::from_value(&p[1])?,
                    body: blob(&p[0])?,
                })
            }
            _ => Err(Error::UnknownEnum {
                kind: "supported realtime body",
                value: kind as u64,
            }),
        }
    }
}
#[derive(Clone, Eq, PartialEq)]
pub struct RtText(pub String);
impl std::fmt::Debug for RtText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RtText([REDACTED])")
    }
}
impl Drop for RtText {
    fn drop(&mut self) {
        self.0.zeroize()
    }
}
impl RealtimeWire for RtText {
    fn validate(&self) -> Result<()> {
        check_body_length(self.0.len())
    }
    fn to_value_ref(&self) -> ValueRef<'_> {
        borrowed_arm(1, b"1", ValueRef::Text(self.0.as_bytes()))
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        decode_plaintext(bytes)
    }

    fn to_value(&self) -> Value {
        arm(1, "1", Value::Text(self.0.as_bytes().to_vec()))
    }
    fn from_value(v: &Value) -> Result<Self> {
        let f = array(v, 2)?;
        crate::expect_unsigned(&f[0], "realtime text version", 1)?;
        let b = crate::codec::text_bytes(variant(&f[1], "1")?)?;
        if b.len() > RT_MAX_BODY_BYTES {
            return Err(Error::IntegerRange("realtime text limit"));
        }
        Ok(Self(
            std::str::from_utf8(b).map_err(|_| Error::Utf8)?.to_owned(),
        ))
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RtCiphertext(pub Vec<u8>);
impl RealtimeWire for RtCiphertext {
    fn validate(&self) -> Result<()> {
        if self.0.len() > RT_MAX_CIPHERTEXT_BYTES {
            return Err(Error::IntegerRange("realtime ciphertext limit"));
        }
        Ok(())
    }
    fn to_value_ref(&self) -> ValueRef<'_> {
        borrowed_arm(0, b"0", ValueRef::Binary(&self.0))
    }
    fn to_value(&self) -> Value {
        arm(0, "0", Value::Binary(self.0.clone()))
    }
    fn from_value(v: &Value) -> Result<Self> {
        let f = array(v, 2)?;
        crate::expect_unsigned(&f[0], "realtime ciphertext", 0)?;
        let bytes = binary(variant(&f[1], "0")?)?;
        if bytes.len() > RT_MAX_CIPHERTEXT_BYTES {
            return Err(Error::IntegerRange("realtime ciphertext limit"));
        }
        Ok(Self(bytes.to_vec()))
    }
}
#[derive(Clone, Eq, PartialEq)]
pub enum RtMessageWrapper {
    Plaintext(Vec<u8>),
    Encrypted(RtMessageBox),
}
impl std::fmt::Debug for RtMessageWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RtMessageWrapper([REDACTED])")
    }
}
impl Drop for RtMessageWrapper {
    fn drop(&mut self) {
        if let Self::Plaintext(body) = self {
            body.zeroize();
        }
    }
}
impl RealtimeWire for RtMessageWrapper {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Plaintext(body) => check_body_length(body.len()),
            Self::Encrypted(boxed) => boxed.validate(),
        }
    }
    fn to_value_ref(&self) -> ValueRef<'_> {
        match self {
            Self::Plaintext(body) => borrowed_arm(0, b"0", ValueRef::Binary(body)),
            Self::Encrypted(boxed) => borrowed_arm(1, b"1", boxed.to_value_ref()),
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Self::Plaintext(b) => arm(0, "0", Value::Binary(b.clone())),
            Self::Encrypted(b) => arm(1, "1", b.to_value()),
        }
    }
    fn from_value(v: &Value) -> Result<Self> {
        let f = array(v, 2)?;
        match unsigned(&f[0])? {
            0 => Ok(Self::Plaintext(blob(variant(&f[1], "0")?)?)),
            1 => Ok(Self::Encrypted(RtMessageBox::from_value(variant(
                &f[1], "1",
            )?)?)),
            value => Err(Error::UnknownEnum {
                kind: "realtime wrapper",
                value,
            }),
        }
    }
}

rt_struct!(RtBox {
    key: RoleAndGeneration,
    boxed: SecretBox
});

rt_struct!(RtMessageBox {
    ciphertext: RtCiphertext,
    key: RoleAndGeneration
});

rt_struct!(RtRolePair {
    read: Role,
    write: Role
});

rt_struct!(RtMessageMetadata { id: RtMessageId, previous_id: RtMessageId, previous_sequence: u64, send_time: u64, kind: RtMessageType, further_user_attribution: Option<RtUserId> });

rt_struct!(RtMessageNoncer { metadata: RtMessageMetadata, sender: Option<FqParty>, app: RtAppId, team: FqParty, channel: RtChannelId });

rt_struct!(RtLastMessage { sequence: u64, kind: RtMessageType, insert_time: u64, sender: Option<RtPartyId>, further_user_attribution: Option<RtUserId> });

rt_struct!(RtChannelMetadata { id: RtChannelId, team: RtTeamId, app: RtAppId, sequence: u64, name: RtBox, description: Option<RtBox>, roles: RtRolePair, last_message: Option<RtLastMessage>, ctime: u64, mtime: u64, updated_at: u64, tier: RtChannelTier, unreadable: bool });

rt_struct!(RtChannelSet { version: u64, channels: Vec<RtChannelMetadata>, mtime: u64 });

rt_struct!(RtInboxKey { app: RtAppId });

rt_struct!(RtChangedThreads {
    app: RtAppId,
    since: u64,
    maximum: u64
});

rt_struct!(RtReadThrough {
    channel: RtChannelId,
    sequence: u64
});

rt_struct!(RtPollInbox {
    app: RtAppId,
    since: u64,
    timeout_milliseconds: u64
});

rt_struct!(RtInboxPollResult {
    bumped: bool,
    inbox_version: u64
});

rt_struct!(RtInboxChannel {
    metadata: RtChannelMetadata,
    inbox_version: u64,
    read_through: u64,
    hidden: bool,
    muted: bool
});

rt_struct!(RtInboxDelta { inbox_version: u64, app: RtAppId, channels: Vec<RtInboxChannel> });

rt_struct!(RtSend {
    metadata: RtMessageMetadata,
    channel: RtChannelIdShort,
    wrapper: RtMessageWrapper,
    expected_previous_sequence: u64
});

rt_struct!(RtSendResult {
    sequence: u64,
    insert_time: u64
});

rt_struct!(RtThreadRange {
    start: u64,
    end: u64
});

rt_struct!(RtThreadQuery { channel: RtChannelId, ranges: Vec<RtThreadRange>, sequences: Vec<u64> });

rt_struct!(RtMessage { metadata: RtMessageMetadata, wrapper: RtMessageWrapper, sequence: u64, sender: Option<RtPartyId>, insert_time: u64 });

rt_struct!(RtMessageList { messages: Vec<RtMessage> });

rt_struct!(RtThreadPage { ranges: Vec<RtMessageList>, sequences: Vec<RtMessage> });

rt_struct!(RtCreateChannelArgument {
    metadata: RtChannelMetadata,
    set_version: u64
});

rt_struct!(RtListChannelsArgument {
    team: RtTeamId,
    app: RtAppId,
    last: u64
});

rt_struct!(RtSendArgument { send: RtSend });

rt_struct!(RtGetThreadArgument {
    query: RtThreadQuery
});

rt_struct!(RtGetInboxVersionArgument { key: RtInboxKey });

rt_struct!(RtGetChangedThreadsArgument {
    query: RtChangedThreads
});

rt_struct!(RtReadThroughArgument {
    read: RtReadThrough
});

rt_struct!(RtPollInboxArgument { poll: RtPollInbox });

rt_struct!(RtSelectVhostArgument { host: RtHostId });

rt_struct!(RtRecentsArgument {
    channel: RtChannelId,
    stop_at: u64,
    limit: u64
});

fn borrowed_arm<'a>(kind: u64, tag: &'a [u8], payload: ValueRef<'a>) -> ValueRef<'a> {
    ValueRef::Array(vec![
        ValueRef::Unsigned(kind),
        ValueRef::Variant(Some((tag, Box::new(payload)))),
    ])
}
fn check_body_length(length: usize) -> Result<()> {
    if length > RT_MAX_BODY_BYTES {
        return Err(Error::IntegerRange("realtime body limit"));
    }
    Ok(())
}
fn decode_plaintext<T: RealtimeWire>(bytes: &[u8]) -> Result<T> {
    if bytes.len() > RT_MAX_CIPHERTEXT_BYTES {
        return Err(Error::IntegerRange("realtime plaintext limit"));
    }
    let value =
        decode_sensitive(bytes).map_err(|_| Error::IntegerRange("malformed realtime plaintext"))?;
    T::from_value(&value).map_err(|error| match error {
        Error::UnknownEnum { .. } => error,
        Error::VariantTag { mut found, .. } => {
            found.zeroize();
            Error::IntegerRange("malformed realtime plaintext")
        }
        // Schema errors may carry an untrusted variant tag. Do not expose it.
        _ => Error::IntegerRange("malformed realtime plaintext"),
    })
}
