//! Typed v0.1.9 realtime calls. All payloads use the Header/Data RPC envelope.
use crate::{encode_call, Result, REAL_TIME_PROTOCOL_ID};
use foks_proto::{
    ChatSendReceipt, RealtimeWire, RtChannelSet, RtChatCapabilities, RtChatCapabilitiesArgument,
    RtCreateChannelArgument, RtGetChangedThreadsArgument, RtGetInboxVersionArgument,
    RtGetThreadArgument, RtInboxDelta, RtInboxPollResult, RtListChannelsArgument, RtMessageList,
    RtPollInboxArgument, RtReadThroughArgument, RtRecentsArgument, RtSelectVhostArgument,
    RtSendArgument, RtThreadPage,
};

pub use foks_proto::RT_MAX_REQUEST_BYTES;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RealtimeRequest {
    ChatCapabilities(RtChatCapabilitiesArgument),
    CreateChannel(RtCreateChannelArgument),
    ListChannels(RtListChannelsArgument),
    Send(RtSendArgument),
    GetThread(RtGetThreadArgument),
    GetInboxVersion(RtGetInboxVersionArgument),
    GetChangedThreads(RtGetChangedThreadsArgument),
    ReadThrough(RtReadThroughArgument),
    PollInbox(RtPollInboxArgument),
    SelectVhost(RtSelectVhostArgument),
    Recents(RtRecentsArgument),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RealtimeResponse {
    ChatCapabilities(RtChatCapabilities),
    Void,
    Channels(RtChannelSet),
    Sent(ChatSendReceipt),
    Thread(RtThreadPage),
    InboxVersion(u64),
    InboxDelta(RtInboxDelta),
    PollResult(RtInboxPollResult),
    Messages(RtMessageList),
}

impl RealtimeRequest {
    pub fn position(&self) -> u64 {
        use crate::*;
        match self {
            Self::ChatCapabilities(_) => RT_CHAT_CAPABILITIES_METHOD_POSITION,
            Self::CreateChannel(_) => RT_NEW_CHANNEL_METHOD_POSITION,
            Self::ListChannels(_) => RT_LIST_CHANNELS_METHOD_POSITION,
            Self::Send(_) => RT_SEND_METHOD_POSITION,
            Self::GetThread(_) => RT_GET_THREAD_METHOD_POSITION,
            Self::GetInboxVersion(_) => RT_GET_INBOX_VERSION_METHOD_POSITION,
            Self::GetChangedThreads(_) => RT_GET_CHANGED_THREADS_METHOD_POSITION,
            Self::ReadThrough(_) => RT_READ_THROUGH_METHOD_POSITION,
            Self::PollInbox(_) => RT_POLL_INBOX_METHOD_POSITION,
            Self::SelectVhost(_) => RT_SELECT_VHOST_METHOD_POSITION,
            Self::Recents(_) => RT_RECENTS_METHOD_POSITION,
        }
    }
    pub fn is_void(&self) -> bool {
        matches!(
            self,
            Self::CreateChannel(_) | Self::ReadThrough(_) | Self::SelectVhost(_)
        )
    }
    pub fn argument(&self) -> Result<Vec<u8>> {
        let bytes = match self {
            Self::ChatCapabilities(v) => v.encoded()?,
            Self::CreateChannel(v) => v.encoded()?,
            Self::ListChannels(v) => v.encoded()?,
            Self::Send(v) => v.encoded()?,
            Self::GetThread(v) => v.encoded()?,
            Self::GetInboxVersion(v) => v.encoded()?,
            Self::GetChangedThreads(v) => v.encoded()?,
            Self::ReadThrough(v) => v.encoded()?,
            Self::PollInbox(v) => v.encoded()?,
            Self::SelectVhost(v) => v.encoded()?,
            Self::Recents(v) => v.encoded()?,
        };
        check_request_size(&bytes)?;
        Ok(bytes)
    }
    pub fn encode_at(&self, sequence: u64) -> Result<Vec<u8>> {
        encode_call(
            REAL_TIME_PROTOCOL_ID,
            self.position(),
            &self.argument()?,
            sequence,
        )
    }
    /// Decode an already unwrapped canonical request, as supplied by `decode_call`.
    pub fn decode_argument(position: u64, bytes: &[u8]) -> Result<Self> {
        use crate::*;
        check_request_size(bytes)?;
        Ok(match position {
            RT_CHAT_CAPABILITIES_METHOD_POSITION => {
                Self::ChatCapabilities(RtChatCapabilitiesArgument::decode(bytes)?)
            }
            RT_NEW_CHANNEL_METHOD_POSITION => {
                Self::CreateChannel(RtCreateChannelArgument::decode(bytes)?)
            }
            RT_LIST_CHANNELS_METHOD_POSITION => {
                Self::ListChannels(RtListChannelsArgument::decode(bytes)?)
            }
            RT_SEND_METHOD_POSITION => Self::Send(RtSendArgument::decode(bytes)?),
            RT_GET_THREAD_METHOD_POSITION => Self::GetThread(RtGetThreadArgument::decode(bytes)?),
            RT_GET_INBOX_VERSION_METHOD_POSITION => {
                Self::GetInboxVersion(RtGetInboxVersionArgument::decode(bytes)?)
            }
            RT_GET_CHANGED_THREADS_METHOD_POSITION => {
                Self::GetChangedThreads(RtGetChangedThreadsArgument::decode(bytes)?)
            }
            RT_READ_THROUGH_METHOD_POSITION => {
                Self::ReadThrough(RtReadThroughArgument::decode(bytes)?)
            }
            RT_POLL_INBOX_METHOD_POSITION => Self::PollInbox(RtPollInboxArgument::decode(bytes)?),
            RT_SELECT_VHOST_METHOD_POSITION => {
                Self::SelectVhost(RtSelectVhostArgument::decode(bytes)?)
            }
            RT_RECENTS_METHOD_POSITION => Self::Recents(RtRecentsArgument::decode(bytes)?),
            _ => {
                return Err(crate::Error::Envelope {
                    expected: "supported realtime method",
                    found: "unsupported method",
                })
            }
        })
    }
    /// Decode response Data after the transport has checked status, header and sequence.
    pub fn decode_result(&self, bytes: &[u8]) -> Result<RealtimeResponse> {
        Ok(match self {
            Self::ChatCapabilities(request) => {
                let response = RtChatCapabilities::decode(bytes)?;
                if response.host != request.host {
                    return Err(crate::Error::Envelope {
                        expected: "requested capability host",
                        found: "different host",
                    });
                }
                RealtimeResponse::ChatCapabilities(response)
            }
            Self::CreateChannel(_) | Self::ReadThrough(_) | Self::SelectVhost(_) => {
                // The transport's checked void reader returns an empty buffer.
                if !bytes.is_empty() && bytes != [0xc0] {
                    return Err(crate::Error::Envelope {
                        expected: "void realtime result",
                        found: "non-void result",
                    });
                }
                RealtimeResponse::Void
            }
            Self::ListChannels(_) => RealtimeResponse::Channels(RtChannelSet::decode(bytes)?),
            Self::Send(_) => RealtimeResponse::Sent(ChatSendReceipt::decode(bytes)?),
            Self::GetThread(_) => RealtimeResponse::Thread(RtThreadPage::decode(bytes)?),
            Self::GetInboxVersion(_) => RealtimeResponse::InboxVersion(u64::decode(bytes)?),
            Self::GetChangedThreads(_) => {
                RealtimeResponse::InboxDelta(RtInboxDelta::decode(bytes)?)
            }
            Self::PollInbox(_) => RealtimeResponse::PollResult(RtInboxPollResult::decode(bytes)?),
            Self::Recents(_) => RealtimeResponse::Messages(RtMessageList::decode(bytes)?),
        })
    }
}
fn check_request_size(bytes: &[u8]) -> Result<()> {
    if bytes.len() > RT_MAX_REQUEST_BYTES {
        return Err(crate::Error::FrameTooLarge {
            received: bytes.len(),
            maximum: RT_MAX_REQUEST_BYTES,
        });
    }
    Ok(())
}
