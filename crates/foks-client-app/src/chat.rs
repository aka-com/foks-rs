//! Foreground chat workflows. Preparation and submission are separate checked
//! operations so the external checkpoint is published before network delivery.
use crate::{
    derive_mutation_key, AccountVault, Capability, CheckedProfileSession, ClientCredentials, Error,
    Result,
};
use foks_client::{ChatChannels, ChatHistory, ChatSession, EncryptedFileMutationStore};
use foks_client_db::{ChatOperation, HardStateStore};
use foks_proto::{EntityId, RtChannelId, RtChannelTier};

impl CheckedProfileSession<'_> {
    fn with_chat<T>(
        &self,
        team_alias: &str,
        vault: &mut AccountVault<'_>,
        operation: impl FnOnce(&mut ChatSession<'_>) -> Result<T>,
    ) -> Result<T> {
        self.profile.require(Capability::Chat)?;
        let stored = vault.team(team_alias)?;
        if !stored.active {
            return Err(Error::InvalidAccount("team creation is pending"));
        }
        let account = vault.account(&stored.account_alias)?;
        let host = self.pinned_host()?;
        let mut chat = self.client.chat_session(
            &host,
            &account.credential,
            &EntityId::from_bytes(stored.team_id.clone())?,
        )?;
        operation(&mut chat)
    }
    pub fn list_chat_channels(
        &self,
        team_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<ChatChannels> {
        self.with_chat(team_alias, vault, |chat| {
            Ok(chat.list_channels(&mut chat.connection()?)?)
        })
    }
    pub fn read_recent_chat(
        &self,
        team_alias: &str,
        channel: RtChannelId,
        limit: u64,
        vault: &mut AccountVault<'_>,
    ) -> Result<ChatHistory> {
        self.with_chat(team_alias, vault, |chat| {
            Ok(chat.read_recent(&mut chat.connection()?, channel, limit)?)
        })
    }
    pub fn read_chat_thread(
        &self,
        team_alias: &str,
        channel: RtChannelId,
        start: u64,
        end: u64,
        vault: &mut AccountVault<'_>,
    ) -> Result<ChatHistory> {
        self.with_chat(team_alias, vault, |chat| {
            Ok(chat.read_thread(&mut chat.connection()?, channel, start, end)?)
        })
    }
    pub fn prepare_chat_channel(
        &self,
        team_alias: &str,
        name: &str,
        description: &str,
        tier: RtChannelTier,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<ChatOperation> {
        self.with_chat(team_alias, vault, |chat| {
            let mut protected = EncryptedFileMutationStore::open(
                &self.paths.protected_mutations,
                derive_mutation_key(master_key),
            )?;
            Ok(chat.prepare_channel(
                &mut chat.connection()?,
                &mut protected,
                name,
                description,
                tier,
            )?)
        })
    }
    pub fn prepare_chat_send(
        &self,
        team_alias: &str,
        channel: RtChannelId,
        text: &str,
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<ChatOperation> {
        self.with_chat(team_alias, vault, |chat| {
            let mut protected = EncryptedFileMutationStore::open(
                &self.paths.protected_mutations,
                derive_mutation_key(master_key),
            )?;
            Ok(chat.prepare_send(&mut chat.connection()?, &mut protected, channel, text)?)
        })
    }
    /// Publishes the prepared ledger checkpoint before delivery. An uncertain
    /// operation only reconciles; it is never blindly resubmitted.
    pub fn attempt_chat_operation(
        &self,
        credentials: &ClientCredentials,
        team_alias: &str,
        id: &[u8; 16],
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<ChatOperation> {
        credentials.checkpoint_before_chat_delivery(self)?;
        self.with_chat(team_alias, vault, |chat| {
            let mut protected = EncryptedFileMutationStore::open(
                &self.paths.protected_mutations,
                derive_mutation_key(master_key),
            )?;
            Ok(chat.attempt_operation(&mut chat.connection()?, &mut protected, id)?)
        })
    }
    pub fn cancel_prepared_chat_operation(
        &self,
        team_alias: &str,
        id: &[u8; 16],
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<ChatOperation> {
        self.with_chat(team_alias, vault, |chat| {
            let mut protected = EncryptedFileMutationStore::open(
                &self.paths.protected_mutations,
                derive_mutation_key(master_key),
            )?;
            Ok(chat.cancel_prepared_operation(&mut protected, id)?)
        })
    }
    pub fn finalize_chat_operation(
        &self,
        team_alias: &str,
        id: &[u8; 16],
        vault: &mut AccountVault<'_>,
        master_key: &[u8; 32],
    ) -> Result<ChatOperation> {
        self.with_chat(team_alias, vault, |chat| {
            let mut protected = EncryptedFileMutationStore::open(
                &self.paths.protected_mutations,
                derive_mutation_key(master_key),
            )?;
            Ok(chat.finalize_operation(&mut protected, id)?)
        })
    }
    pub fn list_pending_chat(
        &self,
        team_alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<Vec<ChatOperation>> {
        self.profile.require(Capability::Chat)?;
        let stored = vault.team(team_alias)?;
        let account = vault.account(&stored.account_alias)?;
        let host = self.pinned_host()?;
        Ok(
            HardStateStore::open(&self.paths.hard_database)?.chat_pending(
                host.host_id().as_bytes(),
                account.credential.uid.as_bytes(),
                &stored.team_id,
            )?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{AccountVault, ClientCredentials, Error, RtChannelId, RtChannelTier};
    use crate::{
        derive_vault_key, CredentialBackend, Profile, ProfileRegistry, ProfileSession,
        ProtocolPolicy, TrustRoot,
    };
    use foks_keystore::EncryptedFileSecretStore;
    use foks_server_testkit::TestEnvironment;
    #[test]
    fn checked_chat_workflows_prepare_checkpoint_and_send() {
        let environment = TestEnvironment::new().unwrap();
        let _server = environment.start_server().unwrap();
        let addresses = environment.addresses().unwrap();
        let state = environment.client_path("app-chat", "state").unwrap();
        let root = environment.client_path("app-chat", "root.der").unwrap();
        environment.write_probe_root(&root).unwrap();
        ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".into(),
                probe: format!("localhost:{}", addresses.probe.port()),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::CertificateDer { path: root },
            })
            .unwrap();
        let profile = ProfileSession::open(&registry, "local").unwrap();
        let credentials = ClientCredentials::open(&state).unwrap();
        let master = credentials.master_key().unwrap();
        let wrong_root = environment.client_path("app-chat-wrong", "state").unwrap();
        ClientCredentials::initialize(&wrong_root, CredentialBackend::PrivateFile).unwrap();
        let wrong = ClientCredentials::open(&wrong_root).unwrap();
        credentials.with_checked_session(&profile,|session| {
            session.probe_and_pin()?;
            let mut store=EncryptedFileSecretStore::open(&session.paths().credential_store,derive_vault_key(&master))?;let mut vault=AccountVault::new(&mut store);
            session.create_account("owner","appchatowner","laptop","owner@example.test","",None,&mut vault,&master)?;
            session.create_named_team("owner","chat-team","app-chat-team",&mut vault,&master)?;
            let op=session.prepare_chat_channel("chat-team","","",RtChannelTier::Bottom,&mut vault,&master)?;
            assert!(session.attempt_chat_operation(&wrong,"chat-team",&op.id,&mut vault,&master).is_err());
            assert!(session.list_chat_channels("chat-team",&mut vault)?.channels.is_empty());
            let created=session.attempt_chat_operation(&credentials,"chat-team",&op.id,&mut vault,&master)?;
            assert_eq!(created.state,foks_client_db::ChatOperationState::Confirmed);
            let send=session.prepare_chat_send("chat-team",RtChannelId(op.scope.channel),"hello from the app",&mut vault,&master)?;
            session.attempt_chat_operation(&credentials,"chat-team",&send.id,&mut vault,&master)?;
            let recent=session.read_recent_chat("chat-team",RtChannelId(op.scope.channel),10,&mut vault)?;
            assert!(matches!(&recent.messages[0].content,foks_client::ChatContent::Text(text) if text.as_str()=="hello from the app"));
            assert!(session.list_pending_chat("chat-team",&mut vault)?.is_empty());
            Ok::<_,Error>(())
        }).unwrap();
    }
}
