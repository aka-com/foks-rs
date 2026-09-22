//! Normalize inputs and prepare immutable encrypted commands.
use super::*;

mod names {
    include!("../name_ranges.rs");
}

/// Go uses simple Unicode lowercase mapping (not contextual/full case folding).
pub fn normalize_chat_name(name: &str) -> Result<String> {
    let name: String = name
        .trim()
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    if name.is_empty() {
        return Ok(name);
    }
    if !(ChatLimits::NAME_MIN_CHARS..=ChatLimits::NAME_MAX_CHARS).contains(&name.chars().count())
        || name == "general"
        || name.contains("--")
        || name.chars().any(|c| {
            !names::NAME_RANGES
                .iter()
                .any(|&(lo, hi)| (lo..=hi).contains(&(c as u32)))
        })
    {
        return Err(Error::ChatInvalidInput(
            "invalid channel name; use empty name for general",
        ));
    }
    Ok(name)
}
impl ChatSession<'_> {
    /// Persist first; submit in a later checked application operation.
    pub fn prepare_channel(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        name: &str,
        description: &str,
        tier: RtChannelTier,
    ) -> Result<ChatOperation> {
        self.prepare_channel_submission(rpc, store, name, description, tier, None)
    }

    pub fn prepare_channel_submission(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        name: &str,
        description: &str,
        tier: RtChannelTier,
        submission: Option<&ChatSubmission>,
    ) -> Result<ChatOperation> {
        if let Some(operation) = self.submitted_operation(submission)? {
            return Ok(operation);
        }

        self.refresh()?;
        let name = RtText(normalize_chat_name(name)?);
        let desc = RtText(
            description
                .chars()
                .map(|c| c.to_lowercase().next().unwrap_or(c))
                .collect(),
        );
        if !desc.0.is_empty()
            && !(ChatLimits::DESCRIPTION_MIN_CHARS..=ChatLimits::DESCRIPTION_MAX_CHARS)
                .contains(&desc.0.chars().count())
        {
            return Err(Error::ChatInvalidInput(
                "description must contain 3 to 512 characters",
            ));
        }
        let set = self.list_current_channels(rpc)?;
        if set
            .channels
            .iter()
            .any(|c| c.metadata.tier == tier && c.name.0 == name.0)
        {
            return Err(Error::ChatNameConflict(
                "channel name already exists in tier",
            ));
        }
        let read = if tier == RtChannelTier::Admin {
            Role::ADMIN
        } else {
            Role::member(0)
        };
        if self.role < read {
            return Err(Error::ChatAccessDenied(
                "member role required to create channel",
            ));
        }
        let name_key = self.current(floor(tier)?)?;
        let desc_key = self.current(read)?;
        let id = random_id()?;
        let version = set
            .version
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(Error::ChatLimit("channel version overflow"))?;
        let metadata = RtChannelMetadata {
            id: RtChannelId(id),
            team: RtTeamId::new(self.team_id.clone())?,
            app: RtAppId::Chat,
            sequence: 1,
            name: RtBox {
                key: name_key,
                boxed: self.keys(name_key)?.seal_text(
                    RtKeyType::ChannelName,
                    &name,
                    random_id()?,
                )?,
            },
            description: Some(RtBox {
                key: desc_key,
                boxed: self.keys(desc_key)?.seal_text(
                    RtKeyType::ChannelDescription,
                    &desc,
                    random_id()?,
                )?,
            }),
            roles: RtRolePair { read, write: read },
            last_message: None,
            ctime: 0,
            mtime: 0,
            updated_at: version,
            tier,
            unreadable: false,
        };
        self.prepare(
            store,
            id,
            submission,
            RtChannelId(id),
            0,
            RealtimeRequest::CreateChannel(RtCreateChannelArgument {
                metadata,
                set_version: version,
            }),
        )
    }

    pub fn prepare_send(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        channel: RtChannelId,
        text: &str,
    ) -> Result<ChatOperation> {
        self.prepare_send_submission(rpc, store, channel, text, None)
    }

    pub fn prepare_send_submission(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        channel: RtChannelId,
        text: &str,
        submission: Option<&ChatSubmission>,
    ) -> Result<ChatOperation> {
        if let Some(operation) = self.submitted_operation(submission)? {
            return Ok(operation);
        }

        Ok(self
            .prepare_fresh_send(rpc, store, channel, text, submission)?
            .0)
    }

    pub fn submit_message<E: From<Error>>(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        channel: RtChannelId,
        text: &str,
        submission: &ChatSubmission,
        before_delivery: impl FnOnce() -> std::result::Result<(), E>,
    ) -> std::result::Result<ChatOperation, E> {
        if let Some(operation) = self.submitted_operation(Some(submission))? {
            return Ok(operation);
        }
        let (op, md) = self.prepare_fresh_send(rpc, store, channel, text, Some(submission))?;
        before_delivery()?;
        let request = self.protected_request(store, &op)?;
        let RealtimeRequest::Send(arg) = &request else {
            return Err(Error::ChatIntegrity("fresh message is not a send").into());
        };
        self.validate_prepared_send(&md, &arg.send)?;
        Ok(self.deliver_prepared(rpc, store, &op, request)?)
    }

    fn prepare_fresh_send(
        &mut self,
        rpc: &mut impl ChatTransport,
        store: &mut impl ProtectedMutationStore,
        channel: RtChannelId,
        text: &str,
        submission: Option<&ChatSubmission>,
    ) -> Result<(ChatOperation, RtChannelMetadata)> {
        if text.is_empty() || text.len() > RT_MAX_BODY_BYTES {
            return Err(Error::ChatInvalidInput("invalid message text length"));
        }
        self.refresh()?;
        let md = self.channel(rpc, channel, true)?;
        let RealtimeResponse::Messages(recent) =
            rpc.request(&RealtimeRequest::Recents(RtRecentsArgument {
                channel,
                limit: 1,
                stop_at: 0,
            }))?
        else {
            return Err(Error::ChatIntegrity("unexpected recent response"));
        };
        if recent.messages.len() > 1 {
            return Err(Error::ChatIntegrity("recent response exceeds request"));
        }
        let history = self.verify_page(rpc, &md, recent.messages)?;
        let previous = history.messages.first().map(|m| &m.message);
        if history
            .messages
            .iter()
            .any(|m| matches!(m.content, crate::ChatContent::Unsupported(_)))
        {
            return Err(Error::ChatUnsupported(
                "latest message has unsupported content",
            ));
        }
        let lower = previous.map_or(0, |m| m.sequence);
        let id = random_id()?;
        let metadata = RtMessageMetadata {
            id: RtMessageId(id),
            previous_id: previous.map_or(RtMessageId([0; 16]), |m| m.metadata.id),
            previous_sequence: lower,
            send_time: crate::now_microseconds()? / 1000,
            kind: RtMessageType::Basic,
            further_user_attribution: None,
        };
        let key = self.current(md.roles.read)?;
        let ciphertext = self.keys(key)?.seal_basic_message(
            &self.noncer(channel, metadata.clone(), self.credential.uid.clone()),
            text.as_bytes(),
        )?;
        let op = self.prepare(
            store,
            id,
            submission,
            channel,
            lower,
            RealtimeRequest::Send(RtSendArgument {
                send: RtSend {
                    metadata,
                    channel: channel.short(),
                    wrapper: RtMessageWrapper::Encrypted(RtMessageBox { key, ciphertext }),
                    expected_previous_sequence: 0,
                },
            }),
        )?;
        Ok((op, md))
    }
}
