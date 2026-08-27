//! Deterministic validation and rendering for the checked FOKS protocol contract.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const ARTIFACT_SCHEMA_VERSION: u64 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub schema_version: u64,
    pub source: SourceIdentity,
    pub protocols: Vec<Protocol>,
    pub statuses: Vec<NamedValue>,
    pub services: Vec<NamedValue>,
    pub sources: Vec<SourceFile>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentity {
    pub module: String,
    pub version: String,
    pub sum: String,
    pub go_mod_sum: String,
    #[serde(default)]
    pub commit: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Protocol {
    pub name: String,
    pub unique_id: u64,
    pub go_file: String,
    pub methods: Vec<Method>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Method {
    pub name: String,
    pub position: u64,
    pub qualified_name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NamedValue {
    pub name: String,
    pub value: i64,
    pub go_file: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFile {
    pub path: String,
    pub sha256: String,
    pub semantic_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub version: u64,
    pub baseline: String,
    pub status_aliases: BTreeMap<String, String>,
    #[serde(rename = "protocol")]
    pub protocols: Vec<ProtocolPolicy>,
    #[serde(rename = "service")]
    pub services: Vec<ServicePolicy>,
    #[serde(rename = "route")]
    pub routes: Vec<RoutePolicy>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProtocolPolicy {
    pub name: String,
    pub upstream: String,
    pub id_constant: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ServicePolicy {
    pub name: String,
    pub upstream: String,
    pub listener: String,
    pub supported: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RoutePolicy {
    pub protocol: String,
    pub method: String,
    #[serde(default)]
    pub upstream_method: Option<String>,
    #[serde(default)]
    pub position_constant: Option<String>,
    pub listener: String,
    pub authentication: String,
    pub request: String,
    pub result: String,
    pub statuses: Vec<String>,
    pub max_request_bytes: usize,
    pub supported: bool,
    pub coverage: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergedService<'a> {
    pub policy: &'a ServicePolicy,
    pub service_type: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergedRoute<'a> {
    pub policy: &'a RoutePolicy,
    pub protocol_id: u64,
    pub position: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Merged<'a> {
    pub artifact: &'a Artifact,
    pub policy: &'a Policy,
    pub services: Vec<MergedService<'a>>,
    pub routes: Vec<MergedRoute<'a>>,
    pub status_codes: BTreeMap<&'a str, u64>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MetadataError {
    #[error("artifact schema {0} is unsupported")]
    Schema(u64),
    #[error("invalid metadata: {0}")]
    Invalid(String),
    #[error("JSON error: {0}")]
    Json(String),
    #[error("TOML error: {0}")]
    Toml(String),
}

pub fn parse_artifact(input: &str) -> Result<Artifact, MetadataError> {
    let artifact: Artifact =
        serde_json::from_str(input).map_err(|error| MetadataError::Json(error.to_string()))?;
    validate_artifact(&artifact)?;
    Ok(artifact)
}

pub fn parse_policy(input: &str) -> Result<Policy, MetadataError> {
    let policy: Policy =
        toml::from_str(input).map_err(|error| MetadataError::Toml(error.to_string()))?;
    if policy.version != 1 {
        return Err(MetadataError::Invalid(format!(
            "policy version {} is unsupported",
            policy.version
        )));
    }
    Ok(policy)
}

pub fn validate_artifact(artifact: &Artifact) -> Result<(), MetadataError> {
    if artifact.schema_version != ARTIFACT_SCHEMA_VERSION {
        return Err(MetadataError::Schema(artifact.schema_version));
    }
    if artifact.source.module.is_empty() || artifact.source.version.is_empty() {
        return invalid("artifact source identity is incomplete");
    }
    if !artifact
        .protocols
        .windows(2)
        .all(|pair| pair[0].name < pair[1].name)
        || !artifact
            .statuses
            .windows(2)
            .all(|pair| pair[0].name < pair[1].name)
        || !artifact
            .services
            .windows(2)
            .all(|pair| pair[0].name < pair[1].name)
        || !artifact
            .sources
            .windows(2)
            .all(|pair| pair[0].path < pair[1].path)
    {
        return invalid("artifact collections are not in canonical order");
    }
    let mut protocol_names = BTreeSet::new();
    let mut protocol_ids = BTreeSet::new();
    for protocol in &artifact.protocols {
        if !protocol_names.insert(protocol.name.as_str()) {
            return invalid(format!("duplicate protocol {}", protocol.name));
        }
        if !protocol_ids.insert(protocol.unique_id) {
            return invalid(format!("duplicate protocol ID {:#x}", protocol.unique_id));
        }
        if protocol.unique_id > u32::MAX.into() {
            return invalid(format!(
                "protocol ID {:#x} exceeds 32 bits",
                protocol.unique_id
            ));
        }
        let mut method_names = BTreeSet::new();
        let mut positions = BTreeSet::new();
        if !protocol.methods.windows(2).all(|pair| {
            (pair[0].position, pair[0].name.as_str()) < (pair[1].position, pair[1].name.as_str())
        }) {
            return invalid(format!("methods in {} are not canonical", protocol.name));
        }
        for method in &protocol.methods {
            if !method_names.insert(method.name.as_str()) {
                return invalid(format!(
                    "duplicate method {}.{}",
                    protocol.name, method.name
                ));
            }
            if !positions.insert(method.position) {
                return invalid(format!(
                    "duplicate method position {} in {}",
                    method.position, protocol.name
                ));
            }
            if method.position > u32::MAX.into()
                || method.qualified_name != format!("{}.{}", protocol.name, method.name)
            {
                return invalid(format!(
                    "invalid method identity {}.{}",
                    protocol.name, method.name
                ));
            }
        }
    }
    validate_named_values("status", &artifact.statuses)?;
    validate_named_values("service", &artifact.services)?;
    let mut sources = BTreeSet::new();
    for source in &artifact.sources {
        if !sources.insert(source.path.as_str())
            || source.sha256.len() != 64
            || !source.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || source.semantic_sha256.len() != 64
            || !source
                .semantic_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return invalid(format!("invalid source entry {}", source.path));
        }
    }
    Ok(())
}

fn validate_named_values(kind: &str, values: &[NamedValue]) -> Result<(), MetadataError> {
    let mut names = BTreeSet::new();
    for value in values {
        if !names.insert(value.name.as_str()) || value.value < 0 {
            return invalid(format!("invalid or duplicate {kind} {}", value.name));
        }
    }
    Ok(())
}

pub fn merge<'a>(artifact: &'a Artifact, policy: &'a Policy) -> Result<Merged<'a>, MetadataError> {
    validate_artifact(artifact)?;
    if policy.baseline != format!("foks-{}", artifact.source.version) {
        return invalid(format!(
            "policy baseline {} does not identify artifact {}",
            policy.baseline, artifact.source.version
        ));
    }
    let upstream_protocols = artifact
        .protocols
        .iter()
        .map(|protocol| (protocol.name.as_str(), protocol))
        .collect::<BTreeMap<_, _>>();
    let upstream_statuses = artifact
        .statuses
        .iter()
        .map(|status| (status.name.as_str(), status.value))
        .collect::<BTreeMap<_, _>>();
    let upstream_services = artifact
        .services
        .iter()
        .map(|service| (service.name.as_str(), service.value))
        .collect::<BTreeMap<_, _>>();

    let mut local_protocols = BTreeMap::new();
    let mut constants = BTreeSet::new();
    for protocol in &policy.protocols {
        if local_protocols
            .insert(protocol.name.as_str(), protocol)
            .is_some()
        {
            return invalid(format!("duplicate local protocol {}", protocol.name));
        }
        if !upstream_protocols.contains_key(protocol.upstream.as_str()) {
            return invalid(format!("unknown upstream protocol {}", protocol.upstream));
        }
        validate_constant(&protocol.id_constant)?;
        if !constants.insert(protocol.id_constant.as_str()) {
            return invalid(format!("duplicate Rust constant {}", protocol.id_constant));
        }
    }

    let mut status_codes = BTreeMap::new();
    for (alias, upstream) in &policy.status_aliases {
        validate_policy_key(alias)?;
        let value = upstream_statuses
            .get(upstream.as_str())
            .ok_or_else(|| MetadataError::Invalid(format!("unknown upstream status {upstream}")))?;
        status_codes.insert(
            alias.as_str(),
            u64::try_from(*value).expect("nonnegative status"),
        );
    }

    let mut services = Vec::new();
    let mut service_names = BTreeSet::new();
    let mut service_types = BTreeSet::new();
    for service in &policy.services {
        let value = upstream_services
            .get(service.upstream.as_str())
            .ok_or_else(|| {
                MetadataError::Invalid(format!("unknown upstream service {}", service.upstream))
            })?;
        let value = u64::try_from(*value).expect("nonnegative service");
        if !service_names.insert(service.name.as_str()) || !service_types.insert(value) {
            return invalid(format!(
                "duplicate local service {} or type {value}",
                service.name
            ));
        }
        validate_listener(&service.listener)?;
        services.push(MergedService {
            policy: service,
            service_type: value,
        });
    }

    let mut routes = Vec::new();
    let mut local_routes = BTreeSet::new();
    let mut dispatch = BTreeSet::new();
    for route in &policy.routes {
        let protocol_policy = local_protocols
            .get(route.protocol.as_str())
            .ok_or_else(|| {
                MetadataError::Invalid(format!("unknown local protocol {}", route.protocol))
            })?;
        let protocol = upstream_protocols[protocol_policy.upstream.as_str()];
        let upstream_method = route.upstream_method.as_deref().unwrap_or(&route.method);
        let method = protocol
            .methods
            .iter()
            .find(|method| method.name == upstream_method)
            .ok_or_else(|| {
                MetadataError::Invalid(format!(
                    "unknown upstream method {}.{}",
                    protocol.name, upstream_method
                ))
            })?;
        if !local_routes.insert((route.protocol.as_str(), route.method.as_str()))
            || !dispatch.insert((protocol.unique_id, method.position))
        {
            return invalid(format!(
                "duplicate route {}.{} or dispatch key",
                route.protocol, route.method
            ));
        }
        if let Some(constant) = &route.position_constant {
            validate_constant(constant)?;
            if !constants.insert(constant.as_str()) {
                return invalid(format!("duplicate Rust constant {constant}"));
            }
        }
        validate_listener(&route.listener)?;
        if route.authentication.is_empty()
            || route.request.is_empty()
            || route.result.is_empty()
            || route.statuses.is_empty()
            || route.coverage.is_empty()
            || !(1..=16 * 1024 * 1024).contains(&route.max_request_bytes)
        {
            return invalid(format!(
                "incomplete route {}.{}",
                route.protocol, route.method
            ));
        }
        for status in &route.statuses {
            if !status_codes.contains_key(status.as_str()) {
                return invalid(format!(
                    "route {}.{} uses unknown status {status}",
                    route.protocol, route.method
                ));
            }
        }
        if !route.supported && route.statuses.as_slice() != ["unsupported"] {
            return invalid(format!(
                "unsupported route {}.{} exposes non-unsupported statuses",
                route.protocol, route.method
            ));
        }
        routes.push(MergedRoute {
            policy: route,
            protocol_id: protocol.unique_id,
            position: method.position,
        });
    }
    Ok(Merged {
        artifact,
        policy,
        services,
        routes,
        status_codes,
    })
}

fn validate_constant(value: &str) -> Result<(), MetadataError> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        || value.as_bytes()[0].is_ascii_digit()
    {
        return invalid(format!("invalid Rust constant name {value:?}"));
    }
    Ok(())
}

fn validate_policy_key(value: &str) -> Result<(), MetadataError> {
    if value.is_empty()
        || value.as_bytes()[0].is_ascii_digit()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return invalid(format!("invalid local policy key {value:?}"));
    }
    Ok(())
}

fn validate_listener(value: &str) -> Result<(), MetadataError> {
    if matches!(value, "probe" | "public_services" | "authenticated") {
        Ok(())
    } else {
        invalid(format!("unknown listener {value}"))
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, MetadataError> {
    Err(MetadataError::Invalid(message.into()))
}

pub fn render_protocol_ids(merged: &Merged<'_>) -> String {
    let mut output = generated_header("Protocol IDs and method positions extracted from go-foks.");
    let protocols = merged
        .artifact
        .protocols
        .iter()
        .map(|protocol| (protocol.name.as_str(), protocol))
        .collect::<BTreeMap<_, _>>();
    for policy in &merged.policy.protocols {
        let protocol = protocols[policy.upstream.as_str()];
        writeln!(
            output,
            "pub const {}: u64 = {:#010x};",
            policy.id_constant, protocol.unique_id
        )
        .expect("write String");
        for route in merged
            .routes
            .iter()
            .filter(|route| route.policy.protocol == policy.name)
        {
            if let Some(constant) = &route.policy.position_constant {
                writeln!(output, "pub const {constant}: u64 = {};", route.position)
                    .expect("write String");
            }
        }
        output.push('\n');
    }
    if output.ends_with("\n\n") {
        output.pop();
    }
    output
}

pub fn render_status_codes(merged: &Merged<'_>) -> String {
    let mut output = generated_header("Status constants extracted from go-foks.");
    let mut emitted = BTreeSet::new();
    for upstream in merged.policy.status_aliases.values() {
        if !emitted.insert(upstream) {
            continue;
        }
        let status = merged
            .artifact
            .statuses
            .iter()
            .find(|status| status.name == *upstream)
            .expect("validated status");
        writeln!(
            output,
            "pub const STATUS_{upstream}: u64 = {};",
            status.value
        )
        .expect("write String");
    }
    output
}

pub fn render_routes(merged: &Merged<'_>) -> String {
    let mut output = generated_header("Server services and routes merged with local policy.");
    output.push_str("use super::{RouteSpec, ServiceSpec};\n\n");
    output.push_str("#[rustfmt::skip]\npub const SERVICES: &[ServiceSpec] = &[\n");
    for service in &merged.services {
        output.push_str("    ServiceSpec {\n");
        writeln!(
            output,
            "        name: {},",
            rust_string(&service.policy.name)
        )
        .expect("write String");
        writeln!(output, "        service_type: {},", service.service_type).expect("write String");
        writeln!(
            output,
            "        listener: {},",
            rust_string(&service.policy.listener)
        )
        .expect("write String");
        writeln!(output, "        supported: {},", service.policy.supported).expect("write String");
        output.push_str("    },\n");
    }
    output.push_str("];\n\n#[rustfmt::skip]\npub const ROUTES: &[RouteSpec] = &[\n");
    for route in &merged.routes {
        let policy = route.policy;
        writeln!(output, "    RouteSpec {{").expect("write String");
        writeln!(
            output,
            "        protocol: {},",
            rust_string(&policy.protocol)
        )
        .expect("write String");
        writeln!(output, "        protocol_id: {:#010x},", route.protocol_id)
            .expect("write String");
        writeln!(output, "        method: {},", rust_string(&policy.method)).expect("write String");
        writeln!(output, "        position: {},", route.position).expect("write String");
        writeln!(
            output,
            "        listener: {},",
            rust_string(&policy.listener)
        )
        .expect("write String");
        writeln!(
            output,
            "        authentication: {},",
            rust_string(&policy.authentication)
        )
        .expect("write String");
        writeln!(output, "        request: {},", rust_string(&policy.request))
            .expect("write String");
        writeln!(output, "        result: {},", rust_string(&policy.result)).expect("write String");
        write!(output, "        statuses: &[").expect("write String");
        write_strings(&mut output, &policy.statuses, rust_string);
        output.push_str("],\n");
        writeln!(
            output,
            "        max_request_bytes: {},",
            policy.max_request_bytes
        )
        .expect("write String");
        writeln!(output, "        supported: {},", policy.supported).expect("write String");
        write!(output, "        coverage: &[").expect("write String");
        write_strings(&mut output, &policy.coverage, rust_string);
        output.push_str("],\n    },\n");
    }
    output.push_str("];\n");
    output
}

pub fn render_contract(merged: &Merged<'_>) -> String {
    let source = &merged.artifact.source;
    let mut output = generated_comment(
        "Complete local contract merged from pinned upstream metadata and policy.",
    );
    writeln!(output, "version = {}", merged.policy.version).expect("write String");
    writeln!(
        output,
        "baseline = {}",
        toml_string(&merged.policy.baseline)
    )
    .expect("write String");
    writeln!(output, "upstream_module = {}", toml_string(&source.module)).expect("write String");
    writeln!(
        output,
        "upstream_version = {}",
        toml_string(&source.version)
    )
    .expect("write String");
    writeln!(output, "upstream_sum = {}", toml_string(&source.sum)).expect("write String");
    writeln!(
        output,
        "upstream_commit = {}\n",
        toml_string(&source.commit)
    )
    .expect("write String");
    output.push_str("[status_codes]\n");
    for (name, value) in &merged.status_codes {
        writeln!(output, "{name} = {value}").expect("write String");
    }
    for service in &merged.services {
        output.push_str("\n[[service]]\n");
        writeln!(output, "name = {}", toml_string(&service.policy.name)).expect("write String");
        writeln!(output, "service_type = {}", service.service_type).expect("write String");
        writeln!(
            output,
            "listener = {}",
            toml_string(&service.policy.listener)
        )
        .expect("write String");
        writeln!(output, "supported = {}", service.policy.supported).expect("write String");
    }
    for route in &merged.routes {
        let policy = route.policy;
        output.push_str("\n[[route]]\n");
        writeln!(output, "protocol = {}", toml_string(&policy.protocol)).expect("write String");
        writeln!(output, "protocol_id = {:#010x}", route.protocol_id).expect("write String");
        writeln!(output, "method = {}", toml_string(&policy.method)).expect("write String");
        writeln!(output, "position = {}", route.position).expect("write String");
        writeln!(output, "listener = {}", toml_string(&policy.listener)).expect("write String");
        writeln!(
            output,
            "authentication = {}",
            toml_string(&policy.authentication)
        )
        .expect("write String");
        writeln!(output, "request = {}", toml_string(&policy.request)).expect("write String");
        writeln!(output, "result = {}", toml_string(&policy.result)).expect("write String");
        write!(output, "statuses = [").expect("write String");
        write_strings(&mut output, &policy.statuses, toml_string);
        output.push_str("]\n");
        writeln!(output, "max_request_bytes = {}", policy.max_request_bytes).expect("write String");
        writeln!(output, "supported = {}", policy.supported).expect("write String");
        write!(output, "coverage = [").expect("write String");
        write_strings(&mut output, &policy.coverage, toml_string);
        output.push_str("]\n");
    }
    output
}

pub fn render_digest_manifest(artifact_bytes: &[u8], artifact: &Artifact) -> String {
    let digest = Sha256::digest(artifact_bytes);
    let mut output = String::new();
    writeln!(output, "artifact_sha256  {}", hex(&digest)).expect("write String");
    writeln!(output, "module           {}", artifact.source.module).expect("write String");
    writeln!(output, "version          {}", artifact.source.version).expect("write String");
    writeln!(output, "module_sum       {}", artifact.source.sum).expect("write String");
    writeln!(output, "go_mod_sum       {}", artifact.source.go_mod_sum).expect("write String");
    writeln!(output, "commit           {}", artifact.source.commit).expect("write String");
    for source in &artifact.sources {
        writeln!(
            output,
            "source_sha256    {}  {}",
            source.sha256, source.path
        )
        .expect("write String");
        writeln!(
            output,
            "semantic_sha256  {}  {}",
            source.semantic_sha256, source.path
        )
        .expect("write String");
    }
    output
}

fn generated_header(description: &str) -> String {
    format!("// @generated by tools/foks-server/generate-protocol.sh; DO NOT EDIT.\n// {description}\n\n")
}

fn generated_comment(description: &str) -> String {
    format!(
        "# @generated by tools/foks-server/generate-protocol.sh; DO NOT EDIT.\n# {description}\n\n"
    )
}

fn rust_string(value: &str) -> String {
    format!("{value:?}")
}

fn toml_string(value: &str) -> String {
    toml::Value::String(value.to_owned()).to_string()
}

fn write_strings(output: &mut String, values: &[String], render: fn(&str) -> String) {
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        output.push_str(&render(value));
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[usize::from(byte >> 4)] as char);
        output.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    output
}
