use super::*;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotSelection {
    pub alias: String,
    pub username: String,
    pub host_id: Vec<u8>,
    pub uid: Vec<u8>,
    pub device_id: Vec<u8>,
}
fn selection_key(alias: &str) -> String {
    format!("bot-account.{alias}")
}
fn copy_account(a: &LoadedAccount) -> LoadedAccount {
    LoadedAccount {
        alias: a.alias.clone(),
        username: a.username.clone(),
        credential: DeviceCredential {
            key_kind: a.credential.key_kind,
            uid: a.credential.uid.clone(),
            seed: SecretSeed::new(*a.credential.seed.as_bytes()),
            certificate_chain: a.credential.certificate_chain.clone(),
        },
    }
}
impl AccountVault<'_> {
    pub fn bot_selection(&mut self, alias: &str) -> Result<Option<BotSelection>> {
        validate_name(alias)?;
        let raw = match self.store.get(&selection_key(alias)) {
            Ok(v) => v,
            Err(foks_keystore::Error::Missing) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let p: BotSelection = serde_json::from_slice(&raw)?;
        if p.alias != alias || p.username.is_empty() || p.username.len() > 256 {
            return Err(Error::InvalidAccount("invalid bot selection"));
        }
        EntityId::from_bytes(p.host_id.clone())?.require_type(foks_proto::ENTITY_HOST)?;
        EntityId::from_bytes(p.uid.clone())?.require_type(ENTITY_USER)?;
        EntityId::from_bytes(p.device_id.clone())?
            .require_type(foks_proto::ENTITY_BOT_TOKEN_KEY)?;
        Ok(Some(p))
    }
    pub fn attach_loaded_bot(&mut self, account: LoadedAccount) -> Result<()> {
        let p = self
            .bot_selection(&account.alias)?
            .ok_or(Error::BotTokenLocked)?;
        if account.credential.key_kind != foks_client::SoftwareKeyKind::BotToken
            || account.credential.uid.as_bytes() != p.uid
            || account.credential.public_material()?.id.as_bytes() != p.device_id
        {
            return Err(Error::InvalidAccount(
                "loaded bot differs from selected credential",
            ));
        }
        self.loaded_bots.insert(account.alias.clone(), account);
        Ok(())
    }
    pub(crate) fn loaded_bot(&mut self, alias: &str) -> Result<LoadedAccount> {
        let p = self.bot_selection(alias)?.ok_or(Error::AccountMissing)?;
        let mut account = self
            .loaded_bots
            .get(alias)
            .map(copy_account)
            .ok_or(Error::BotTokenLocked)?;
        account.username = p.username;
        Ok(account)
    }
    pub fn account_display_name(&mut self, alias: &str) -> Result<String> {
        if let Some(p) = self.bot_selection(alias)? {
            return Ok(p.username);
        }
        Ok(self.account(alias)?.username)
    }
    pub(crate) fn update_bot_label(&mut self, alias: &str, username: &str) -> Result<()> {
        let mut p = self.bot_selection(alias)?.ok_or(Error::AccountMissing)?;
        p.username = username.into();
        self.store.put(
            &selection_key(alias),
            &Zeroizing::new(serde_json::to_vec(&p)?),
        )?;
        Ok(())
    }
}
impl CheckedProfileSession<'_> {
    /// Returns the loaded account to the resident caller. It is never written to the vault.
    pub fn load_bot_account(
        &self,
        alias: &str,
        input: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<LoadedAccount> {
        self.profile.require(Capability::DeviceAdministration)?;
        validate_name(alias)?;
        let selected = vault.bot_selection(alias)?;
        if selected.is_none() && vault.contains(alias)? {
            return Err(Error::AccountExists);
        }
        let token = foks_crypto::BotToken::import(input).map_err(|_| Error::BotToken)?;
        let host = self.pinned_host()?;
        let credential = self.client.load_bot_token(&host, &token)?;
        let user = self
            .client
            .authenticate_and_pin(&host, &credential)?
            .verified;
        let username =
            String::from_utf8(user.username_utf8().to_vec()).map_err(|_| Error::BotToken)?;
        let p = BotSelection {
            alias: alias.into(),
            username: username.clone(),
            host_id: host.host_id().as_bytes().to_vec(),
            uid: credential.uid.as_bytes().to_vec(),
            device_id: credential.public_material()?.id.as_bytes().to_vec(),
        };
        if selected.is_some_and(|old| {
            old.host_id != p.host_id || old.uid != p.uid || old.device_id != p.device_id
        }) {
            return Err(Error::InvalidAccount(
                "token does not match the original selected bot",
            ));
        }
        vault.store.put(
            &selection_key(alias),
            &Zeroizing::new(serde_json::to_vec(&p)?),
        )?;
        Ok(LoadedAccount {
            alias: alias.into(),
            username,
            credential,
        })
    }
}
