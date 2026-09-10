use super::policy::ChatLimits;
use super::RealtimeConnection;
use crate::{AuthenticatedTeamOutcome, DeviceCredential, Error, FoksClient, PinnedHost, Result};
use foks_client_db::{ChatScope, HardStateStore};
use foks_crypto::{derive_realtime_keys, RealtimeKeys};
use foks_proto::{
    EntityId, FqParty, Role, RoleAndGeneration, RtAppId, RtChannelId, RtChannelMetadata,
    RtChannelTier, RtHostId, RtKeyType, RtListChannelsArgument, RtMessageMetadata, RtMessageNoncer,
    RtTeamId, RtText, RtUserId, ENTITY_NAMED_TEAM,
};
use foks_rpc::{RealtimeRequest, RealtimeResponse};

/// Transport seam for fault injection and the real pinned RT connection.
pub trait ChatTransport {
    fn request(&mut self, request: &RealtimeRequest) -> Result<RealtimeResponse>;
}
impl ChatTransport for RealtimeConnection {
    fn request(&mut self, request: &RealtimeRequest) -> Result<RealtimeResponse> {
        self.call(request)
    }
}
pub struct ChatSession<'a> {
    pub(super) client: &'a FoksClient,
    pub(super) host: &'a PinnedHost,
    pub(super) credential: &'a DeviceCredential,
    team: Option<AuthenticatedTeamOutcome>,
    pub(super) team_id: EntityId,
    pub(super) role: Role,
}
pub struct ChatChannel {
    pub metadata: RtChannelMetadata,
    pub name: RtText,
    pub description: Option<RtText>,
}
pub struct ChatChannels {
    pub host: RtHostId,
    pub actor: RtUserId,
    pub team: RtTeamId,
    pub version: u64,
    pub channels: Vec<ChatChannel>,
}
impl FoksClient {
    /// Binds identity without network I/O. Each network workflow authenticates once.
    pub fn chat_session<'a>(
        &'a self,
        host: &'a PinnedHost,
        credential: &'a DeviceCredential,
        team: &EntityId,
    ) -> Result<ChatSession<'a>> {
        if team.entity_type() != ENTITY_NAMED_TEAM {
            return Err(Error::ChatUnsupported("chat requires a named team"));
        }
        Ok(ChatSession {
            client: self,
            host,
            credential,
            team_id: team.clone(),
            team: None,
            role: Role::NONE,
        })
    }
}
impl ChatSession<'_> {
    pub fn connection(&self) -> Result<RealtimeConnection> {
        self.client.realtime_connection(self.host, self.credential)
    }
    pub(super) fn refresh(&mut self) -> Result<()> {
        // Drop prior private keys before refreshing, including on failure.
        self.team = None;
        self.role = Role::NONE;
        for _ in 0..ChatLimits::AUTHENTICATION_ATTEMPTS {
            let user = self
                .client
                .authenticate_and_pin(self.host, self.credential)?;
            let loaded = self.client.load_and_pin_team(
                self.host,
                self.credential,
                &user.verified,
                &user.puks,
                &self.team_id,
            )?;
            if user.verified.tree_root() != loaded.verified.tree_root() {
                continue;
            }
            if loaded.verified.host() != self.host.host_id() {
                return Err(Error::ChatIntegrity("team host mismatch"));
            }
            let member = loaded
                .verified
                .members()
                .iter()
                .find(|m| {
                    m.party == self.credential.uid
                        && m.source_role == Role::OWNER
                        && m.scoped_host
                            .as_ref()
                            .is_none_or(|h| h == self.host.host_id())
                })
                .ok_or(Error::ChatAccessDenied(
                    "chat requires direct local source-owner membership",
                ))?;
            self.role = member.role;
            self.team = Some(loaded);
            return Ok(());
        }
        Err(Error::ChatRefreshRequired(
            "team and actor roots changed during refresh",
        ))
    }
    fn team(&self) -> Result<&AuthenticatedTeamOutcome> {
        self.team.as_ref().ok_or(Error::ChatOperationState(
            "chat operation is not authenticated",
        ))
    }
    pub(super) fn scope(&self, channel: RtChannelId) -> ChatScope {
        ChatScope {
            host: self.host.host_id().as_bytes().to_vec(),
            uid: self.credential.uid.as_bytes().to_vec(),
            team: self.team_id.as_bytes().to_vec(),
            channel: channel.0,
        }
    }
    pub(super) fn keys(&self, key: RoleAndGeneration) -> Result<RealtimeKeys> {
        let ptk = self
            .team()?
            .ptks
            .iter()
            .find(|k| k.role == key.role && k.generation == key.generation)
            .ok_or(Error::ChatKeyUnavailable(
                "verified historical chat key unavailable",
            ))?;
        Ok(derive_realtime_keys(&ptk.seed, RtAppId::Chat)?)
    }
    pub(super) fn current(&self, role: Role) -> Result<RoleAndGeneration> {
        let generation = self
            .team()?
            .verified
            .shared_key(role)
            .ok_or(Error::ChatKeyUnavailable("chat role key unavailable"))?
            .generation;
        let key = RoleAndGeneration { role, generation };
        self.keys(key)?;
        Ok(key)
    }
    pub(super) fn validate_channel(&self, md: &RtChannelMetadata) -> Result<()> {
        if md.team.entity() != &self.team_id
            || md.app != RtAppId::Chat
            || md.id.0 == [0; 16]
            || md.sequence == 0
            || md.roles.read < floor(md.tier)?
            || md.roles.write < md.roles.read
            || md.name.key.role != floor(md.tier)?
            || md
                .description
                .as_ref()
                .is_some_and(|d| d.key.role != md.roles.read)
        {
            return Err(Error::ChatIntegrity("invalid channel identity or policy"));
        }
        if self.role < floor(md.tier)? {
            return Err(Error::ChatAccessDenied("channel tier is inaccessible"));
        }
        Ok(())
    }
    pub fn list_channels(&mut self, rpc: &mut impl ChatTransport) -> Result<ChatChannels> {
        self.refresh()?;
        self.list_current_channels(rpc)
    }
    pub(super) fn list_current_channels(
        &self,
        rpc: &mut impl ChatTransport,
    ) -> Result<ChatChannels> {
        let RealtimeResponse::Channels(set) =
            rpc.request(&RealtimeRequest::ListChannels(RtListChannelsArgument {
                team: RtTeamId::new(self.team_id.clone())?,
                app: RtAppId::Chat,
                last: 0,
            }))?
        else {
            return Err(Error::ChatIntegrity("unexpected channel response"));
        };
        if set.channels.len() > ChatLimits::CHANNELS || set.version > i64::MAX as u64 {
            return Err(Error::ChatLimit("channel set limit"));
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut shorts = std::collections::BTreeSet::new();
        let mut channels = Vec::new();
        for md in set.channels {
            self.validate_channel(&md)?;
            if !ids.insert(md.id.0)
                || !shorts.insert(md.id.short().get())
                || md.updated_at == 0
                || md.updated_at > set.version
            {
                return Err(Error::ChatIntegrity("duplicate channel or invalid version"));
            }
            let name = self
                .keys(md.name.key)?
                .open_text(RtKeyType::ChannelName, &md.name.boxed)?;
            let readable = self.role >= md.roles.read && !md.unreadable;
            let description = if readable {
                md.description
                    .as_ref()
                    .map(|b| {
                        self.keys(b.key)?
                            .open_text(RtKeyType::ChannelDescription, &b.boxed)
                            .map_err(Error::from)
                    })
                    .transpose()?
            } else {
                None
            };
            let mut md = md;
            if !readable {
                md.description = None;
                md.last_message = None;
                md.mtime = md.ctime;
                md.unreadable = true;
            }
            channels.push(ChatChannel {
                metadata: md,
                name,
                description,
            });
        }
        Ok(ChatChannels {
            host: RtHostId::new(self.host.host_id().clone())?,
            actor: RtUserId::new(self.credential.uid.clone())?,
            team: RtTeamId::new(self.team_id.clone())?,
            version: set.version,
            channels,
        })
    }
    pub(super) fn channel(
        &self,
        rpc: &mut impl ChatTransport,
        id: RtChannelId,
        write: bool,
    ) -> Result<RtChannelMetadata> {
        let md = self
            .list_current_channels(rpc)?
            .channels
            .into_iter()
            .find(|c| c.metadata.id == id)
            .ok_or(Error::ChatNotFound("channel unavailable"))?
            .metadata;
        if md.unreadable || self.role < md.roles.read || (write && self.role < md.roles.write) {
            return Err(Error::ChatAccessDenied("channel access denied"));
        }
        Ok(md)
    }
    pub(super) fn noncer(
        &self,
        channel: RtChannelId,
        metadata: RtMessageMetadata,
        sender: EntityId,
    ) -> RtMessageNoncer {
        RtMessageNoncer {
            metadata,
            sender: Some(FqParty {
                host: self.host.host_id().clone(),
                party: sender,
            }),
            app: RtAppId::Chat,
            team: FqParty {
                host: self.host.host_id().clone(),
                party: self.team_id.clone(),
            },
            channel,
        }
    }
    pub(super) fn hard(&self) -> Result<HardStateStore> {
        Ok(HardStateStore::open(&self.host.database_path)?)
    }
}
pub(super) fn floor(tier: RtChannelTier) -> Result<Role> {
    match tier {
        RtChannelTier::Bottom => Ok(Role::member(-0x4000)),
        RtChannelTier::Admin => Ok(Role::ADMIN),
        _ => Err(Error::ChatUnsupported("unsupported chat tier")),
    }
}
pub(super) fn random_id() -> Result<[u8; 16]> {
    let mut id = [0; 16];
    getrandom::fill(&mut id).map_err(|_| Error::ChatRandomness("chat randomness unavailable"))?;
    if id == [0; 16] {
        return Err(Error::ChatRandomness("zero random ID"));
    }
    Ok(id)
}
