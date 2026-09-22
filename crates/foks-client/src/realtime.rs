//! Low-level realtime connection. Team verification and durable workflows are separate.
use crate::{DeviceCredential, Error, FoksClient, PinnedHost, PooledConnection, Result};
use foks_proto::{RtHostId, RtSelectVhostArgument};
use foks_rpc::{RealtimeRequest, RealtimeResponse};
use std::time::Duration;

/// How long a poll connection may spend connecting, handshaking and binding
/// its pinned host. The caller holds its profile's admission while it opens
/// the connection, so every other request for that profile waits behind a
/// realtime host that accepts a connection and never answers; only the long
/// poll that follows, which runs without admission, may last the poll budget.
const REALTIME_CONNECT_BUDGET: Duration = Duration::from_secs(20);

/// How long one long poll on an established connection may wait for an
/// event before the server answers empty.
const REALTIME_POLL_BUDGET: Duration = Duration::from_secs(60);

/// The deadline for opening a poll connection: the client's own budget,
/// bounded by what a caller may hold its profile's admission for.
fn realtime_connect_budget(timeout: Duration) -> Duration {
    timeout.min(REALTIME_CONNECT_BUDGET)
}

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
        // Connecting runs under the bounded budget; the connection then takes
        // the poll budget for the calls that follow. The pool is the
        // connection's own, so no ordinary request borrows a poll socket.
        let mut connection = self
            .isolated_with_timeout(realtime_connect_budget(self.timeout()))
            .realtime_connection(host, credential)?;
        connection.pooled.set_timeout(REALTIME_POLL_BUDGET);
        Ok(connection)
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

mod capabilities;
mod history;
mod inbox;
mod operations;
pub(crate) use operations::validate_inventory_request;
mod session;
pub use history::{ChatContent, ChatHistory, ChatMessage};
pub use inbox::{
    ChatConversation, ChatInbox, ChatPollResult, ChatPreview, ChatPreviewCache, ChatPreviewContent,
    ChatPreviewKey, ChatSyncResult, NoChatPreviewCache,
};
pub use operations::normalize_chat_name;
pub use session::{ChatChannel, ChatChannels, ChatReadReuse, ChatSession, ChatTransport};

mod policy;
pub use policy::ChatLimits;

#[cfg(test)]
mod tests {
    use super::*;
    use foks_client_db::{Acceptance, HardStateStore};
    use foks_verify::verify_public_host;

    #[test]
    fn connect_budget_is_bounded_below_the_poll_budget() {
        assert_eq!(
            realtime_connect_budget(Duration::from_secs(60)),
            REALTIME_CONNECT_BUDGET
        );
        assert_eq!(
            realtime_connect_budget(Duration::from_secs(5)),
            Duration::from_secs(5)
        );
        assert!(REALTIME_CONNECT_BUDGET < REALTIME_POLL_BUDGET);
    }

    /// A realtime host that accepts the connection and never completes the
    /// handshake must release the caller within its own budget, not the poll
    /// budget: the caller holds its profile's admission meanwhile.
    #[test]
    fn a_silent_realtime_host_releases_the_caller_within_the_connect_budget() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        // Accepts the connection and holds it open without a byte in reply
        // until the test is done, whether or not the client ever connects.
        let accepting = std::thread::spawn(move || {
            let mut accepted = None;
            loop {
                if accepted.is_none() {
                    if let Ok((socket, _)) = listener.accept() {
                        accepted = Some(socket);
                    }
                }
                if done_rx.recv_timeout(Duration::from_millis(20)).is_ok() {
                    break;
                }
            }
            drop(accepted);
        });
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("hard.sqlite3");
        let snapshot = verify_public_host(
            "foks.app",
            include_bytes!(
                "../../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
            ),
        )
        .unwrap()
        .snapshot;
        assert_eq!(
            HardStateStore::open(&database)
                .unwrap()
                .accept_verified_host(&snapshot)
                .unwrap(),
            Acceptance::Inserted
        );
        let mut client = FoksClient::default();
        client.set_timeout(Duration::from_secs(1));
        let mut host = client.pinned_host("foks.app", &database).unwrap();
        host.realtime = Some(crate::ProbeTarget {
            hostname: "127.0.0.1".to_owned(),
            port,
        });
        let mut uid = vec![0x21; 33];
        uid[0] = foks_proto::ENTITY_USER;
        // The device presents a certificate for its own key: the client
        // checks the two agree before it connects.
        let seed = foks_proto::SecretSeed::new([0x71; 32]);
        let pkcs8 = foks_crypto::device_signing_key_pkcs8(&seed).unwrap();
        let key = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(
            &rustls::pki_types::PrivatePkcs8KeyDer::from(pkcs8.to_vec()),
            &rcgen::PKCS_ED25519,
        )
        .unwrap();
        let certificate = rcgen::CertificateParams::default()
            .self_signed(&key)
            .unwrap();
        let credential = DeviceCredential {
            key_kind: crate::SoftwareKeyKind::Device,
            uid: foks_proto::EntityId::from_bytes(uid).unwrap(),
            seed,
            certificate_chain: vec![certificate.der().to_vec()],
        };
        let started = std::time::Instant::now();
        let error = client
            .realtime_poll_connection(&host, &credential)
            .err()
            .expect("a silent host does not yield a poll connection");
        let elapsed = started.elapsed();
        done_tx.send(()).unwrap();
        accepting.join().unwrap();
        assert!(
            matches!(error, Error::DeadlineExceeded),
            "gave up with {error:?} after {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "the poll budget, not the connect budget, bounded the connect: {elapsed:?}"
        );
    }
}
