use super::history::anchor;
use super::policy::{ChatLimits, PROTECTED_MATERIAL_DOMAIN, REQUEST_HASH_DOMAIN};
use super::session::{floor, random_id, ChatSession, ChatTransport};
use crate::{Error, ProtectedMutationStore, Result};
use foks_client_db::{
    ChatOperation, ChatOperationKind as Kind, ChatOperationState as State, ChatSubmission,
};
use foks_proto::{
    RealtimeWire, Role, RtAppId, RtBox, RtChannelId, RtChannelMetadata, RtChannelTier,
    RtCreateChannelArgument, RtKeyType, RtMessage, RtMessageBox, RtMessageId, RtMessageMetadata,
    RtMessageType, RtMessageWrapper, RtPartyId, RtRecentsArgument, RtRolePair, RtSend,
    RtSendArgument, RtSendResult, RtTeamId, RtText, RT_MAX_BODY_BYTES,
};
use foks_rpc::{RealtimeRequest, RealtimeResponse};
use zeroize::Zeroizing;

mod attempt;
mod material;
mod prepare;
mod recovery;
pub use prepare::normalize_chat_name;
#[cfg(test)]
use recovery::recovery_window;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_adapts_to_local_limits_but_does_not_retry_integrity_errors() {
        let mut widths = Vec::new();
        let result = recovery_window(|width| {
            widths.push(width);
            if width > 1 {
                Err(Error::ChatLimit("page bytes"))
            } else {
                Ok(42)
            }
        })
        .unwrap();
        assert_eq!(result, 42);
        assert_eq!(widths, vec![100, 50, 25, 12, 6, 3, 1]);
        let mut attempts = 0;
        assert!(matches!(
            recovery_window::<()>(|_| {
                attempts += 1;
                Err(Error::ChatIntegrity("retained anchor contradiction"))
            }),
            Err(Error::ChatIntegrity(_))
        ));
        assert_eq!(attempts, 1);
    }
    #[test]
    fn go_channel_normalization_rules() {
        for (input, expected) in [
            ("", ""),
            ("  HeLLo  ", "hello"),
            ("日本語", "日本語"),
            ("-abc-", "-abc-"),
            ("İST", "ist"),
            ("💬💬💬", "💬💬💬"),
        ] {
            assert_eq!(normalize_chat_name(input).unwrap(), expected);
        }
        for input in [
            "general",
            "ab",
            "a--b",
            "a b",
            "a_b",
            "abc!",
            "abc\u{200b}",
            "abc\u{7f}",
        ] {
            assert!(
                matches!(normalize_chat_name(input), Err(Error::ChatInvalidInput(_))),
                "{input}"
            );
        }
    }
}
