use foks_rpc::{encode_success_response_at, encode_void_success_response_at, RpcStatus};

use crate::rpc::{RouteId, RoutedCall};

use super::super::ServerData;

pub(super) trait Operations {
    fn validate_probe(&self, argument: &[u8]) -> Result<(), RpcStatus>;
    fn current_probe_response(&self) -> Result<Vec<u8>, RpcStatus>;
    fn validate_host_argument(&self, argument: &[u8]) -> Result<(), RpcStatus>;
    fn current_root(&self) -> Result<Vec<u8>, RpcStatus>;
    fn historical_roots(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
}

impl Operations for ServerData {
    fn validate_probe(&self, argument: &[u8]) -> Result<(), RpcStatus> {
        ServerData::validate_probe(self, argument)
    }

    fn current_probe_response(&self) -> Result<Vec<u8>, RpcStatus> {
        ServerData::current_probe_response(self)
    }

    fn validate_host_argument(&self, argument: &[u8]) -> Result<(), RpcStatus> {
        ServerData::validate_host_argument(self, argument)
    }

    fn current_root(&self) -> Result<Vec<u8>, RpcStatus> {
        ServerData::current_root(self)
    }

    fn historical_roots(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        ServerData::historical_roots(self, argument)
    }
}

pub(super) fn response(
    operations: &impl Operations,
    call: RoutedCall,
) -> Result<Vec<u8>, RpcStatus> {
    let sequence = call.call.sequence();
    match call.route.id {
        RouteId::ProbeProbe => {
            operations.validate_probe(call.call.argument())?;
            encode_success_response_at(&operations.current_probe_response()?, sequence)
                .map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::RegSelectVHost | RouteId::MerkleQuerySelectVHost | RouteId::KvStoreSelectVHost => {
            operations.validate_host_argument(call.call.argument())?;
            encode_void_success_response_at(sequence).map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::MerkleQueryGetCurrentRoot => {
            operations.validate_host_argument(call.call.argument())?;
            encode_success_response_at(&operations.current_root()?, sequence)
                .map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::MerkleQueryGetHistoricalRoots => encode_success_response_at(
            &operations.historical_roots(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        _ => Err(RpcStatus::Unsupported),
    }
}
