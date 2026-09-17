use crate::commands::execution::ambiguous_mutation_response;
use crate::commands::servers::{
    check_existing_or_add_profile, normalized_probe_endpoint, normalized_probe_hostname,
    require_transport_profile, reset_result_response, ProfileSummary, ProfileTrustSummary,
};
use crate::commands::tests::support::{phase_four_state, test_profile, test_profile_value};
use crate::commands::validation::MAXIMUM_FIRST_RUN_ROWS;
use foks_agent_proto::{Operation, ProfileProtocol, ProfileTrust};
use std::path::PathBuf;
use std::sync::Mutex;

pub(super) struct ProfileListTransport {
    pub(super) value: serde_json::Value,
}

impl foks_desktop::AgentTransport for ProfileListTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        assert_eq!(operation, Operation::ListProfiles);
        Ok(self.value.clone())
    }
}

pub(super) struct SetupProfileTransport {
    pub(super) profiles: Vec<ProfileSummary>,
    pub(super) probe_error: Option<String>,
    pub(super) calls: Mutex<Vec<Operation>>,
}

impl SetupProfileTransport {
    pub(super) fn new(profiles: Vec<ProfileSummary>) -> Self {
        Self {
            profiles,
            probe_error: None,
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl foks_desktop::AgentTransport for SetupProfileTransport {
    fn call(&self, operation: Operation) -> Result<serde_json::Value, foks_desktop::AgentError> {
        self.calls.lock().unwrap().push(operation.clone());
        match operation {
            Operation::ListProfiles => serde_json::to_value(&self.profiles)
                .map_err(|error| foks_desktop::AgentError::Transport(error.to_string())),
            Operation::Probe { profile } => {
                if let Some(error) = &self.probe_error {
                    return Err(foks_desktop::AgentError::Protocol {
                        code: foks_agent_proto::ErrorCode::OperationFailed,
                        message: error.clone(),
                        fields: foks_agent_proto::ErrorFields::default(),
                    });
                }
                let configured = self
                    .profiles
                    .iter()
                    .find(|candidate| candidate.name == profile)
                    .expect("target profile should exist in mock transport");
                Ok(serde_json::json!({
                    "acceptance":"unchanged",
                    "lookup_name":normalized_probe_hostname(&configured.probe).unwrap(),
                    "canonical_name":"localhost",
                    "host_id_hex":"02".repeat(33),
                    "host_chain_sequence":5,
                    "merkle_epoch":9
                }))
            }
            Operation::CheckAndAddProfile {
                name,
                probe,
                protocol,
                trust,
            } => {
                assert_eq!(protocol, ProfileProtocol::V019);
                assert_eq!(trust, ProfileTrust::WebPki);
                let lookup_name = normalized_probe_hostname(&probe).unwrap();
                Ok(serde_json::json!({
                    "profile":{
                        "name":name,
                        "probe":probe,
                        "protocol":{"generation":"v019"},
                        "trust":{"kind":"web-pki"}
                    },
                    "probe":{
                        "acceptance":"inserted",
                        "lookup_name":lookup_name,
                        "canonical_name":lookup_name,
                        "host_id_hex":"02".repeat(33),
                        "host_chain_sequence":1,
                        "merkle_epoch":2
                    }
                }))
            }
            other => panic!("unexpected setup profile operation {other:?}"),
        }
    }
}

#[test]
fn setup_reuses_the_one_profile_at_the_normalized_endpoint() {
    let mut local = test_profile("local");
    local.probe = "localhost:4430".to_owned();
    local.trust = ProfileTrustSummary::CertificateDer {
        path: PathBuf::from("/private/foks-dev-ca.der"),
    };
    let transport = SetupProfileTransport::new(vec![local]);

    let checked = check_existing_or_add_profile(
        &transport,
        "setup-localhost".to_owned(),
        "localhost".to_owned(),
        None,
    )
    .unwrap();

    assert_eq!(checked.profile, "local");
    assert_eq!(checked.acceptance, "unchanged");
    assert_eq!(
        transport.calls.lock().unwrap().as_slice(),
        &[
            Operation::ListProfiles,
            Operation::Probe {
                profile: "local".to_owned()
            }
        ]
    );
}

#[test]
fn setup_creates_webpki_only_when_no_endpoint_matches() {
    let mut local = test_profile("local");
    local.probe = "localhost:4430".to_owned();
    local.trust = ProfileTrustSummary::CertificateDer {
        path: PathBuf::from("/private/foks-dev-ca.der"),
    };
    let transport = SetupProfileTransport::new(vec![local]);

    let checked = check_existing_or_add_profile(
        &transport,
        "setup-localhost".to_owned(),
        "localhost:4431".to_owned(),
        None,
    )
    .unwrap();

    assert_eq!(checked.profile, "setup-localhost");
    assert_eq!(checked.acceptance, "inserted");
    assert_eq!(
        transport.calls.lock().unwrap().as_slice(),
        &[
            Operation::ListProfiles,
            Operation::CheckAndAddProfile {
                name: "setup-localhost".to_owned(),
                probe: "localhost:4431".to_owned(),
                protocol: ProfileProtocol::V019,
                trust: ProfileTrust::WebPki,
            }
        ]
    );
}

#[test]
fn setup_fails_closed_for_ambiguous_or_failed_saved_profiles() {
    let mut first = test_profile("first");
    first.probe = "LOCALHOST.".to_owned();
    let mut second = test_profile("second");
    second.probe = "localhost:4430".to_owned();
    let duplicate = SetupProfileTransport::new(vec![first, second]);
    let error = check_existing_or_add_profile(
        &duplicate,
        "setup-localhost".to_owned(),
        "localhost".to_owned(),
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, "profile-conflict");
    assert_eq!(
        duplicate.calls.lock().unwrap().as_slice(),
        &[Operation::ListProfiles]
    );

    let mut local = test_profile("local");
    local.probe = "localhost:4430".to_owned();
    local.trust = ProfileTrustSummary::CertificateDer {
        path: PathBuf::from("/private/foks-dev-ca.der"),
    };
    let failed = SetupProfileTransport {
        probe_error: Some("certificate validation failed for saved profile".to_owned()),
        ..SetupProfileTransport::new(vec![local])
    };
    let error = check_existing_or_add_profile(
        &failed,
        "setup-localhost".to_owned(),
        "localhost".to_owned(),
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, "operation-failed");
    assert_eq!(
        failed.calls.lock().unwrap().as_slice(),
        &[
            Operation::ListProfiles,
            Operation::Probe {
                profile: "local".to_owned()
            }
        ]
    );
}

#[test]
fn setup_endpoint_normalization_keeps_ports_in_the_identity() {
    assert_eq!(
        normalized_probe_endpoint("LOCALHOST."),
        normalized_probe_endpoint("localhost:4430")
    );
    assert_ne!(
        normalized_probe_endpoint("localhost"),
        normalized_probe_endpoint("localhost:4431")
    );
    assert_eq!(
        normalized_probe_endpoint("::1"),
        normalized_probe_endpoint("[::1]:4430")
    );
}

#[test]
fn profile_preflight_is_exact_bounded_and_unique() {
    let real_wire = serde_json::to_value(vec![test_profile("work")]).unwrap();
    assert_eq!(
        real_wire,
        serde_json::json!([{
            "name":"work",
            "probe":"foks.example",
            "protocol":{"generation":"v019"},
            "trust":{"kind":"web-pki"}
        }])
    );
    assert!(require_transport_profile(&ProfileListTransport { value: real_wire }, "work").is_ok());

    let mut unknown = test_profile_value("work");
    unknown
        .as_object_mut()
        .unwrap()
        .insert("invented".to_owned(), serde_json::Value::Bool(true));
    let error = require_transport_profile(
        &ProfileListTransport {
            value: serde_json::Value::Array(vec![unknown]),
        },
        "work",
    )
    .unwrap_err();
    assert_eq!(error.code, "invalid-response");

    assert_eq!(
        require_transport_profile(
            &ProfileListTransport {
                value: serde_json::json!([{"name":"work"}]),
            },
            "work",
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );

    for value in [
        serde_json::Value::Array(vec![test_profile_value("has space")]),
        serde_json::Value::Array(vec![test_profile_value("work"), test_profile_value("work")]),
    ] {
        assert_eq!(
            require_transport_profile(&ProfileListTransport { value }, "work")
                .unwrap_err()
                .code,
            "invalid-response"
        );
    }

    let mut invalid_probe = test_profile("work");
    invalid_probe.probe = "https://foks.example".to_owned();
    assert_eq!(
        require_transport_profile(
            &ProfileListTransport {
                value: serde_json::json!([invalid_probe]),
            },
            "work",
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );

    let mut invalid_trust = test_profile("work");
    invalid_trust.trust = ProfileTrustSummary::CertificateDer {
        path: PathBuf::new(),
    };
    assert_eq!(
        require_transport_profile(
            &ProfileListTransport {
                value: serde_json::json!([invalid_trust]),
            },
            "work",
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );

    let mut unknown_protocol = test_profile_value("work");
    unknown_protocol["protocol"]
        .as_object_mut()
        .unwrap()
        .insert("invented".to_owned(), serde_json::Value::Bool(true));
    assert_eq!(
        require_transport_profile(
            &ProfileListTransport {
                value: serde_json::Value::Array(vec![unknown_protocol]),
            },
            "work",
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );

    let too_many = (0..=MAXIMUM_FIRST_RUN_ROWS)
        .map(|index| test_profile_value(format!("p{index}")))
        .collect::<Vec<_>>();
    assert_eq!(
        require_transport_profile(
            &ProfileListTransport {
                value: serde_json::Value::Array(too_many),
            },
            "work",
        )
        .unwrap_err()
        .code,
        "invalid-response"
    );
}

#[test]
fn malformed_reset_success_is_post_mutation_ambiguity() {
    let state = phase_four_state(Vec::new());
    let value = serde_json::json!({"profile":"other","hard_state_reset":true});
    let error = reset_result_response(value, "work")
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
        .unwrap_err();
    assert_eq!(error.code, "response-binding");
    assert!(error.ambiguous && error.fatal);
    assert_eq!(state.begin_mutation().unwrap_err().code, "ambiguous");
}
