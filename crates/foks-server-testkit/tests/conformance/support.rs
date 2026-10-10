use foks_client::{PinnedHost, ProbeOutcome};
use foks_server_testkit::{InProcessServer, TestClient, TestEnvironment, TestProfile};

pub(crate) struct Fixture {
    pub(crate) environment: TestEnvironment,
    pub(crate) server: InProcessServer,
    pub(crate) client: TestClient,
    pub(crate) probe: ProbeOutcome,
}

impl Fixture {
    pub(crate) fn start(client_id: &str) -> Self {
        Self::start_with_profile(client_id, TestProfile::Default)
    }

    pub(crate) fn start_with_profile(client_id: &str, profile: TestProfile) -> Self {
        let environment = TestEnvironment::with_profile(profile).unwrap();
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
