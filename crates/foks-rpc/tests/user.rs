use foks_rpc::{
    encode_get_client_cert_chain_request, encode_get_current_merkle_root_request,
    encode_get_historical_merkle_roots_request, encode_get_owner_puk_request,
    encode_load_user_chain_request,
};
use foks_snowpack::{decode, Value};

const DIR: &str = "../foks-snowpack/tests/fixtures/foks-v0.1.9/user";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{DIR}/{name}")).unwrap()
}

#[test]
fn merkle_query_requests_match_the_official_go_oracle() {
    assert_eq!(
        encode_get_current_merkle_root_request().unwrap(),
        fixture("merkle-current-root-request.frame")
    );
    assert_eq!(
        encode_get_historical_merkle_roots_request(&[996], &[997, 996, 994, 992]).unwrap(),
        fixture("merkle-historical-roots-request.frame")
    );
}

fn binary_fixture(name: &str) -> Vec<u8> {
    match decode(&fixture(name)).unwrap() {
        Value::Binary(bytes) => bytes,
        other => panic!("expected binary fixture, got {other:?}"),
    }
}

#[test]
fn registration_certificate_request_matches_go_v019() {
    let uid = binary_fixture("uid.snowp");
    let device = binary_fixture("device-id.snowp");
    assert_eq!(
        encode_get_client_cert_chain_request(&uid, &device).unwrap(),
        fixture("reg-cert-request.frame")
    );
}

#[test]
fn user_chain_request_matches_go_v019() {
    let uid = binary_fixture("uid.snowp");
    assert_eq!(
        encode_load_user_chain_request(&uid, 1).unwrap(),
        fixture("user-load-request.frame")
    );
}

#[test]
fn owner_puk_request_matches_go_v019() {
    let device = binary_fixture("device-id.snowp");
    assert_eq!(
        encode_get_owner_puk_request(&device).unwrap(),
        fixture("user-puk-request.frame")
    );
}
