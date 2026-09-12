//! Typed, cancellable IPC. No credentials, database access or CLI subprocesses.
use crate::{
    contract::{encode_content, ContractError, Invocation, MAX_FILE_BYTES},
    session::{text_result, Backend},
};
use foks_agent_client::AgentClient;
use foks_agent_proto::{
    data::{
        DataCatalog, DataCatalogEntry, DataChunk, DataEntry, DataMember, DataMemberships, DataRead,
        DataScope, DataUsage,
    },
    Operation, ResponseResult,
};
use rmcp::model::CallToolResult;
use serde::de::DeserializeOwned;
use std::{collections::HashSet, path::Path};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

mod stat;
mod writes;
use writes::{write_result, write_spec};

pub struct AgentBackend {
    client: AgentClient,
    account: DataScope,
}

impl AgentBackend {
    pub fn connect(socket: &Path, profile: String, account_alias: String) -> Result<Self, String> {
        let client = AgentClient::new(socket);
        let account: DataScope = call(
            &client,
            Operation::BindDataAccount {
                profile: profile.clone(),
                account_alias: account_alias.clone(),
            },
            &CancellationToken::new(),
        )?;
        if account.profile != profile
            || account.account_alias != account_alias
            || account.team_id.is_some()
        {
            return Err("agent returned a different account scope".into());
        }
        Ok(Self { client, account })
    }

    fn scope(
        &self,
        team: Option<&str>,
        cancelled: &CancellationToken,
    ) -> Result<DataScope, String> {
        let Some(team) = team.filter(|t| !t.is_empty()) else {
            return Ok(self.account.clone());
        };
        let scope: DataScope = self.read(
            &self.account,
            DataRead::ResolveTeam {
                selector: team.to_owned(),
            },
            cancelled,
        )?;
        if scope.profile != self.account.profile
            || scope.account_alias != self.account.account_alias
            || scope.host_id != self.account.host_id
            || scope.user_id != self.account.user_id
            || scope.team_id.is_none()
        {
            return Err("agent changed the selected account scope".into());
        }
        Ok(scope)
    }

    fn read<T: DeserializeOwned>(
        &self,
        scope: &DataScope,
        query: DataRead,
        cancelled: &CancellationToken,
    ) -> Result<T, String> {
        call(
            &self.client,
            Operation::ReadData {
                scope: scope.clone(),
                query,
            },
            cancelled,
        )
    }

    fn entry(
        &self,
        scope: &DataScope,
        path: &str,
        version: u64,
        cancelled: &CancellationToken,
    ) -> Result<DataEntry, String> {
        let result: DataEntry = self.read(
            scope,
            DataRead::Entry {
                path: path.to_owned(),
                version,
            },
            cancelled,
        )?;
        if result.path != path || result.version != version {
            return Err("agent changed the requested entry".into());
        }
        Ok(result)
    }

    fn resolve<'a>(
        &self,
        scope: &DataScope,
        catalog: &'a DataCatalog,
        input: &str,
        follow_final: bool,
        cancelled: &CancellationToken,
    ) -> Result<Option<&'a DataCatalogEntry>, String> {
        let mut path = encode_path(input)?;
        let mut seen = HashSet::new();
        for _ in 0..32 {
            if path == "/" {
                return Ok(None);
            }
            if !seen.insert(path.clone()) {
                return Err("KV symlink cycle".into());
            }
            let pieces: Vec<_> = path.trim_start_matches('/').split('/').collect();
            let mut prefix = String::new();
            let mut redirect = None;
            for (i, component) in pieces.iter().enumerate() {
                prefix.push('/');
                prefix.push_str(component);
                let row = catalog
                    .entries
                    .iter()
                    .find(|r| r.path == prefix)
                    .ok_or("KV path does not exist or is inaccessible")?;
                let final_entry = i + 1 == pieces.len();
                if row.node_type == "symlink" && (!final_entry || follow_final) {
                    let mut node = self.entry(scope, &prefix, row.version, cancelled)?;
                    let target = node.symlink_target.take().ok_or("symlink has no target")?;
                    let parent = prefix.rsplit_once('/').map_or("", |(p, _)| p);
                    let raw_parent = decode_path(parent)?;
                    let target = if target.starts_with('/') {
                        target
                    } else {
                        format!("{raw_parent}/{target}")
                    };
                    let suffix = pieces[i + 1..].join("/");
                    redirect = Some(if suffix.is_empty() {
                        encode_path(&target)?
                    } else {
                        format!("{}/{}", encode_path(&target)?.trim_end_matches('/'), suffix)
                    });
                    break;
                }
                if final_entry {
                    return Ok(Some(row));
                }
                if row.node_type != "directory" {
                    return Err("KV parent is not a directory".into());
                }
            }
            path = redirect.ok_or("KV path could not be resolved")?;
        }
        Err("KV symlink depth exceeded".into())
    }
}

impl Backend for AgentBackend {
    fn invoke(
        &self,
        invocation: Invocation,
        cancelled: &CancellationToken,
    ) -> Result<CallToolResult, String> {
        match invocation {
            Invocation::Members(args) => {
                let scope = self.scope(Some(&args.team), cancelled)?;
                let rows: Vec<DataMember> = self.read(&scope, DataRead::Members, cancelled)?;
                let mut text = String::new();
                for row in rows {
                    let date = time::OffsetDateTime::from_unix_timestamp_nanos(
                        i128::from(row.added_millis) * 1_000_000,
                    )
                    .map_err(|_| "invalid admission time")?
                    .replace_nanosecond(0)
                    .map_err(|_| "invalid admission time")?
                    .format(&time::format_description::well_known::Rfc3339)
                    .map_err(|_| "invalid admission time")?;
                    text.push_str(&format!(
                        "{}\t{}\t{}\t{}\t{date}\n",
                        row.name, row.host, row.source_role, row.destination_role
                    ));
                }
                Ok(text_result(text))
            }
            Invocation::Memberships(_) => {
                let rows: DataMemberships =
                    self.read(&self.account, DataRead::Memberships, cancelled)?;
                let mut text = String::new();
                for row in &rows.teams {
                    text.push_str(&format!(
                        "{}\t{}\t{}\t{}\t{}\n",
                        row.team,
                        row.source_role,
                        row.destination_role,
                        row.via.as_deref().unwrap_or("-"),
                        row.index_range
                    ));
                }
                if !rows.complete {
                    text.push_str("Incomplete: remote memberships require their authenticated host profile.\n");
                }
                let mut result = text_result(text);
                result.structured_content =
                    Some(serde_json::to_value(&rows).map_err(|_| "invalid membership result")?);
                Ok(result)
            }
            Invocation::Usage(args) => {
                let scope = self.scope(args.team.as_deref(), cancelled)?;
                let usage: DataUsage = self.read(&scope, DataRead::Usage, cancelled)?;
                Ok(text_result(format!(
                    "Num Files: {}\nTotal Size: {}",
                    usage.files, usage.bytes
                )))
            }
            Invocation::List(args) => {
                let scope = self.scope(args.team.as_deref(), cancelled)?;
                let catalog: DataCatalog = self.read(&scope, DataRead::Catalog, cancelled)?;
                let path = encode_path(&args.path)?;
                let path = if path == "/" {
                    path
                } else {
                    match self.resolve(&scope, &catalog, &args.path, true, cancelled)? {
                        None => "/".to_owned(),
                        Some(row) if row.node_type == "directory" => row.path.clone(),
                        _ => return Err("list path is not a directory".into()),
                    }
                };
                let prefix = if path == "/" {
                    "/".to_owned()
                } else {
                    format!("{path}/")
                };
                let mut text = String::new();
                for row in &catalog.entries {
                    let Some(name) = row.path.strip_prefix(&prefix).filter(|n| !n.contains('/'))
                    else {
                        continue;
                    };
                    let date = time::OffsetDateTime::from_unix_timestamp_nanos(
                        i128::from(row.modified_microseconds) * 1000,
                    )
                    .map_err(|_| "KV modification time is invalid")?
                    .replace_nanosecond(0)
                    .map_err(|_| "KV modification time is invalid")?
                    .format(&time::format_description::well_known::Rfc3339)
                    .map_err(|_| "KV modification time is invalid")?;
                    let kind = match row.node_type.as_str() {
                        "directory" => "dir",
                        "small-file" => "file",
                        "file" => "file",
                        "symlink" => "symlink",
                        _ => return Err("unknown KV node type".into()),
                    };
                    text.push_str(&format!("{}\t{kind}\t{date}\n", decode_path(name)?));
                }
                Ok(text_result(text))
            }
            Invocation::Get(args) => {
                let scope = self.scope(args.team.as_deref(), cancelled)?;
                let catalog: DataCatalog = self.read(&scope, DataRead::Catalog, cancelled)?;
                let row = self
                    .resolve(&scope, &catalog, &args.path, true, cancelled)?
                    .ok_or("get requires a file")?;
                let mut content = Zeroizing::new(Vec::new());
                if row.node_type == "small-file" {
                    let node = self.entry(&scope, &row.path, row.version, cancelled)?;
                    let bytes = node
                        .content
                        .as_deref()
                        .ok_or("small file content is missing")?;
                    if bytes.len() > MAX_FILE_BYTES {
                        return Err(ContractError::TooLarge.to_string());
                    }
                    content.extend_from_slice(bytes);
                } else if row.node_type == "file" {
                    loop {
                        let offset = content.len() as u64;
                        let length = (MAX_FILE_BYTES - content.len()).min(128 * 1024) as u32;
                        if length == 0 {
                            return Err(ContractError::TooLarge.to_string());
                        }
                        let chunk: DataChunk = self.read(
                            &scope,
                            DataRead::Chunk {
                                path: row.path.clone(),
                                version: row.version,
                                offset,
                                length,
                            },
                            cancelled,
                        )?;
                        if chunk.path != row.path
                            || chunk.version != row.version
                            || chunk.offset != offset
                            || (chunk.content.is_empty() && !chunk.eof)
                            || chunk.content.len() > length as usize
                        {
                            return Err("agent returned an inconsistent chunk".into());
                        }
                        content.extend_from_slice(&chunk.content);
                        if chunk.eof {
                            break;
                        }
                    }
                } else {
                    return Err("get requires a file".into());
                }
                // Revalidate identity/access and the selected version before publishing plaintext.
                let current: DataCatalog = self.read(&scope, DataRead::Catalog, cancelled)?;
                if current.user_chain_sequence != catalog.user_chain_sequence
                    || current.team_chain_sequence != catalog.team_chain_sequence
                    || !current.entries.iter().any(|r| {
                        r.path == row.path && r.node_id == row.node_id && r.version == row.version
                    })
                {
                    return Err("file changed during read".into());
                }
                Ok(text_result(
                    encode_content(&content, args.base64).map_err(|e| e.to_string())?,
                ))
            }
            Invocation::Stat(args) => {
                let scope = self.scope(args.team.as_deref(), cancelled)?;
                let catalog: DataCatalog = self.read(&scope, DataRead::Catalog, cancelled)?;
                let (path, version) = if encode_path(&args.path)? == "/" {
                    ("/".to_owned(), None)
                } else {
                    match self.resolve(&scope, &catalog, &args.path, true, cancelled)? {
                        Some(row) => (row.path.clone(), Some(row.version)),
                        None => ("/".to_owned(), None),
                    }
                };
                let evidence: foks_agent_proto::data::DataStat = self.read(
                    &scope,
                    DataRead::Stat {
                        path: path.clone(),
                        version,
                    },
                    cancelled,
                )?;
                if evidence.path != path || evidence.version != version {
                    return Err("agent changed stat scope".into());
                }
                if let Some(bytes) = &evidence.dirent {
                    let entry =
                        foks_proto::KvDirent::decode(bytes).map_err(|_| "invalid stat dirent")?;
                    let node_id: String = entry
                        .value
                        .0
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    if !catalog.entries.iter().any(|row| {
                        row.path == path && row.node_id == node_id && row.version == entry.version
                    }) {
                        return Err("stat target changed after selection".into());
                    }
                } else {
                    let directory = foks_proto::KvDirectoryPair::decode(
                        evidence.directory.as_deref().ok_or("missing stat root")?,
                    )
                    .map_err(|_| "invalid stat root")?;
                    let root_id: String = directory
                        .active
                        .id
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    if catalog.root_id.as_deref() != Some(root_id.as_str()) {
                        return Err("stat root changed after selection".into());
                    }
                }
                Ok(text_result(stat::go_stat(evidence)?.to_string()))
            }

            Invocation::Put(args) => {
                let body = crate::contract::decode_content(&args.content, args.base64)
                    .map_err(|error| error.to_string())?;
                self.write(
                    args.fennec_submission_id.as_deref(),
                    write_spec(
                        foks_agent_proto::data::DataWriteKind::Put,
                        &args.path,
                        None,
                        args.team.clone(),
                        args.overwrite,
                        args.mkdir_p,
                        false,
                        &body,
                    )?,
                    &body,
                    cancelled,
                )
            }
            Invocation::Mkdir(args) => self.write(
                args.fennec_submission_id.as_deref(),
                write_spec(
                    foks_agent_proto::data::DataWriteKind::Mkdir,
                    &args.path,
                    None,
                    args.team,
                    false,
                    args.mkdir_p,
                    false,
                    &[],
                )?,
                &[],
                cancelled,
            ),
            Invocation::Remove(args) => self.write(
                args.fennec_submission_id.as_deref(),
                write_spec(
                    foks_agent_proto::data::DataWriteKind::Remove,
                    &args.path,
                    None,
                    args.team,
                    false,
                    false,
                    args.recursive,
                    &[],
                )?,
                &[],
                cancelled,
            ),
            Invocation::Move(args) => self.write(
                args.fennec_submission_id.as_deref(),
                write_spec(
                    foks_agent_proto::data::DataWriteKind::Move,
                    &args.src,
                    Some(&args.dst),
                    args.team,
                    false,
                    false,
                    false,
                    &[],
                )?,
                &[],
                cancelled,
            ),
            Invocation::Status(args) => {
                let result: foks_agent_proto::data::DataWriteOutcome = call(
                    &self.client,
                    Operation::DataWriteStatus {
                        submission: foks_agent_proto::data::DataSubmission {
                            scope: self.account.clone(),
                            submission_id: args.fennec_submission_id.clone(),
                        },
                    },
                    cancelled,
                )?;
                if result.submission_id != args.fennec_submission_id {
                    return Err("agent changed submission identity".into());
                }
                write_result(result)
            }
            Invocation::Pending(_) => {
                let result: Vec<foks_agent_proto::data::DataWriteOutcome> = call(
                    &self.client,
                    Operation::PendingDataWrites {
                        scope: self.account.clone(),
                    },
                    cancelled,
                )?;
                if result.len() > 64 {
                    return Err("pending inventory exceeds limit".into());
                }
                Ok(text_result(
                    serde_json::to_string(&result).map_err(|_| "invalid pending inventory")?,
                ))
            }
        }
    }
}

fn call<T: DeserializeOwned>(
    client: &AgentClient,
    operation: Operation,
    cancelled: &CancellationToken,
) -> Result<T, String> {
    let response = client
        .call_cancellable(operation, &|| cancelled.is_cancelled())
        .map_err(|error| match error {
            foks_agent_client::Error::Ambiguous(_) => {
                "submission outcome unknown; check operation status before retrying".to_owned()
            }
            _ => "authenticated local agent unavailable; start or unlock the selected account"
                .to_owned(),
        })?;
    match response.result {
        ResponseResult::Success { value } => {
            serde_json::from_value(value).map_err(|_| "invalid typed agent response".to_owned())
        }
        ResponseResult::Error { code, message, .. } => Err(format!("{code:?}: {message}")),
    }
}

fn encode_path(path: &str) -> Result<String, String> {
    if path.len() > 4096 || path.contains('\0') {
        return Err("invalid KV path".into());
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let mut encoded = String::new();
    for part in parts {
        encoded.push('/');
        for byte in part.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
                encoded.push(byte as char);
            } else {
                use std::fmt::Write as _;
                write!(&mut encoded, "%{byte:02X}").map_err(|_| "invalid KV path")?;
            }
        }
    }
    if encoded.is_empty() {
        encoded.push('/');
    }
    if encoded.len() > 4096 {
        return Err("encoded KV path exceeds limit".into());
    }
    Ok(encoded)
}

fn decode_path(path: &str) -> Result<String, String> {
    let mut decoded = Vec::new();
    let mut bytes = path.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let high = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or("invalid encoded KV name")?;
            let low = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or("invalid encoded KV name")?;
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(b);
        }
    }
    String::from_utf8(decoded).map_err(|_| "KV name is not UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_are_foks_paths_and_literal_percent_is_preserved() {
        assert_eq!(encode_path("").unwrap(), "/");
        assert_eq!(encode_path("a/../ü %20").unwrap(), "/%C3%BC%20%2520");
        assert_eq!(decode_path("/%C3%BC%20%2520").unwrap(), "/ü %20");
        assert!(encode_path("bad\0path").is_err());
    }
}
