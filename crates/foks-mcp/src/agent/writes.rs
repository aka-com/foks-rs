//! Durable write translation and recovery results, separate from read projections.
use super::*;

impl AgentBackend {
    pub(super) fn write(
        &self,
        input: Option<&str>,
        spec: foks_agent_proto::data::DataWriteSpec,
        body: &[u8],
        cancelled: &CancellationToken,
    ) -> Result<CallToolResult, String> {
        use foks_agent_proto::data::{DataSubmission, DataWriteOutcome, DataWriteStatus};
        let id = match input {
            Some(id) => id
                .parse::<foks_agent_proto::data::SubmissionHandle>()
                .map_err(|error| error.to_string())?
                .to_string(),
            None => {
                let mut random = [0; 16];
                getrandom::fill(&mut random).map_err(|_| "randomness unavailable")?;
                let issued = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| "clock-untrusted")?
                    .as_secs();
                foks_agent_proto::data::SubmissionHandle::new(issued, random).to_string()
            }
        };
        let prepared: DataWriteOutcome = call(
            &self.client,
            Operation::PrepareDataWrite {
                scope: self.account.clone(),
                submission_id: id.clone(),
                spec: spec.clone(),
            },
            cancelled,
        )
        .map_err(|error| {
            format!(
                "{error}; submission_id={id}; inspect fennec_pending/status before another write"
            )
        })?;
        if prepared.submission_id != id {
            return Err("agent changed submission identity".into());
        }
        if prepared.status != DataWriteStatus::Prepared {
            return write_tool_result(prepared, spec.kind);
        }
        if cancelled.is_cancelled() {
            return Err(format!("cancelled; prepared submission_id={id}"));
        }
        let submission = DataSubmission {
            scope: self.account.clone(),
            submission_id: id.clone(),
        };
        let result: DataWriteOutcome = if spec.kind == foks_agent_proto::data::DataWriteKind::Put {
            let header = foks_agent_proto::KvUploadHeader {
                adapter: Some(submission),
                store: foks_agent_proto::KvStoreRef::Account(foks_agent_proto::AccountStoreRef {
                    profile: self.account.profile.clone(),
                    account_alias: self.account.account_alias.clone(),
                }),
                path: spec.path,
                total_length: body.len() as u64,
                read_role: foks_agent_proto::KvRole::Owner,
                write_role: foks_agent_proto::KvRole::Owner,
                precondition: foks_agent_proto::KvPrecondition::Create,
                mkdir_p: false,
            };
            let response = self.client.put_kv_stream_cancellable(header, &mut &body[..], &|| cancelled.is_cancelled())
                .map_err(|_| format!("upload interrupted; inspect submission_id={id}; do not repeat with a new ID"))?;
            match response.result {
                ResponseResult::Success { value } => {
                    serde_json::from_value(value).map_err(|_| "invalid write outcome")?
                }
                ResponseResult::Error { code, .. } => {
                    return Err(format!(
                        "{}; inspect submission_id={id}",
                        error_code_name(code)
                    ))
                }
            }
        } else {
            call(
                &self.client,
                Operation::ExecuteDataWrite { submission },
                cancelled,
            )
            .map_err(|error| format!("{error}; submission_id={id}"))?
        };
        if result.submission_id != id {
            return Err("agent changed submission identity".into());
        }
        write_tool_result(result, spec.kind)
    }
}

fn write_tool_result(
    result: foks_agent_proto::data::DataWriteOutcome,
    kind: foks_agent_proto::data::DataWriteKind,
) -> Result<CallToolResult, String> {
    let committed = result.status == foks_agent_proto::data::DataWriteStatus::Committed;
    let node_id = result.node_id.clone();
    let mut response = write_result(result)?;
    if committed {
        let text = if kind == foks_agent_proto::data::DataWriteKind::Mkdir {
            match node_id {
                Some(id) if id.len() == 34 => {
                    let mut bytes = [0; 17];
                    for (byte, pair) in bytes.iter_mut().zip(id.as_bytes().chunks_exact(2)) {
                        *byte = u8::from_str_radix(
                            std::str::from_utf8(pair).map_err(|_| "invalid directory ID")?,
                            16,
                        )
                        .map_err(|_| "invalid directory ID")?;
                    }
                    format!("DirID: 1{}", foks_proto::encode_base62_strict(&bytes[1..]))
                }
                _ => "committed; original directory ID is unavailable in this recovery result"
                    .to_owned(),
            }
        } else {
            "ok".to_owned()
        };
        response.content = vec![rmcp::model::ContentBlock::text(text)];
    }
    Ok(response)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn write_spec(
    kind: foks_agent_proto::data::DataWriteKind,
    path: &str,
    destination: Option<&str>,
    team_selector: Option<String>,
    overwrite: bool,
    mkdir_p: bool,
    recursive: bool,
    body: &[u8],
) -> Result<foks_agent_proto::data::DataWriteSpec, String> {
    Ok(foks_agent_proto::data::DataWriteSpec {
        kind,
        path: encode_path(path)?,
        destination: destination.map(encode_path).transpose()?,
        team_selector: team_selector.filter(|team| !team.is_empty()),
        overwrite,
        mkdir_p,
        recursive,
        body_length: body.len() as u64,
        body_hash: foks_crypto::kv_adapter_body_hash(body),
    })
}

pub(super) fn write_result(
    result: foks_agent_proto::data::DataWriteOutcome,
) -> Result<CallToolResult, String> {
    let value = serde_json::to_value(&result).map_err(|_| "invalid write outcome")?;
    use foks_agent_proto::data::DataWriteStatus;
    let guidance=match result.status {
        DataWriteStatus::Expired => " This handle cannot execute; a new explicit write needs a newly issued handle.",
        DataWriteStatus::Rejected => " This handle cannot execute; correct the request before making a new explicit write with a new handle.",
        DataWriteStatus::SubmissionUnknown => " Inspect status; do not repeat this write with a new handle.",
        _ => "",
    };
    let mut response = text_result(format!("{value}{guidance}"));
    response.structured_content = Some(value);
    response.is_error = Some(matches!(
        result.status,
        foks_agent_proto::data::DataWriteStatus::Rejected
            | foks_agent_proto::data::DataWriteStatus::SubmissionUnknown
            | foks_agent_proto::data::DataWriteStatus::Expired
    ));
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_admission_errors_keep_their_wire_names() {
        assert_eq!(
            error_code_name(foks_agent_proto::ErrorCode::RetentionFull),
            "retention-full"
        );
        assert_eq!(
            error_code_name(foks_agent_proto::ErrorCode::ClockUntrusted),
            "clock-untrusted"
        );
    }
}
