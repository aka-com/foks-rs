//! Capability discovery is independent of Basic history and inbox ownership.
use super::{ChatSession, ChatTransport};
use crate::{Error, Result};
use foks_proto::{RealtimeWire, RtChatCapabilities, RtChatCapabilitiesArgument, RtHostId};
use foks_rpc::{
    RealtimeRequest, RealtimeResponse, REAL_TIME_PROTOCOL_ID, RT_CHAT_CAPABILITIES_METHOD_POSITION,
    STATUS_NOT_IMPLEMENTED,
};

impl ChatSession<'_> {
    /// Query the authenticated host. Do not cache this across identity changes.
    pub fn capabilities(&self, rpc: &mut impl ChatTransport) -> Result<RtChatCapabilities> {
        query(rpc, RtHostId::new(self.host.host_id().clone())?)
    }
}

fn query(rpc: &mut impl ChatTransport, host: RtHostId) -> Result<RtChatCapabilities> {
    match rpc.request(&RealtimeRequest::ChatCapabilities(
        RtChatCapabilitiesArgument { host: host.clone() },
    )) {
        Ok(RealtimeResponse::ChatCapabilities(capabilities)) => {
            if capabilities.host != host {
                return Err(Error::ChatIntegrity("capabilities host mismatch"));
            }
            capabilities
                .validate()
                .map_err(|_| Error::ChatIntegrity("invalid capabilities"))?;
            Ok(capabilities)
        }
        Ok(_) => Err(Error::ChatIntegrity("unexpected capabilities response")),
        Err(Error::Rpc(foks_rpc::Error::MethodNotFound {
            protocol_id: REAL_TIME_PROTOCOL_ID,
            position: RT_CHAT_CAPABILITIES_METHOD_POSITION,
        }))
        | Err(Error::Rpc(foks_rpc::Error::RemoteStatus {
            code: STATUS_NOT_IMPLEMENTED,
            ..
        })) => Ok(RtChatCapabilities::basic_only(host)),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Reply(Option<Result<RealtimeResponse>>);
    impl ChatTransport for Reply {
        fn request(&mut self, request: &RealtimeRequest) -> Result<RealtimeResponse> {
            assert!(matches!(request, RealtimeRequest::ChatCapabilities(_)));
            self.0.take().unwrap()
        }
    }
    fn host(byte: u8) -> RtHostId {
        let mut bytes = vec![byte; 33];
        bytes[0] = foks_proto::ENTITY_HOST;
        RtHostId::new(foks_proto::EntityId::from_bytes(bytes).unwrap()).unwrap()
    }
    #[test]
    fn only_explicit_matching_unsupported_outcomes_mean_basic_only() {
        let expected = RtChatCapabilities::basic_only(host(1));
        let unknown = Error::Rpc(foks_rpc::Error::MethodNotFound {
            protocol_id: REAL_TIME_PROTOCOL_ID,
            position: RT_CHAT_CAPABILITIES_METHOD_POSITION,
        });
        assert_eq!(
            query(&mut Reply(Some(Err(unknown))), host(1)).unwrap(),
            expected
        );
        for response in [
            Err(Error::DeadlineExceeded),
            Err(Error::ChatAccessDenied("removed")),
            Err(Error::Rpc(foks_rpc::Error::MethodNotFound {
                protocol_id: REAL_TIME_PROTOCOL_ID,
                position: 0,
            })),
            Ok(RealtimeResponse::Void),
            Ok(RealtimeResponse::ChatCapabilities(
                RtChatCapabilities::basic_only(host(2)),
            )),
        ] {
            assert!(query(&mut Reply(Some(response)), host(1)).is_err());
        }
    }
}
