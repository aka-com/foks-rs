//! Public Beacon discovery: HostID to advertised endpoint.

use foks_rpc::{encode_success_response_at, RpcStatus};

use crate::rpc::{RouteId, RoutedCall};

use super::super::ServerData;

pub(super) trait Operations {
    fn beacon_lookup(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus>;
}

impl Operations for ServerData {
    fn beacon_lookup(&self, argument: &[u8]) -> Result<Vec<u8>, RpcStatus> {
        ServerData::beacon_lookup(self, argument)
    }
}

pub(super) fn response(
    operations: &impl Operations,
    call: RoutedCall,
) -> Result<Vec<u8>, RpcStatus> {
    let sequence = call.call.sequence();
    match call.route.id {
        RouteId::BeaconBeaconLookup => {
            encode_success_response_at(&operations.beacon_lookup(call.call.argument())?, sequence)
                .map_err(|_| RpcStatus::Unsupported)
        }
        _ => Err(RpcStatus::Unsupported),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use foks_proto::EntityId;
    use foks_snowpack::{encode, Value};

    use super::*;
    use crate::rpc::{route_call, Listener, ROUTES};

    struct FixedEndpoint;

    impl Operations for FixedEndpoint {
        fn beacon_lookup(&self, _: &[u8]) -> Result<Vec<u8>, RpcStatus> {
            Ok(encode(&Value::Text(b"localhost:4430".to_vec())).unwrap())
        }
    }

    #[test]
    fn beacon_lookup_is_supported_and_wraps_a_result_header() {
        let route = ROUTES
            .iter()
            .find(|route| route.protocol == "Beacon" && route.method == "beaconLookup")
            .expect("the beacon route is generated");
        assert!(route.supported);
        let argument = encode(&Value::Array(vec![Value::Binary(vec![2; 33])])).unwrap();
        let framed =
            foks_rpc::encode_call(route.protocol_id, route.position, &argument, 0).unwrap();
        let call = foks_rpc::read_call(&mut Cursor::new(framed), 4096).unwrap();
        let routed = route_call(call, Listener::PublicServices).unwrap();
        let framed_response = response(&FixedEndpoint, routed).unwrap();
        let body = foks_rpc::read_response(&mut Cursor::new(framed_response), 4096, 0).unwrap();
        let host =
            EntityId::from_bytes([vec![foks_proto::ENTITY_HOST], vec![1; 32]].concat()).unwrap();
        assert_eq!(
            foks_rpc::decode_beacon_lookup_response(host, &body)
                .unwrap()
                .address,
            "localhost:4430"
        );
    }
}
