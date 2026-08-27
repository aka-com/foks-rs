use std::io::Write as _;
use std::net::TcpStream;
use std::sync::Arc;

use foks_rpc::{encode_probe_success_response, encode_status_response_at, read_call, RpcStatus};

use crate::rpc::{route_call, Listener};
use crate::{Result, SessionLimits};

pub(crate) fn serve(
    stream: TcpStream,
    listener: Listener,
    tls: &Arc<rustls::ServerConfig>,
    probe_response: &[u8],
    limits: SessionLimits,
) -> Result<()> {
    stream.set_read_timeout(Some(limits.io_timeout))?;
    stream.set_write_timeout(Some(limits.io_timeout))?;
    let connection = rustls::ServerConnection::new(Arc::clone(tls))?;
    let mut stream = rustls::StreamOwned::new(connection, stream);
    for _ in 0..limits.maximum_requests {
        let call = match read_call(&mut stream, limits.maximum_frame_bytes) {
            Ok(call) => call,
            Err(foks_rpc::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock
                ) =>
            {
                break;
            }
            Err(error) => return Err(error.into()),
        };
        let sequence = call.sequence();
        let response = match route_call(call, listener) {
            Ok(call) if listener == Listener::Probe && call.route.method == "probe" => {
                encode_probe_success_response(probe_response)?
            }
            _ => encode_status_response_at(&RpcStatus::Unsupported, sequence)?,
        };
        stream.write_all(&response)?;
        stream.flush()?;
        while stream.conn.wants_write() {
            stream.conn.complete_io(&mut stream.sock)?;
        }
    }
    Ok(())
}
