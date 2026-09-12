//! Keep framing and pending responses bounded, including under a blocked stdout.
//! The SDK owns JSON-RPC dispatch, initialization and cancellation semantics.

use crate::contract::{ACTIVE_CALLS, MAX_INPUT_BYTES, MAX_OUTPUT_BYTES, QUEUED_CALLS};
use futures_util::{SinkExt as _, StreamExt as _};
use rmcp::{
    model::{JsonRpcMessage, RequestId},
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{async_rw::JsonRpcMessageCodec, Transport},
    RoleServer,
};
use std::{collections::HashSet, future::Future, io, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::Mutex,
};
use tokio_util::codec::{FramedRead, FramedWrite};

pub struct BoundedTransport<R, W> {
    reader: FramedRead<R, JsonRpcMessageCodec<RxJsonRpcMessage<RoleServer>>>,
    writer: Arc<Mutex<FramedWrite<W, JsonRpcMessageCodec<TxJsonRpcMessage<RoleServer>>>>>,
    pending: Arc<std::sync::Mutex<HashSet<RequestId>>>,
    stopped: bool,
    lifetime: tokio_util::sync::CancellationToken,
}

impl<R: AsyncRead, W: AsyncWrite> BoundedTransport<R, W> {
    pub fn lifetime(&self) -> tokio_util::sync::CancellationToken {
        self.lifetime.clone()
    }

    pub fn new(reader: R, writer: W) -> Self {
        Self::with_input_limit(reader, writer, MAX_INPUT_BYTES)
    }

    fn with_input_limit(reader: R, writer: W, limit: usize) -> Self {
        Self {
            reader: FramedRead::new(reader, JsonRpcMessageCodec::new_with_max_length(limit)),
            writer: Arc::new(Mutex::new(FramedWrite::new(
                writer,
                JsonRpcMessageCodec::new(),
            ))),
            pending: Arc::default(),
            stopped: false,
            lifetime: tokio_util::sync::CancellationToken::new(),
        }
    }
}

impl<R, W> Transport<RoleServer> for BoundedTransport<R, W>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send + 'static,
{
    type Error = io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleServer>,
    ) -> impl Future<Output = io::Result<()>> + Send + 'static {
        let writer = self.writer.clone();
        let pending = self.pending.clone();
        let guard = self.lifetime.clone().drop_guard();
        async move {
            // Check encoded size, not plaintext length: JSON escaping can expand sixfold.
            let bytes =
                serde_json::to_vec(&item).map_err(|_| io::Error::other("invalid MCP output"))?;
            if bytes.len() > MAX_OUTPUT_BYTES {
                return Err(io::Error::other("MCP output limit exceeded"));
            }
            drop(bytes);
            let id = match &item {
                JsonRpcMessage::Response(r) => Some(r.id.clone()),
                JsonRpcMessage::Error(e) => e.id.clone(),
                _ => None,
            };
            tokio::time::timeout(Duration::from_secs(30), async {
                writer
                    .lock()
                    .await
                    .send(item)
                    .await
                    .map_err(|_| io::Error::other("MCP output failed"))
            })
            .await
            .map_err(|_| io::Error::other("MCP output stalled"))??;
            if let Some(id) = id {
                pending
                    .lock()
                    .map_err(|_| io::Error::other("MCP admission unavailable"))?
                    .remove(&id);
            }
            guard.disarm();
            Ok(())
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        if self.stopped {
            return None;
        }
        let Some(Ok(message)) = self.reader.next().await else {
            self.stopped = true;
            self.lifetime.cancel();
            return None;
        };
        if let JsonRpcMessage::Request(request) = &message {
            if serde_json::to_vec(&request.id).ok()?.len() > 256 {
                self.stopped = true;
                self.lifetime.cancel();
                return None;
            }
            let mut pending = self.pending.lock().ok()?;
            if pending.len() >= ACTIVE_CALLS + QUEUED_CALLS || !pending.insert(request.id.clone()) {
                // Closing rejects abusive pipelining without queuing unbounded busy responses.
                // Cancellation notifications do not consume a request slot.
                self.stopped = true;
                self.lifetime.cancel();
                return None;
            }
        }
        Some(message)
    }

    async fn close(&mut self) -> io::Result<()> {
        self.stopped = true;
        self.lifetime.cancel();
        tokio::time::timeout(Duration::from_secs(1), async {
            self.writer.lock().await.close().await
        })
        .await
        .map_err(|_| io::Error::other("MCP close timed out"))?
        .map_err(|_| io::Error::other("MCP close failed"))
    }
}

impl<R, W> Drop for BoundedTransport<R, W> {
    fn drop(&mut self) {
        self.lifetime.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt as _;

    #[tokio::test]
    async fn oversized_and_duplicate_requests_close_without_dispatch() {
        let (mut input, reader) = tokio::io::duplex(1024);
        let mut transport = BoundedTransport::with_input_limit(reader, tokio::io::sink(), 64);
        input.write_all(&[b' '; 65]).await.unwrap();
        assert!(transport.receive().await.is_none());
        let (mut input, reader) = tokio::io::duplex(1024);
        let mut transport = BoundedTransport::new(reader, tokio::io::sink());
        let request = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n";
        input.write_all(request).await.unwrap();
        input.write_all(request).await.unwrap();
        assert!(transport.receive().await.is_some());
        assert!(transport.receive().await.is_none());
    }

    #[tokio::test]
    async fn outstanding_request_budget_includes_unsent_responses() {
        let (mut input, reader) = tokio::io::duplex(8192);
        let mut transport = BoundedTransport::new(reader, tokio::io::sink());
        for id in 0..=ACTIVE_CALLS + QUEUED_CALLS {
            input
                .write_all(
                    format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"ping\"}}\n").as_bytes(),
                )
                .await
                .unwrap();
            assert_eq!(
                transport.receive().await.is_some(),
                id < ACTIVE_CALLS + QUEUED_CALLS
            );
        }
    }
}
