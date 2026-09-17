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
use std::{collections::HashMap, future::Future, io, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{Mutex, Semaphore},
};
use tokio_util::codec::{FramedRead, FramedWrite};

enum PendingRequest {
    Running,
    Publishing(Arc<Semaphore>),
}

pub struct BoundedTransport<R, W> {
    reader: FramedRead<R, JsonRpcMessageCodec<RxJsonRpcMessage<RoleServer>>>,
    writer: Arc<Mutex<FramedWrite<W, JsonRpcMessageCodec<TxJsonRpcMessage<RoleServer>>>>>,
    pending: Arc<Mutex<HashMap<RequestId, PendingRequest>>>,
    // The SDK cancels receive futures while polling other service events.
    // Keep a decoded frame until its admission awaits have completed.
    received: Option<RxJsonRpcMessage<RoleServer>>,
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
            received: None,
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
            // A peer may see all response bytes before flush completes. Mark
            // that ID as publishing so legitimate reuse waits for its completion.
            // Other IDs and cancellation notifications must remain readable even
            // when stdout is blocked; never hold the admission mutex across I/O.
            let completion = Arc::new(Semaphore::new(0));
            tokio::time::timeout(Duration::from_secs(30), async {
                if let Some(id) = &id {
                    pending
                        .lock()
                        .await
                        .insert(id.clone(), PendingRequest::Publishing(completion.clone()));
                }
                writer
                    .lock()
                    .await
                    .send(item)
                    .await
                    .map_err(|_| io::Error::other("MCP output failed"))?;
                if let Some(id) = &id {
                    pending.lock().await.remove(id);
                }
                completion.close();
                Ok::<(), io::Error>(())
            })
            .await
            .map_err(|_| io::Error::other("MCP output stalled"))??;
            guard.disarm();
            Ok(())
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleServer>> {
        if self.stopped {
            return None;
        }
        if self.received.is_none() {
            let Some(Ok(message)) = self.reader.next().await else {
                self.stopped = true;
                self.lifetime.cancel();
                return None;
            };
            self.received = Some(message);
        }
        if let Some(JsonRpcMessage::Request(request)) = &self.received {
            if serde_json::to_vec(&request.id).ok()?.len() > 256 {
                self.stopped = true;
                self.lifetime.cancel();
                return None;
            }
            loop {
                let mut pending = self.pending.lock().await;
                match pending.get(&request.id) {
                    Some(PendingRequest::Publishing(completion)) => {
                        let completion = completion.clone();
                        drop(pending);
                        tokio::select! {
                            _ = completion.acquire() => (),
                            _ = self.lifetime.cancelled() => {self.stopped=true;return None;}
                        }
                    }
                    None if pending.len() < ACTIVE_CALLS + QUEUED_CALLS => {
                        pending.insert(request.id.clone(), PendingRequest::Running);
                        break;
                    }
                    _ => {
                        // Bound pipelining without queuing unbounded busy replies.
                        self.stopped = true;
                        self.lifetime.cancel();
                        return None;
                    }
                }
            }
        }
        self.received.take()
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

    #[derive(Default)]
    struct FlushGate {
        written: tokio::sync::Notify,
        released: std::sync::atomic::AtomicBool,
        waker: futures_util::task::AtomicWaker,
    }
    struct PublishedBeforeFlush(Arc<FlushGate>);
    impl AsyncWrite for PublishedBeforeFlush {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            bytes: &[u8],
        ) -> std::task::Poll<io::Result<usize>> {
            self.0.written.notify_one();
            std::task::Poll::Ready(Ok(bytes.len()))
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            self.0.waker.register(cx.waker());
            if self.0.released.load(std::sync::atomic::Ordering::Acquire) {
                std::task::Poll::Ready(Ok(()))
            } else {
                std::task::Poll::Pending
            }
        }
        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn request_id_reuse_waits_for_response_publication_to_finish() {
        let (mut input, reader) = tokio::io::duplex(1024);
        let gate = Arc::new(FlushGate::default());
        let mut transport = BoundedTransport::new(reader, PublishedBeforeFlush(gate.clone()));
        let request = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n";
        input.write_all(request).await.unwrap();
        assert!(transport.receive().await.is_some());
        let response = serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#).unwrap();
        let send = tokio::spawn(transport.send(response));
        gate.written.notified().await;
        // Unrelated requests and cancellation remain readable during blocked output.
        input
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n")
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), transport.receive())
                .await
                .unwrap()
                .is_some()
        );
        input.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":2}}\n").await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), transport.receive())
                .await
                .unwrap()
                .is_some()
        );
        input.write_all(request).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), transport.receive())
                .await
                .is_err()
        );
        gate.released
            .store(true, std::sync::atomic::Ordering::Release);
        gate.waker.wake();
        send.await.unwrap().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), transport.receive())
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn cancelled_admission_retains_the_decoded_request() {
        let (mut input, reader) = tokio::io::duplex(1024);
        let mut transport = BoundedTransport::new(reader, tokio::io::sink());
        let pending = transport.pending.clone();
        let lock = pending.lock().await;
        input
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n")
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), transport.receive())
                .await
                .is_err()
        );
        assert!(transport.received.is_some());
        drop(lock);
        let message = tokio::time::timeout(Duration::from_millis(100), transport.receive())
            .await
            .unwrap()
            .unwrap();
        let JsonRpcMessage::Request(request) = message else {
            panic!("expected retained request")
        };
        assert_eq!(request.id, RequestId::Number(7));
        assert!(transport.received.is_none());
        assert!(matches!(
            pending.lock().await.get(&request.id),
            Some(PendingRequest::Running)
        ));
    }

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
