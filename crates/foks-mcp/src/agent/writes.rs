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
                "{}; submission_id={id}; inspect foks_pending or foks_status before another write",
                self.explain_clock_refusal(error, cancelled)
            )
        })?;
        if prepared.submission_id != id {
            return Err("agent changed submission identity".into());
        }
        if prepared.status != DataWriteStatus::Prepared {
            // A handle issued before the rejection watermark returns `expired`
            // rather than `clock-untrusted`, including the one minted above
            // while a trusted clock is still behind that watermark.
            let pause = self.expiry_pause(&prepared, cancelled);
            return write_tool_result(prepared, spec.kind, pause);
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
                        self.explain_clock_refusal(error_code_name(code), cancelled)
                    ))
                }
            }
        } else {
            call(
                &self.client,
                Operation::ExecuteDataWrite { submission },
                cancelled,
            )
            .map_err(|error| {
                format!(
                    "{}; submission_id={id}",
                    self.explain_clock_refusal(error, cancelled)
                )
            })?
        };
        if result.submission_id != id {
            return Err("agent changed submission identity".into());
        }
        let pause = self.expiry_pause(&result, cancelled);
        write_tool_result(result, spec.kind, pause)
    }

    /// The clock pause behind an `expired` outcome, if any. While admission is
    /// paused or behind the watermark, a newly issued handle cannot execute
    /// either, so the result explains the pause instead of suggesting one.
    fn expiry_pause(
        &self,
        result: &foks_agent_proto::data::DataWriteOutcome,
        cancelled: &CancellationToken,
    ) -> Option<ClockPause> {
        (result.status == foks_agent_proto::data::DataWriteStatus::Expired)
            .then(|| self.clock_pause(cancelled))
            .flatten()
    }
}

fn write_tool_result(
    result: foks_agent_proto::data::DataWriteOutcome,
    kind: foks_agent_proto::data::DataWriteKind,
    pause: Option<ClockPause>,
) -> Result<CallToolResult, String> {
    let committed = result.status == foks_agent_proto::data::DataWriteStatus::Committed;
    let node_id = result.node_id.clone();
    let mut response = write_result(result, pause)?;
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

/// Renders a write outcome. With `pause`, the result also carries
/// `writes_paused`, `writes_resume_at` when the clock is behind the watermark,
/// and the pause explanation, which replaces the advice to issue a new handle.
pub(super) fn write_result(
    result: foks_agent_proto::data::DataWriteOutcome,
    pause: Option<ClockPause>,
) -> Result<CallToolResult, String> {
    let mut value = serde_json::to_value(&result).map_err(|_| "invalid write outcome")?;
    use foks_agent_proto::data::DataWriteStatus;
    let guidance=match result.status {
        DataWriteStatus::Expired if pause.is_some() => " This handle cannot execute.",
        DataWriteStatus::Expired => " This handle cannot execute; a new explicit write needs a newly issued handle.",
        DataWriteStatus::Rejected => " This handle cannot execute; correct the request before making a new explicit write with a new handle.",
        DataWriteStatus::SubmissionUnknown => " Inspect status; do not repeat this write with a new handle.",
        _ => "",
    };
    if let (Some(pause), serde_json::Value::Object(fields)) = (pause, &mut value) {
        fields.insert("writes_paused".into(), serde_json::Value::Bool(true));
        if let ClockPause::Behind(resumes) = pause {
            fields.insert("writes_resume_at".into(), resumes.into());
        }
    }
    let mut response = text_result(format!("{value}{guidance}"));
    if let Some(pause) = pause {
        response
            .content
            .push(rmcp::model::ContentBlock::text(pause.explanation()));
    }
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

    #[test]
    fn clock_refusals_explain_the_pause_and_the_repair_outside_mcp() {
        use super::super::{with_clock_guidance, ClockPause, CLOCK_PAUSED};
        let refused = with_clock_guidance(
            format!(
                "{}: adapter clock is untrusted",
                error_code_name(foks_agent_proto::ErrorCode::ClockUntrusted)
            ),
            Some(ClockPause::Untrusted),
        );
        assert!(refused.ends_with(CLOCK_PAUSED), "{refused}");
        assert!(refused.contains("foks-rs retention reanchor"));
        assert_eq!(
            with_clock_guidance("retention-full: full".into(), None),
            "retention-full: full"
        );
        // Behind the watermark, a repair cannot help, so the explanation
        // names the time instead of the repair.
        let behind = with_clock_guidance(
            "clock-untrusted: adapter clock is untrusted".into(),
            Some(ClockPause::Behind(1_700_086_400)),
        );
        assert!(behind.contains("until Unix time 1700086400"), "{behind}");
        assert!(!behind.contains("retention reanchor"), "{behind}");
    }

    #[test]
    fn expired_writes_behind_the_watermark_report_the_pause_instead_of_a_new_handle() {
        use super::super::ClockPause;
        use foks_agent_proto::data::{DataWriteOutcome, DataWriteStatus};
        let expired = || DataWriteOutcome {
            submission_id: "handle".into(),
            status: DataWriteStatus::Expired,
            partial: false,
            node_id: None,
        };
        let text = |response: &CallToolResult| {
            response
                .content
                .iter()
                .filter_map(|block| block.as_text().map(|text| text.text.clone()))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let unpaused = write_result(expired(), None).unwrap();
        assert!(text(&unpaused).contains("newly issued handle"));
        assert!(unpaused.structured_content.as_ref().unwrap()["writes_paused"].is_null());
        // A trusted clock behind the watermark expires the handle MCP just
        // issued, so another new handle would expire too.
        let behind = write_result(expired(), Some(ClockPause::Behind(1_700_086_400))).unwrap();
        assert_eq!(behind.is_error, Some(true));
        let explained = text(&behind);
        assert!(!explained.contains("newly issued handle"), "{explained}");
        assert!(
            explained.contains("until Unix time 1700086400"),
            "{explained}"
        );
        let fields = behind.structured_content.unwrap();
        assert_eq!(fields["status"], "expired");
        assert_eq!(fields["writes_paused"], true);
        assert_eq!(fields["writes_resume_at"], 1_700_086_400);
        let untrusted = write_result(expired(), Some(ClockPause::Untrusted)).unwrap();
        assert!(text(&untrusted).contains("foks-rs retention reanchor"));
        assert!(untrusted.structured_content.unwrap()["writes_resume_at"].is_null());
    }
}
