use std::io::Write as _;
use std::net::TcpStream;
use std::sync::Arc;

use foks_rpc::{
    encode_status_response_at, encode_success_response_at, encode_void_success_response_at,
    read_call, RpcStatus,
};
use foks_snowpack::{decode, Value};

use crate::rpc::{route_call, Listener, RouteError, RoutedCall};
use crate::{Result, SessionLimits};

pub(crate) struct ServerData {
    probe_response: Arc<[u8]>,
    host_id: Vec<u8>,
    canonical_name: String,
    current_root: Vec<u8>,
}

impl ServerData {
    pub(crate) fn from_probe(probe_response: Arc<[u8]>) -> Result<Self> {
        let probe = foks_proto::ProbeResponse::decode(&probe_response)?;
        let first = probe
            .hostchain
            .first()
            .ok_or(crate::Error::Config("empty bootstrap hostchain"))?;
        let host_id = first.decode_change()?.host.into_bytes();
        let zone = foks_proto::PublicZone::decode(&probe.public_zone.inner)?;
        let canonical_name = endpoint_host(&zone.services.probe)
            .ok_or(crate::Error::Config("invalid bootstrap probe endpoint"))?
            .to_owned();
        Ok(Self {
            probe_response,
            host_id,
            canonical_name,
            current_root: probe.merkle_root.inner,
        })
    }

    fn response(&self, call: RoutedCall) -> std::result::Result<Vec<u8>, RpcStatus> {
        let sequence = call.call.sequence();
        match (call.route.protocol, call.route.method) {
            ("Probe", "probe") => {
                self.validate_probe(call.call.argument())?;
                encode_success_response_at(&self.probe_response, sequence)
                    .map_err(|_| RpcStatus::Unsupported)
            }
            ("Reg" | "MerkleQuery" | "KvStore", "selectVHost") => {
                self.validate_host_argument(call.call.argument())?;
                encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
            }
            ("MerkleQuery", "getCurrentRoot") => {
                self.validate_host_argument(call.call.argument())?;
                encode_success_response_at(&self.current_root, sequence)
                    .map_err(|_| RpcStatus::Unsupported)
            }
            _ => Err(RpcStatus::Unsupported),
        }
    }

    fn validate_host_argument(&self, argument: &[u8]) -> std::result::Result<(), RpcStatus> {
        let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
            return Err(bad_arguments("host argument is not a struct"));
        };
        let [Value::Binary(host)] = fields.as_slice() else {
            return Err(bad_arguments("host argument has the wrong shape"));
        };
        if host.len() != 33 || host.first() != Some(&foks_proto::ENTITY_HOST) {
            return Err(bad_arguments("host argument is not a HostID"));
        }
        if host != &self.host_id {
            return Err(RpcStatus::NotFound("host not found".to_owned()));
        }
        Ok(())
    }

    fn validate_probe(&self, argument: &[u8]) -> std::result::Result<(), RpcStatus> {
        let Value::Array(fields) = decode(argument).map_err(bad_arguments)? else {
            return Err(bad_arguments("probe argument is not a struct"));
        };
        let [Value::Text(hostname), Value::Unsigned(_), host_id] = fields.as_slice() else {
            return Err(bad_arguments("probe argument has the wrong shape"));
        };
        if hostname.as_slice() != self.canonical_name.as_bytes() {
            return Err(RpcStatus::NotFound("host not found".to_owned()));
        }
        match host_id {
            Value::Null => Ok(()),
            Value::Binary(host_id)
                if host_id.len() == 33
                    && host_id.first() == Some(&foks_proto::ENTITY_HOST)
                    && host_id == &self.host_id =>
            {
                Ok(())
            }
            Value::Binary(host_id)
                if host_id.len() == 33 && host_id.first() == Some(&foks_proto::ENTITY_HOST) =>
            {
                Err(RpcStatus::NotFound("host not found".to_owned()))
            }
            Value::Binary(_) => Err(bad_arguments("probe HostID is malformed")),
            _ => Err(bad_arguments("probe HostID has the wrong shape")),
        }
    }
}

pub(crate) fn serve(
    stream: TcpStream,
    listener: Listener,
    tls: &Arc<rustls::ServerConfig>,
    service_data: &Arc<ServerData>,
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
            Ok(call) => match service_data.response(call) {
                Ok(response) => response,
                Err(status) => encode_status_response_at(&status, sequence)?,
            },
            Err(RouteError::RequestTooLarge { .. }) => encode_status_response_at(
                &RpcStatus::BadArguments("request exceeds the method limit".to_owned()),
                sequence,
            )?,
            Err(RouteError::Unknown { .. } | RouteError::WrongListener) => {
                encode_status_response_at(&RpcStatus::Unsupported, sequence)?
            }
        };
        stream.write_all(&response)?;
        stream.flush()?;
        while stream.conn.wants_write() {
            stream.conn.complete_io(&mut stream.sock)?;
        }
    }
    Ok(())
}

fn bad_arguments(error: impl std::fmt::Display) -> RpcStatus {
    let mut message = error.to_string();
    if message.len() > 160 {
        let mut end = 160;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    RpcStatus::BadArguments(message)
}

fn endpoint_host(endpoint: &str) -> Option<&str> {
    if let Some(endpoint) = endpoint.strip_prefix('[') {
        let (host, port) = endpoint.split_once("]:")?;
        (!host.is_empty() && port.parse::<u16>().is_ok()).then_some(host)
    } else {
        let (host, port) = endpoint.rsplit_once(':')?;
        (!host.is_empty() && port.parse::<u16>().is_ok()).then_some(host)
    }
}
