//! Low-level realtime connection. Team verification and durable workflows are separate.
use crate::{DeviceCredential, Error, FoksClient, PinnedHost, PooledConnection, Result};
use foks_proto::{RtHostId, RtSelectVhostArgument};
use foks_rpc::{RealtimeRequest, RealtimeResponse};
use std::time::Duration;

pub struct RealtimeConnection {
    pooled: PooledConnection,
}
impl RealtimeConnection {
    pub fn call(&mut self, request: &RealtimeRequest) -> Result<RealtimeResponse> {
        if matches!(request, RealtimeRequest::SelectVhost(_)) {
            return Err(Error::Transport(
                "realtime connection is already bound to its pinned host",
            ));
        }
        let encoded = request.encode_at(0)?;
        let result = self.pooled.call(&encoded, request.is_void())?;
        match request.decode_result(&result) {
            Ok(response) => Ok(response),
            Err(error) => {
                self.pooled.invalidate();
                Err(error.into())
            }
        }
    }
}
impl FoksClient {
    pub fn realtime_poll_connection(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<RealtimeConnection> {
        self.isolated_with_timeout(Duration::from_secs(60))
            .realtime_connection(host, credential)
    }

    pub fn realtime_connection(
        &self,
        host: &PinnedHost,
        credential: &DeviceCredential,
    ) -> Result<RealtimeConnection> {
        let target = host
            .realtime
            .as_ref()
            .ok_or(Error::PinnedService("realtime"))?;
        let select = RealtimeRequest::SelectVhost(RtSelectVhostArgument {
            host: RtHostId::new(host.host_id.clone())?,
        })
        .encode_at(0)?;
        Ok(RealtimeConnection {
            pooled: self.pooled_connection_with_material(
                host,
                target,
                &credential.seed,
                &credential.certificate_chain,
                &select,
            )?,
        })
    }
}

mod history;
mod inbox;
mod operations;
mod session;
pub use history::{ChatContent, ChatHistory, ChatMessage};
pub use inbox::{
    ChatConversation, ChatInbox, ChatPollResult, ChatPreview, ChatPreviewContent, ChatSyncResult,
};
pub use operations::normalize_chat_name;
pub use session::{ChatChannel, ChatChannels, ChatSession, ChatTransport};

mod policy;
pub use policy::ChatLimits;
