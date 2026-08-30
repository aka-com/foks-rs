use foks_rpc::{encode_success_response_at, encode_void_success_response_at, RpcStatus};

use crate::rpc::{RouteId, RoutedCall};

use super::super::ServerData;

pub(super) trait Operations {
    fn validate_probe(&self, argument: &[u8]) -> Result<(), RpcStatus>;
    fn current_probe_response(&self) -> Result<Vec<u8>, RpcStatus>;
    fn validate_host_argument(&self, argument: &[u8]) -> Result<(), RpcStatus>;
    fn validate_optional_host_argument(&self, argument: &[u8]) -> Result<(), RpcStatus>;
    fn current_root(&self) -> Result<Vec<u8>, RpcStatus>;
    fn current_root_signed(&self) -> Result<Vec<u8>, RpcStatus>;
    fn historical_roots(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn merkle_lookup(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn merkle_multi_lookup(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn current_root_hash(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
    fn merkle_check_key_exists(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
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

    fn validate_optional_host_argument(&self, argument: &[u8]) -> Result<(), RpcStatus> {
        ServerData::validate_optional_host_argument(self, argument)
    }

    fn current_root(&self) -> Result<Vec<u8>, RpcStatus> {
        ServerData::current_root(self)
    }

    fn current_root_signed(&self) -> Result<Vec<u8>, RpcStatus> {
        ServerData::current_root_signed(self)
    }

    fn historical_roots(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        ServerData::historical_roots(self, argument)
    }

    fn merkle_lookup(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        ServerData::merkle_lookup(self, argument)
    }

    fn merkle_multi_lookup(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        ServerData::merkle_multi_lookup(self, argument)
    }

    fn current_root_hash(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        ServerData::current_root_hash(self, argument)
    }

    fn merkle_check_key_exists(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        ServerData::merkle_check_key_exists(self, argument)
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
            operations.validate_optional_host_argument(call.call.argument())?;
            encode_success_response_at(&operations.current_root()?, sequence)
                .map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::MerkleQueryGetCurrentRootSigned => {
            operations.validate_optional_host_argument(call.call.argument())?;
            encode_success_response_at(&operations.current_root_signed()?, sequence)
                .map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::MerkleQueryGetHistoricalRoots => encode_success_response_at(
            &operations.historical_roots(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::MerkleQueryLookup => {
            encode_success_response_at(&operations.merkle_lookup(call.call.argument())?, sequence)
                .map_err(|_| RpcStatus::Unsupported)
        }
        RouteId::MerkleQueryMLookup => encode_success_response_at(
            &operations.merkle_multi_lookup(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::MerkleQueryGetCurrentRootHash => encode_success_response_at(
            &operations.current_root_hash(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        RouteId::MerkleQueryCheckKeyExists => encode_success_response_at(
            &operations.merkle_check_key_exists(call.call.argument())?,
            sequence,
        )
        .map_err(|_| RpcStatus::Unsupported),
        _ => Err(RpcStatus::Unsupported),
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, io::Cursor};

    use super::*;
    use crate::rpc::{route_call, Listener};

    #[derive(Default)]
    struct RecordingOperations {
        strict_host_validations: Cell<usize>,
        optional_host_validations: Cell<usize>,
    }

    impl Operations for RecordingOperations {
        fn validate_probe(&self, _: &[u8]) -> Result<(), RpcStatus> {
            unreachable!()
        }

        fn current_probe_response(&self) -> Result<Vec<u8>, RpcStatus> {
            unreachable!()
        }

        fn validate_host_argument(&self, argument: &[u8]) -> Result<(), RpcStatus> {
            self.strict_host_validations
                .set(self.strict_host_validations.get() + 1);
            super::super::super::validate_host_argument_against(argument, &expected_host(), false)
        }

        fn validate_optional_host_argument(&self, argument: &[u8]) -> Result<(), RpcStatus> {
            self.optional_host_validations
                .set(self.optional_host_validations.get() + 1);
            super::super::super::validate_host_argument_against(argument, &expected_host(), true)
        }

        fn current_root(&self) -> Result<Vec<u8>, RpcStatus> {
            Ok(user_fixture("merkle-root-998.snowp"))
        }

        fn current_root_signed(&self) -> Result<Vec<u8>, RpcStatus> {
            Ok(foks_snowpack::encode(&foks_snowpack::Value::Unsigned(1)).unwrap())
        }

        fn historical_roots(&self, _: &[u8]) -> Result<Vec<u8>, RpcStatus> {
            unreachable!()
        }

        fn merkle_lookup(&self, _: &[u8]) -> Result<Vec<u8>, RpcStatus> {
            unreachable!()
        }

        fn merkle_multi_lookup(&self, _: &[u8]) -> Result<Vec<u8>, RpcStatus> {
            unreachable!()
        }

        fn current_root_hash(&self, _: &[u8]) -> Result<Vec<u8>, RpcStatus> {
            unreachable!()
        }

        fn merkle_check_key_exists(&self, _: &[u8]) -> Result<Vec<u8>, RpcStatus> {
            unreachable!()
        }
    }

    fn expected_host() -> Vec<u8> {
        [vec![foks_proto::ENTITY_HOST], vec![0x41; 32]].concat()
    }

    fn user_fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../foks-snowpack/tests/fixtures/foks-v0.1.9/user")
                .join(name),
        )
        .unwrap()
    }

    fn routed_merkle_call(position: u64, host: foks_snowpack::Value) -> RoutedCall {
        let argument = foks_snowpack::encode(&foks_snowpack::Value::Array(vec![host])).unwrap();
        let framed =
            foks_rpc::encode_call(foks_rpc::MERKLE_QUERY_PROTOCOL_ID, position, &argument, 1)
                .unwrap();
        let call = foks_rpc::read_call(&mut Cursor::new(framed), 4096).unwrap();
        route_call(call, Listener::PublicServices).unwrap()
    }

    #[test]
    fn current_root_routes_use_optional_host_validation_only() {
        let operations = RecordingOperations::default();

        response(
            &operations,
            routed_merkle_call(
                foks_rpc::MERKLE_GET_CURRENT_ROOT_METHOD_POSITION,
                foks_snowpack::Value::Null,
            ),
        )
        .unwrap();
        response(
            &operations,
            routed_merkle_call(
                foks_rpc::MERKLE_GET_CURRENT_ROOT_SIGNED_METHOD_POSITION,
                foks_snowpack::Value::Null,
            ),
        )
        .unwrap();

        assert_eq!(operations.optional_host_validations.get(), 2);
        assert_eq!(operations.strict_host_validations.get(), 0);
    }

    #[test]
    fn unsigned_current_root_response_matches_the_generated_go_frame() {
        let operations = RecordingOperations::default();
        let actual = response(
            &operations,
            routed_merkle_call(
                foks_rpc::MERKLE_GET_CURRENT_ROOT_METHOD_POSITION,
                foks_snowpack::Value::Null,
            ),
        )
        .unwrap();
        assert_eq!(actual, user_fixture("merkle-current-root-response.frame"));
    }

    #[test]
    fn select_vhost_keeps_strict_host_validation() {
        let operations = RecordingOperations::default();

        response(
            &operations,
            routed_merkle_call(
                foks_rpc::MERKLE_SELECT_VHOST_METHOD_POSITION,
                foks_snowpack::Value::Binary(expected_host()),
            ),
        )
        .unwrap();

        assert_eq!(operations.strict_host_validations.get(), 1);
        assert_eq!(operations.optional_host_validations.get(), 0);
    }

    #[test]
    fn selected_vhost_rejects_omission_and_another_listener_host() {
        let operations = RecordingOperations::default();
        assert!(matches!(
            response(
                &operations,
                routed_merkle_call(
                    foks_rpc::MERKLE_SELECT_VHOST_METHOD_POSITION,
                    foks_snowpack::Value::Null,
                ),
            ),
            Err(RpcStatus::BadArguments(_))
        ));

        let mut other_host = expected_host();
        other_host[1] ^= 1;
        assert!(matches!(
            response(
                &operations,
                routed_merkle_call(
                    foks_rpc::MERKLE_SELECT_VHOST_METHOD_POSITION,
                    foks_snowpack::Value::Binary(other_host),
                ),
            ),
            Err(RpcStatus::NotFound(_))
        ));
    }
}
