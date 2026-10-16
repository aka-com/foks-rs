use std::time::{Duration, Instant};

use foks_server_testkit::{TestAccountSpec, TestClient, TestEnvironment};

#[test]
fn an_expired_caller_cannot_refresh_its_budget_or_expire_an_independent_pool_borrower() {
    let environment = TestEnvironment::new().unwrap();
    let server = environment.start_server().unwrap();
    let client = TestClient::new(&environment, "caller-deadline").unwrap();
    let host = client.probe_and_pin().unwrap().pinned;
    let account = client
        .create_account(&host, &TestAccountSpec::new("callerdeadline", 0x39))
        .unwrap();
    client
        .foks()
        .authenticate_and_pin(&host, &account.credential)
        .unwrap();

    let expired = client
        .foks()
        .with_operation_deadline(Instant::now() - Duration::from_secs(1));
    assert!(client.foks().shares_connection_pool_with(&expired));
    let mut retried = expired.with_operation_deadline(Instant::now() + Duration::from_secs(120));
    retried.set_timeout(Duration::from_secs(120));
    assert!(matches!(
        retried.authenticate_and_pin(&host, &account.credential),
        Err(foks_client::Error::DeadlineExceeded)
    ));

    client
        .foks()
        .authenticate_and_pin(&host, &account.credential)
        .unwrap();
    let next = client
        .foks()
        .with_operation_deadline(Instant::now() + Duration::from_secs(15));
    next.authenticate_and_pin(&host, &account.credential)
        .unwrap();
    server.shutdown().unwrap();
}
