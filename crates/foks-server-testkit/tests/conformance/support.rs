use foks_client::{PinnedHost, ProbeOutcome};
use foks_server_testkit::{InProcessServer, TestClient, TestEnvironment};

pub(crate) struct Fixture {
    pub(crate) environment: TestEnvironment,
    pub(crate) server: InProcessServer,
    pub(crate) client: TestClient,
    pub(crate) probe: ProbeOutcome,
}

impl Fixture {
    pub(crate) fn start(client_id: &str) -> Self {
        let environment = TestEnvironment::new().unwrap();
        let server = environment.start_server().unwrap();
        let client = TestClient::new(&environment, client_id).unwrap();
        let probe = client.probe_and_pin().unwrap();
        Self {
            environment,
            server,
            client,
            probe,
        }
    }

    pub(crate) fn host(&self) -> &PinnedHost {
        &self.probe.pinned
    }
}
