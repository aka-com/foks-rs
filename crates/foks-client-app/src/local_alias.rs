//! Editable display aliases, separate from immutable local command selectors.
use super::*;

pub fn validate_local_alias(label: &str) -> Result<()> {
    if label.is_empty()
        || label != label.trim()
        || label.len() > 64
        || label.chars().any(char::is_control)
    {
        return Err(Error::InvalidAccount("local alias must be 1–64 UTF-8 bytes without surrounding whitespace or control characters"));
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalAlias {
    version: u32,
    account_alias: String,
    uid: Vec<u8>,
    label: String,
}
fn key(alias: &str) -> String {
    format!("account-local-alias.{alias}")
}

impl AccountVault<'_> {
    fn local_alias_uid(&mut self, alias: &str) -> Result<Vec<u8>> {
        validate_name(alias)?;
        if let Some(bot) = self.bot_selection(alias)? {
            return Ok(bot.uid);
        }
        match self.account(alias) {
            Ok(account) => Ok(account.credential.uid.as_bytes().to_vec()),
            Err(Error::AccountMissing) => Ok(self.yubi_account(alias)?.uid.as_bytes().to_vec()),
            Err(error) => Err(error),
        }
    }

    pub fn local_account_alias(&mut self, alias: &str) -> Result<Option<String>> {
        let uid = self.local_alias_uid(alias)?;
        let bytes = match self.store.get(&key(alias)) {
            Ok(bytes) => bytes,
            Err(foks_keystore::Error::Missing) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let stored: LocalAlias = serde_json::from_slice(&bytes)?;
        if stored.version != 1 || stored.account_alias != alias {
            return Err(Error::InvalidAccount("local alias binding changed"));
        }
        validate_local_alias(&stored.label)?;
        if stored.uid != uid {
            return Ok(None);
        }
        Ok(Some(stored.label))
    }

    pub fn set_local_account_alias(&mut self, alias: &str, label: &str) -> Result<()> {
        validate_local_alias(label)?;
        let uid = self.local_alias_uid(alias)?;
        for other in self.aliases()? {
            if other != alias
                && self
                    .local_account_alias(&other)?
                    .as_deref()
                    .unwrap_or(&other)
                    == label
            {
                return Err(Error::InvalidAccount(
                    "another account already uses this local alias",
                ));
            }
        }
        if label == alias {
            self.store.remove(&key(alias))?;
        } else {
            let stored = LocalAlias {
                version: 1,
                account_alias: alias.to_owned(),
                uid,
                label: label.to_owned(),
            };
            self.store
                .put(&key(alias), &Zeroizing::new(serde_json::to_vec(&stored)?))?;
        }
        Ok(())
    }
}
