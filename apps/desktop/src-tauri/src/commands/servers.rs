//! Server profiles, trust verification, status, and local reset.

use crate::agent::AgentError;
use crate::commands::context::AppState;
use crate::commands::enrollment::{
    validated_pending_dtos, PendingOperationDto, PendingOperationResponse,
};
use crate::commands::execution::{
    ambiguous_mutation_response, ambiguous_worker_failure, apply_operation_value,
    apply_profile_operation_value, apply_profile_operation_with_profile, map_mutation_error,
    read_profile_operation_value, MutationKind,
};
use crate::commands::types::MutationDto;
use crate::commands::validation::{
    bounded_field, bounded_local_name, bounded_secret, deserialize_secret,
    exact_profile_confirmation, invalid_request, invalid_response, require_main_window,
    require_nested_response_row_cap, require_response_row_cap, serialize_secret, valid_local_name,
    valid_response_text, valid_typed_entity_id_hex, HOST_ID_PREFIX,
};
use foks_agent_proto::{Operation, PendingOperationSummary, ProfileProtocol, ProfileTrust};
use foks_desktop::CatalogStoreSummary;
use foks_protocol_metadata::PINNED_PROTOCOL_METADATA_SHA256;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use tauri::State;
use zeroize::Zeroizing;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckedProfileDto {
    pub profile: String,
    pub acceptance: String,
    pub lookup_name: String,
    pub canonical_name: String,
    pub host_id: String,
    pub chain: u64,
    pub epoch: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CheckedProfileResponse {
    pub(super) profile: CheckedProfileIdentity,
    pub(super) probe: CheckedProbeResponse,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CheckedProfileIdentity {
    pub(super) name: String,
    pub(super) probe: String,
    pub(super) protocol: serde_json::Value,
    pub(super) trust: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CheckedProbeResponse {
    pub(super) acceptance: String,
    pub(super) lookup_name: String,
    pub(super) canonical_name: String,
    pub(super) host_id_hex: String,
    pub(super) host_chain_sequence: u64,
    pub(super) merkle_epoch: u64,
    #[serde(default)]
    pub(super) server_version: Option<CheckedServerVersionResponse>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CheckedServerVersionResponse {
    pub(super) minimum: Option<String>,
    pub(super) newest: Option<String>,
    pub(super) message: String,
    pub(super) compatible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckedServerVersionDto {
    pub minimum: Option<String>,
    pub newest: Option<String>,
    pub message: String,
    pub compatible: bool,
}

fn checked_server_version(
    version: Option<CheckedServerVersionResponse>,
) -> Result<Option<CheckedServerVersionDto>, AgentError> {
    let Some(version) = version else {
        return Ok(None);
    };
    // The message is often empty on a compatible server, so it is only bounded
    // and control-character checked rather than required to be non-empty.
    if version
        .minimum
        .as_deref()
        .is_some_and(|value| !valid_response_text(value, 64))
        || version
            .newest
            .as_deref()
            .is_some_and(|value| !valid_response_text(value, 64))
        || version.message.len() > 1024
        || version.message.contains(['\0', '\r', '\n'])
    {
        return Err(invalid_response(
            "The agent returned an invalid server version response.",
        ));
    }
    Ok(Some(CheckedServerVersionDto {
        minimum: version.minimum,
        newest: version.newest,
        message: version.message,
        compatible: version.compatible,
    }))
}

impl CheckedProfileDto {
    pub(super) fn from_response(
        expected_profile: &str,
        expected_probe: &str,
        response: CheckedProfileResponse,
    ) -> Result<Self, AgentError> {
        let probe = response.probe;
        if response.profile.name != expected_profile
            || response.profile.probe != expected_probe
            || response.profile.protocol != serde_json::json!({"generation": "v019"})
            || response.profile.trust != serde_json::json!({"kind": "web-pki"})
            || probe.acceptance != "inserted"
            || normalized_probe_hostname(expected_probe).as_deref()
                != Some(probe.lookup_name.as_str())
            || !valid_response_text(&probe.canonical_name, 256)
            || !valid_typed_entity_id_hex(&probe.host_id_hex, HOST_ID_PREFIX)
        {
            return Err(invalid_response(
                "The agent returned an invalid server verification response.",
            ));
        }
        Ok(Self {
            profile: response.profile.name,
            acceptance: probe.acceptance,
            lookup_name: probe.lookup_name,
            canonical_name: probe.canonical_name,
            host_id: probe.host_id_hex,
            chain: probe.host_chain_sequence,
            epoch: probe.merkle_epoch,
        })
    }

    pub(super) fn from_existing_probe(
        requested_probe: &str,
        profile: &ProfileSummary,
        value: serde_json::Value,
    ) -> Result<Self, AgentError> {
        let checked = checked_server_response(value, &profile.name)?;
        if normalized_probe_endpoint(&profile.probe) != normalized_probe_endpoint(requested_probe)
            || normalized_probe_hostname(requested_probe).as_deref()
                != Some(checked.lookup_name.as_str())
        {
            return Err(invalid_response(
                "The agent returned server verification details for a different address.",
            ));
        }
        Ok(Self {
            profile: checked.profile,
            acceptance: checked.acceptance,
            lookup_name: checked.lookup_name,
            canonical_name: checked.canonical_name,
            host_id: checked.host_id,
            chain: checked.chain,
            epoch: checked.epoch,
        })
    }
}

pub(super) fn checked_server_response(
    value: serde_json::Value,
    expected_profile: &str,
) -> Result<CheckedServerDto, AgentError> {
    let report: CheckedProbeResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if !matches!(
        report.acceptance.as_str(),
        "inserted" | "advanced" | "unchanged"
    ) || !valid_response_text(&report.lookup_name, 256)
        || !valid_response_text(&report.canonical_name, 256)
        || !valid_typed_entity_id_hex(&report.host_id_hex, HOST_ID_PREFIX)
    {
        return Err(invalid_response(
            "The agent returned an invalid server verification response.",
        ));
    }
    let server_version = checked_server_version(report.server_version)?;
    Ok(CheckedServerDto {
        profile: expected_profile.to_owned(),
        acceptance: report.acceptance,
        lookup_name: report.lookup_name,
        canonical_name: report.canonical_name,
        host_id: report.host_id_hex,
        chain: report.host_chain_sequence,
        epoch: report.merkle_epoch,
        server_version,
    })
}

pub(super) fn added_server_response(
    value: serde_json::Value,
    expected_profile: &str,
    expected_probe: &str,
) -> Result<AddedServerDto, AgentError> {
    let profile: ProfileSummary = serde_json::from_value(value.clone())
        .map_err(|error| invalid_response(error.to_string()))?;
    let canonical =
        serde_json::to_value(&profile).map_err(|error| invalid_response(error.to_string()))?;
    if canonical != value
        || profile.name != expected_profile
        || profile.probe != expected_probe
        || !valid_profile_summary(&profile)
        || profile.protocol != ProfileProtocolSummary::V019
        || profile.trust != ProfileTrustSummary::WebPki
    {
        return Err(invalid_response(
            "The agent returned an invalid server profile record.",
        ));
    }
    Ok(AddedServerDto {
        profile: profile.name,
        configured_probe: profile.probe,
    })
}

pub(super) fn forgotten_server_response(
    value: serde_json::Value,
    expected_profile: &str,
) -> Result<ForgottenServerDto, AgentError> {
    let response: RemovedProfileResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.profile != expected_profile || !response.removed {
        return Err(invalid_response(
            "Failed to confirm removal of the selected server profile.",
        ));
    }
    Ok(ForgottenServerDto {
        profile: response.profile,
        removed: response.removed,
    })
}

pub(super) fn server_label_response(
    value: serde_json::Value,
    expected_profile: &str,
    expected_label: &Option<String>,
) -> Result<ServerLabelDto, AgentError> {
    let response: ProfileLabelResponse = serde_json::from_value(value.clone())
        .map_err(|error| invalid_response(error.to_string()))?;
    let canonical =
        serde_json::to_value(&response).map_err(|error| invalid_response(error.to_string()))?;
    if canonical != value
        || response.profile != expected_profile
        || &response.label != expected_label
    {
        return Err(invalid_response(
            "The agent returned a label result for a different server profile.",
        ));
    }
    Ok(ServerLabelDto {
        profile: response.profile,
        label: response.label,
        changed: response.changed,
    })
}

pub(super) fn server_status_response(
    value: serde_json::Value,
    expected_profile: &str,
    expected_probe: &str,
    expected_lease_required: bool,
) -> Result<ServerStatusSnapshotDto, AgentError> {
    let report: ServerStatusResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if report.profile != expected_profile
        || report.configured_probe != expected_probe
        || report.lease_required != expected_lease_required
        || (!report.lease_required && report.lease_expires_at.is_some())
        || !valid_probe_target(&report.configured_probe)
    {
        return Err(invalid_response(
            "The agent returned an invalid server status response.",
        ));
    }
    let expected_lookup = normalized_probe_hostname(expected_probe)
        .ok_or_else(|| invalid_response("The configured server address is invalid."))?;
    let host = report
        .host
        .map(|host| {
            if host.lookup_name != expected_lookup
                || !valid_response_text(&host.canonical_name, 256)
                || !valid_typed_entity_id_hex(&host.host_id_hex, HOST_ID_PREFIX)
            {
                return Err(invalid_response(
                    "The agent returned invalid host information.",
                ));
            }
            Ok(StoredHostDto {
                lookup_name: host.lookup_name,
                canonical_name: host.canonical_name,
                host_id: host.host_id_hex,
                chain: host.host_chain_sequence,
                epoch: host.merkle_epoch,
            })
        })
        .transpose()?;
    Ok(ServerStatusSnapshotDto {
        profile: report.profile,
        configured_probe: report.configured_probe,
        host,
        lease_required: report.lease_required,
        lease_expires_at: report.lease_expires_at,
        chat_available: report.chat_available,
    })
}

pub(super) fn reset_preview_response(
    value: serde_json::Value,
    expected_profile: &str,
) -> Result<ResetPreviewDto, AgentError> {
    require_nested_response_row_cap(&value, "resumables", "reset resumables")?;
    require_nested_response_row_cap(&value, "artifacts", "reset artifacts")?;
    let response: ResetPreviewResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.profile != expected_profile
        || response.artifacts.len() > 16
        || response.token.is_empty()
        || response.token.len() > 1024
        || response.token.contains(['\0', '\r', '\n'])
        || response.expires_in_seconds == 0
        || response.expires_in_seconds > 24 * 60 * 60
    {
        return Err(invalid_response("The server reset preview is invalid."));
    }
    let pending = response
        .resumables
        .into_iter()
        .map(Into::into)
        .collect::<Vec<PendingOperationSummary>>();
    let resumables = validated_pending_dtos(&pending)?;
    let mut artifacts: Vec<ResetArtifactDto> = Vec::new();
    for artifact in response.artifacts {
        let kind = match artifact.kind {
            foks_agent_proto::ResetArtifactKind::HardState => "hard-state",
            foks_agent_proto::ResetArtifactKind::SoftState => "soft-state",
            foks_agent_proto::ResetArtifactKind::ProtectedMutations => "protected-mutations",
            foks_agent_proto::ResetArtifactKind::CredentialsAndResumables => {
                "credentials-and-resumables"
            }
            foks_agent_proto::ResetArtifactKind::ExternalRollbackCheckpoint => {
                "external-rollback-checkpoint"
            }
            foks_agent_proto::ResetArtifactKind::ExternalDatabaseClaim => "external-database-claim",
            foks_agent_proto::ResetArtifactKind::ExternalImportReadiness => {
                "external-import-readiness"
            }
            foks_agent_proto::ResetArtifactKind::ExternalPublicationAuthorization => {
                "external-publication-authorization"
            }
        };
        if let Some(aggregate) = artifacts.iter_mut().find(|entry| entry.kind == kind) {
            aggregate.entries =
                aggregate
                    .entries
                    .checked_add(artifact.entries)
                    .ok_or_else(|| {
                        invalid_response("Reset artifact totals exceeded maximum supported values.")
                    })?;
            aggregate.bytes = aggregate.bytes.checked_add(artifact.bytes).ok_or_else(|| {
                invalid_response("The agent returned overflowing reset artifact totals.")
            })?;
        } else {
            artifacts.push(ResetArtifactDto {
                kind,
                entries: artifact.entries,
                bytes: artifact.bytes,
            });
        }
    }
    Ok(ResetPreviewDto {
        profile: response.profile,
        resumables,
        artifacts,
        token: response.token,
        expires_in_seconds: response.expires_in_seconds,
    })
}

pub(super) fn reset_result_response(
    value: serde_json::Value,
    expected_profile: &str,
) -> Result<MutationDto, AgentError> {
    let response: ResetResultResponse =
        serde_json::from_value(value).map_err(|error| invalid_response(error.to_string()))?;
    if response.profile != expected_profile || !response.hard_state_reset {
        return Err(invalid_response(
            "The agent did not confirm reset completion for the selected server.",
        ));
    }
    Ok(MutationDto { applied: true })
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileSummary {
    pub(super) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) label: Option<String>,
    pub(super) probe: String,
    pub(super) protocol: ProfileProtocolSummary,
    pub(super) trust: ProfileTrustSummary,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "generation", rename_all = "kebab-case", deny_unknown_fields)]
pub(super) enum ProfileProtocolSummary {
    V019,
    CurrentProbeOnly {
        canary_public_key: String,
        lease_url: String,
        last_artifact: Option<Box<foks_compat_artifact::SignedCanaryArtifact>>,
    },
    CurrentValidated {
        canary_public_key: String,
        lease_url: String,
        artifact: Box<foks_compat_artifact::SignedCanaryArtifact>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub(super) enum ProfileTrustSummary {
    WebPki,
    CertificateDer { path: PathBuf },
}

#[derive(Debug, Serialize)]
pub struct ServerDto {
    pub id: String,
    pub name: String,
    pub label: Option<String>,
    pub configured_probe: String,
    pub host_id: Option<String>,
    pub chain: Option<u64>,
    pub epoch: Option<u64>,
    pub lease: Option<serde_json::Value>,
    pub accounts: Vec<String>,
    pub state: &'static str,
    pub chat_available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedServerDto {
    pub profile: String,
    pub configured_probe: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerLabelDto {
    pub profile: String,
    pub label: Option<String>,
    pub changed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredHostDto {
    pub lookup_name: String,
    pub canonical_name: String,
    pub host_id: String,
    pub chain: u64,
    pub epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatusSnapshotDto {
    pub profile: String,
    pub configured_probe: String,
    pub host: Option<StoredHostDto>,
    pub lease_required: bool,
    pub lease_expires_at: Option<u64>,
    pub chat_available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckedServerDto {
    pub profile: String,
    pub acceptance: String,
    pub lookup_name: String,
    pub canonical_name: String,
    pub host_id: String,
    pub chain: u64,
    pub epoch: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<CheckedServerVersionDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgottenServerDto {
    pub profile: String,
    pub removed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetArtifactDto {
    pub kind: &'static str,
    pub entries: u64,
    pub bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPreviewDto {
    pub profile: String,
    pub resumables: Vec<PendingOperationDto>,
    pub artifacts: Vec<ResetArtifactDto>,
    #[serde(serialize_with = "serialize_secret")]
    pub token: Zeroizing<String>,
    pub expires_in_seconds: u64,
}

impl std::fmt::Debug for ResetPreviewDto {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResetPreviewDto")
            .field("profile", &self.profile)
            .field("resumables", &self.resumables)
            .field("artifacts", &self.artifacts)
            .field("token", &"[REDACTED]")
            .field("expires_in_seconds", &self.expires_in_seconds)
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerStatusResponse {
    profile: String,
    configured_probe: String,
    host: Option<StoredHostResponse>,
    lease_required: bool,
    lease_expires_at: Option<u64>,
    chat_available: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredHostResponse {
    lookup_name: String,
    canonical_name: String,
    host_id_hex: String,
    host_chain_sequence: u64,
    merkle_epoch: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemovedProfileResponse {
    profile: String,
    removed: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProfileLabelResponse {
    profile: String,
    label: Option<String>,
    changed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetPreviewResponse {
    profile: String,
    resumables: Vec<PendingOperationResponse>,
    artifacts: Vec<ResetArtifactResponse>,
    #[serde(deserialize_with = "deserialize_secret")]
    token: Zeroizing<String>,
    expires_in_seconds: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetArtifactResponse {
    kind: foks_agent_proto::ResetArtifactKind,
    entries: u64,
    bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetResultResponse {
    profile: String,
    hard_state_reset: bool,
}

pub(super) fn require_transport_profile(
    transport: &dyn foks_desktop::AgentTransport,
    expected: &str,
) -> Result<(), AgentError> {
    transport_profile(transport, expected).map(|_| ())
}

pub(super) fn transport_profile(
    transport: &dyn foks_desktop::AgentTransport,
    expected: &str,
) -> Result<ProfileSummary, AgentError> {
    let value = transport
        .call(Operation::ListProfiles)
        .map_err(AgentError::from_desktop)?;
    let profiles = profile_summaries(value)?;
    profiles
        .into_iter()
        .find(|profile| profile.name == expected)
        .ok_or_else(|| {
            AgentError::new(
                "profile-not-found",
                "The requested server profile was not found. Refresh the server list.",
                false,
            )
        })
}

fn valid_profile_summary(profile: &ProfileSummary) -> bool {
    valid_local_name(&profile.name)
        && foks_client_app::normalize_profile_label(&profile.name, profile.label.clone())
            .is_ok_and(|label| label == profile.label)
        && valid_probe_target(&profile.probe)
        && match &profile.trust {
            ProfileTrustSummary::WebPki => true,
            ProfileTrustSummary::CertificateDer { path } => !path.as_os_str().is_empty(),
        }
        && match &profile.protocol {
            ProfileProtocolSummary::V019 => true,
            ProfileProtocolSummary::CurrentProbeOnly {
                canary_public_key,
                lease_url,
                last_artifact,
            } => {
                valid_current_profile_policy(
                    canary_public_key,
                    lease_url,
                    last_artifact.as_deref(),
                    &profile.probe,
                ) && last_artifact
                    .as_deref()
                    .is_none_or(|artifact| !canary_grants_desktop(artifact))
            }
            ProfileProtocolSummary::CurrentValidated {
                canary_public_key,
                lease_url,
                artifact,
            } => {
                canary_grants_desktop(artifact)
                    && valid_current_profile_policy(
                        canary_public_key,
                        lease_url,
                        Some(artifact),
                        &profile.probe,
                    )
            }
        }
}

fn canary_grants_desktop(artifact: &foks_compat_artifact::SignedCanaryArtifact) -> bool {
    artifact.artifact.outcome == foks_compat_artifact::Outcome::Compatible
        && artifact.artifact.protocol_metadata_sha256 == PINNED_PROTOCOL_METADATA_SHA256
        && artifact.artifact.capabilities.iter().all(|capability| {
            matches!(
                capability.as_str(),
                "signup"
                    | "user-sync"
                    | "kv"
                    | "device-administration"
                    | "recovery"
                    | "passphrases"
                    | "teams"
                    | "chat"
                    | "federation"
            )
        })
}

fn valid_current_profile_policy(
    public_key: &str,
    lease_url: &str,
    artifact: Option<&foks_compat_artifact::SignedCanaryArtifact>,
    probe: &str,
) -> bool {
    let Ok(public_key) = foks_compat_artifact::decode_public_key(public_key) else {
        return false;
    };
    if !valid_lease_url(lease_url) {
        return false;
    }
    artifact.is_none_or(|artifact| {
        artifact.artifact.target == probe && artifact.verify(&public_key).is_ok()
    })
}

fn valid_lease_url(value: &str) -> bool {
    if value.is_empty() || value.len() > 2 * 1024 {
        return false;
    }
    let Ok(parsed) = url::Url::parse(value) else {
        return false;
    };
    parsed.scheme() == "https"
        && !parsed.cannot_be_a_base()
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
}

pub(super) fn valid_probe_target(input: &str) -> bool {
    normalized_probe_endpoint(input).is_some()
}

pub(super) fn normalized_probe_hostname(input: &str) -> Option<String> {
    normalized_probe_endpoint(input).map(|(hostname, _)| hostname)
}

pub(super) fn normalized_probe_endpoint(input: &str) -> Option<(String, u16)> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let (hostname, port, is_ipv6) = if let Some(bracketed) = input.strip_prefix('[') {
        let (hostname, suffix) = bracketed.split_once(']')?;
        let port = match suffix {
            "" => 4430,
            suffix => suffix
                .strip_prefix(':')
                .and_then(|port| port.parse::<u16>().ok())?,
        };
        let Ok(hostname) = hostname.parse::<std::net::Ipv6Addr>() else {
            return None;
        };
        (hostname.to_string(), port, true)
    } else if let Ok(address) = input.parse::<std::net::Ipv6Addr>() {
        (address.to_string(), 4430, true)
    } else {
        let (hostname, port) = match input.rsplit_once(':') {
            Some((hostname, port)) if !hostname.contains(':') => {
                let Ok(port) = port.parse::<u16>() else {
                    return None;
                };
                (hostname, port)
            }
            Some(_) => return None,
            None => (input, 4430),
        };
        (
            hostname.trim_end_matches('.').to_ascii_lowercase(),
            port,
            false,
        )
    };
    if port == 0 {
        return None;
    }
    let valid = is_ipv6
        || (!hostname.is_empty()
            && hostname.is_ascii()
            && hostname.len() <= 253
            && hostname.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            }));
    valid.then_some((hostname, port))
}

fn profile_summaries(value: serde_json::Value) -> Result<Vec<ProfileSummary>, AgentError> {
    require_response_row_cap(&value, "server profiles")?;
    let rows = value
        .as_array()
        .expect("the row cap accepted only a JSON array");
    let profiles = rows
        .iter()
        .map(|row| {
            let profile: ProfileSummary = serde_json::from_value(row.clone())
                .map_err(|error| invalid_response(error.to_string()))?;
            let canonical = serde_json::to_value(&profile)
                .map_err(|error| invalid_response(error.to_string()))?;
            if canonical != *row || !valid_profile_summary(&profile) {
                return Err(invalid_response(
                    "The agent returned an invalid server profile.",
                ));
            }
            Ok(profile)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut names = std::collections::HashSet::new();
    if profiles
        .iter()
        .any(|profile| !valid_local_name(&profile.name) || !names.insert(profile.name.clone()))
    {
        return Err(invalid_response(
            "The agent returned duplicate or invalid server profiles.",
        ));
    }
    Ok(profiles)
}

fn checked_profile_response_error(message: impl Into<String>) -> AgentError {
    let mut error = AgentError::new("response-binding", message, false);
    error.ambiguous = true;
    error.fatal = true;
    error
}

pub(super) fn check_existing_or_add_profile(
    transport: &dyn foks_desktop::AgentTransport,
    profile_name: String,
    probe: String,
    go_candidate: Option<(String, String)>,
) -> Result<CheckedProfileDto, AgentError> {
    let requested_endpoint = normalized_probe_endpoint(&probe)
        .ok_or_else(|| invalid_request("Enter a valid DNS name, IP address, or host and port."))?;
    let profiles = profile_summaries(
        transport
            .call(Operation::ListProfiles)
            .map_err(AgentError::from_desktop)?,
    )?;
    let mut matching = profiles.into_iter().filter(|profile| {
        normalized_probe_endpoint(&profile.probe) == Some(requested_endpoint.clone())
    });
    let existing = matching.next();
    if matching.next().is_some() {
        return Err(AgentError::new(
            "profile-conflict",
            "Multiple saved server profiles use this address. Remove the duplicate before continuing setup.",
            false,
        ));
    }
    if let Some(profile) = existing {
        let value = transport
            .call(Operation::Probe {
                profile: profile.name.clone(),
            })
            .map_err(|error| map_mutation_error(error, MutationKind::Guarded))?;
        let checked = CheckedProfileDto::from_existing_probe(&probe, &profile, value)
            .map_err(|error| checked_profile_response_error(error.message))?;
        if go_candidate
            .as_ref()
            .is_some_and(|(_, host)| checked.host_id != *host)
        {
            return Err(invalid_response(
                "The saved server does not match the imported profile.",
            ));
        }
        return Ok(checked);
    }

    let expected_profile = profile_name.clone();
    let expected_probe = probe.clone();
    let operation = match go_candidate {
        Some((candidate_id, _)) => Operation::CheckAndAddGoProfile {
            candidate_id,
            name: profile_name,
            probe,
            protocol: ProfileProtocol::V019,
            trust: ProfileTrust::WebPki,
        },
        None => Operation::CheckAndAddProfile {
            name: profile_name,
            probe,
            protocol: ProfileProtocol::V019,
            trust: ProfileTrust::WebPki,
        },
    };
    let value = transport
        .call(operation)
        .map_err(|error| map_mutation_error(error, MutationKind::Create))?;
    let response: CheckedProfileResponse = serde_json::from_value(value).map_err(|error| {
        checked_profile_response_error(format!("Invalid checked-server response: {error}"))
    })?;
    CheckedProfileDto::from_response(&expected_profile, &expected_probe, response)
        .map_err(|error| checked_profile_response_error(error.message))
}

#[tauri::command]
pub async fn check_and_add_profile(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile_name: String,
    probe: String,
) -> Result<CheckedProfileDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile_name = bounded_local_name(
        &profile_name,
        "Server profile name must be 1 to 64 letters, numbers, hyphens, or underscores.",
    )?;
    let probe = bounded_field(
        &probe,
        2 * 1024,
        "Server address must be 2,048 bytes or fewer.",
    )?;
    if !valid_probe_target(&probe) {
        return Err(invalid_request(
            "Enter a valid DNS name, IP address, or host and port.",
        ));
    }
    state.invalidate_catalog();
    let transport = state.agent.transport();
    let result = tauri::async_runtime::spawn_blocking(move || {
        check_existing_or_add_profile(transport.as_ref(), profile_name, probe, None)
    })
    .await
    .map_err(|error| {
        ambiguous_worker_failure(
            &state,
            format!("Server verification was interrupted before completion: {error}"),
        )
    })?;
    match result {
        Ok(checked) => Ok(checked),
        Err(error) => {
            if error.ambiguous {
                state
                    .mutation_requires_refresh
                    .store(true, Ordering::Release);
            }
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn add_server(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile_name: String,
    probe: String,
) -> Result<AddedServerDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile_name = bounded_local_name(
        &profile_name,
        "Server profile name must be 1 to 64 letters, numbers, hyphens, or underscores.",
    )?;
    let probe = bounded_field(
        &probe,
        2 * 1024,
        "Server address must be 2,048 bytes or fewer.",
    )?;
    if !valid_probe_target(&probe) {
        return Err(invalid_request(
            "Enter a valid DNS name, IP address, or host and port.",
        ));
    }
    let expected_profile = profile_name.clone();
    let expected_probe = probe.clone();
    let value = apply_operation_value(
        &state,
        Operation::AddProfile {
            name: profile_name,
            probe,
            protocol: ProfileProtocol::V019,
            trust: ProfileTrust::WebPki,
        },
        MutationKind::Create,
    )
    .await?;
    added_server_response(value, &expected_profile, &expected_probe)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn set_server_label(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    label: Option<String>,
) -> Result<ServerLabelDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let label = foks_client_app::normalize_profile_label(&profile, label).map_err(|_| {
        invalid_request(
            "Display name must be 64 UTF-8 bytes or fewer and cannot contain NUL or line breaks.",
        )
    })?;
    let expected_profile = profile.clone();
    let expected_label = label.clone();
    let value = apply_operation_value(
        &state,
        Operation::SetProfileLabel { profile, label },
        MutationKind::Guarded,
    )
    .await?;
    server_label_response(value, &expected_profile, &expected_label)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn forget_server(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    confirmation: String,
) -> Result<ForgottenServerDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    exact_profile_confirmation(&profile, &confirmation, "forget")?;
    state.ensure_profile_available(&profile)?;
    let expected = profile.clone();
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::RemoveProfile { name: profile },
        MutationKind::Guarded,
    )
    .await?;
    forgotten_server_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))
}

#[tauri::command]
pub async fn describe_server_status(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    fresh: Option<bool>,
) -> Result<ServerStatusSnapshotDto, AgentError> {
    require_main_window(&webview)?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let expected = profile.clone();
    let cached = state
        .catalog
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .filter(|_| !fresh.unwrap_or(false))
        .and_then(|catalog| {
            catalog
                .profile_overviews
                .iter()
                .find(|overview| overview.profile == profile)
                .map(|overview| overview.server_status.clone())
        });
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || {
        let configured = transport_profile(transport.as_ref(), &profile)?;
        let value = match cached {
            Some(foks_agent_proto::ResponseResult::Success { value }) => value,
            Some(foks_agent_proto::ResponseResult::Error {
                code,
                message,
                fields,
            }) => {
                return Err(AgentError::from_desktop(
                    foks_desktop::AgentError::Protocol {
                        code,
                        message,
                        fields,
                    },
                ));
            }
            None => transport
                .call(Operation::DescribeServerStatus {
                    profile: profile.clone(),
                })
                .map_err(AgentError::from_desktop)?,
        };
        let lease_required = !matches!(configured.protocol, ProfileProtocolSummary::V019);
        server_status_response(value, &expected, &configured.probe, lease_required)
    })
    .await
    .map_err(|error| AgentError::unknown(format!("Failed to load server status: {error}")))?
}

#[tauri::command]
pub async fn check_server(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
) -> Result<CheckedServerDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let state = state.for_profile(&profile)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let expected = profile.clone();
    let (value, configured) = apply_profile_operation_with_profile(
        &state,
        profile.clone(),
        Operation::Probe { profile },
        MutationKind::Guarded,
    )
    .await?;
    let checked = checked_server_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))?;
    let expected_lookup = normalized_probe_hostname(&configured.probe).ok_or_else(|| {
        ambiguous_mutation_response(
            &state,
            "The selected server profile has an invalid server address.",
        )
    })?;
    if checked.lookup_name != expected_lookup {
        return Err(ambiguous_mutation_response(
            &state,
            "The server check returned results for a different lookup name.",
        ));
    }
    Ok(checked)
}

#[tauri::command]
pub async fn describe_reset(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
) -> Result<ResetPreviewDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    // Serialize preview issuance with mutations so the one-use token
    // describes a stable state.
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    let expected = profile.clone();
    let value = read_profile_operation_value(
        &state,
        profile.clone(),
        Operation::DescribeResetHardState { profile },
    )
    .await?;
    reset_preview_response(value, &expected)
}

#[tauri::command]
pub async fn reset_server(
    app: tauri::AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    profile: String,
    confirmation: String,
    token: String,
) -> Result<MutationDto, AgentError> {
    require_main_window(&webview)?;
    crate::applock::require_unlocked(&app)?;
    let _mutation = state.begin_mutation()?;
    let profile = bounded_local_name(&profile, "Provide a valid server profile name.")?;
    exact_profile_confirmation(&profile, &confirmation, "reset")?;
    let token = bounded_secret(
        token,
        1024,
        "The reset preview confirmation token is missing or invalid. Preview the reset again.",
    )?;
    let expected = profile.clone();
    let value = apply_profile_operation_value(
        &state,
        profile.clone(),
        Operation::ResetHardState { profile, token },
        MutationKind::Guarded,
    )
    .await?;
    let result = reset_result_response(value, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))?;
    super::chat_local::forget_profile(&app, &expected)
        .map_err(|error| ambiguous_mutation_response(&state, error.message))?;
    Ok(result)
}

#[tauri::command]
pub async fn list_servers(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    generation: Option<u64>,
) -> Result<Vec<ServerDto>, AgentError> {
    require_main_window(&webview)?;
    let (_, catalog) = state.catalog_at(generation)?;
    // A caller bound to a snapshot must not receive never-probed rows because
    // the snapshot is absent; that is a stale read, not a fresh install.
    if generation.is_some() && catalog.is_none() {
        return Err(AgentError::new(
            "catalog-required",
            "Refresh the vault before loading server details.",
            true,
        ));
    }
    let transport = state.agent.transport();
    tauri::async_runtime::spawn_blocking(move || {
        let value = transport
            .call(Operation::ListProfiles)
            .map_err(AgentError::from_desktop)?;
        let profiles = profile_summaries(value)?;
        Ok(profiles
            .into_iter()
            .map(|profile| {
                let accounts = catalog
                    .as_ref()
                    .map(|catalog| {
                        catalog
                            .stores
                            .iter()
                            .filter_map(|store| match store {
                                CatalogStoreSummary::Account { store }
                                    if store.profile == profile.name =>
                                {
                                    Some(store.account_alias.clone())
                                }
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let blocked = catalog
                    .as_ref()
                    .is_some_and(|catalog| catalog.profile_blocked(&profile.name));
                ServerDto {
                    id: profile.name.clone(),
                    name: profile.name,
                    label: profile.label,
                    configured_probe: profile.probe,
                    host_id: None,
                    chain: None,
                    epoch: None,
                    lease: None,
                    accounts,
                    state: if blocked { "blocked" } else { "never-probed" },
                    chat_available: false,
                }
            })
            .collect())
    })
    .await
    .map_err(|error| AgentError::unknown(format!("failed to load servers: {error}")))?
}
