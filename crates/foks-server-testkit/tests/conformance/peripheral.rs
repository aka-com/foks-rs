use std::io::Write as _;
use std::sync::Arc;

use foks_rpc::arguments::{LogSendInitFileArgument, LogSendUploadBlockArgument};
use foks_snowpack::{decode, Value};
use rustls::pki_types::{CertificateDer, ServerName};

use crate::support::Fixture;

#[test]
pub(crate) fn go_client_waitlist_and_log_send_work() {
    let fixture = Fixture::start("go-client-peripheral");
    let mut tls = public_stream(&fixture);

    tls.write_all(&foks_rpc::encode_join_waitlist_request_at(b"go-client@example.com", 0).unwrap())
        .unwrap();
    let response =
        foks_rpc::read_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 0).unwrap();
    let Value::Binary(waitlist_id) = decode(&response).unwrap() else {
        panic!("waitlist response was not a binary identifier");
    };
    assert_eq!(waitlist_id.len(), 13);
    assert_eq!(waitlist_id[0], 1);

    tls.write_all(&foks_rpc::encode_log_send_init_request_at(1).unwrap())
        .unwrap();
    let response =
        foks_rpc::read_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 1).unwrap();
    let Value::Binary(log_send_id) = decode(&response).unwrap() else {
        panic!("log-send response was not a binary identifier");
    };
    let log_send_id: [u8; 17] = log_send_id.try_into().unwrap();
    assert_eq!(log_send_id[0], 48);

    let payload = b"official Go-compatible diagnostic payload";
    tls.write_all(
        &foks_rpc::encode_log_send_init_file_request_at(
            &LogSendInitFileArgument {
                id: log_send_id,
                file_id: 7,
                filename: b"client.log".to_vec(),
                content_length: payload.len() as u64,
                content_hash: [0x44; 32],
                block_count: 1,
            },
            2,
        )
        .unwrap(),
    )
    .unwrap();
    foks_rpc::read_void_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 2).unwrap();

    let upload = LogSendUploadBlockArgument {
        id: log_send_id,
        file_id: 7,
        block_number: 0,
        block: payload.to_vec(),
    };
    tls.write_all(&foks_rpc::encode_log_send_upload_block_request_at(&upload, 3).unwrap())
        .unwrap();
    foks_rpc::read_void_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 3).unwrap();

    tls.write_all(&foks_rpc::encode_log_send_upload_block_request_at(&upload, 4).unwrap())
        .unwrap();
    assert!(matches!(
        foks_rpc::read_void_response(&mut tls, foks_rpc::DEFAULT_MAX_FRAME_LENGTH, 4),
        Err(foks_rpc::Error::RemoteStatus { code: 1001, .. })
    ));
}

fn public_stream(
    fixture: &Fixture,
) -> rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream> {
    let mut roots = rustls::RootCertStore::empty();
    for certificate in fixture.host().tls_ca_certificates() {
        roots
            .add(CertificateDer::from(certificate.clone()))
            .unwrap();
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let tcp = std::net::TcpStream::connect(fixture.server.addresses().public_services).unwrap();
    let connection = rustls::ClientConnection::new(
        Arc::new(config),
        ServerName::try_from("localhost".to_owned()).unwrap(),
    )
    .unwrap();
    rustls::StreamOwned::new(connection, tcp)
}
