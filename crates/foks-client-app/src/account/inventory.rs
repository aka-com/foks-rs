//! Exhaustive vault ownership. No opaque authenticated bytes are transferable.
use super::*;

pub(crate) struct VaultRecordDescriptor {
    pub key: String,
    pub exportable: bool,
}

#[derive(Clone, Copy)]
enum VaultFamily {
    LocalAlias,
    Account,
    YubiAccount,
    BotAccount,
    Backup,
    Pending,
    PendingDevice,
    PendingKex,
    KexOffer,
    PendingRecovery,
    PendingYubi,
    Team,
    TeamRekey,
    TeamMemberEdit,
    FederationExpulsion,
    BotEnrollment,
    InvitationInbox,
    InvitationCompletionRecord,
    WebAdmin,
}
fn parse_key(key: &str) -> Result<(VaultFamily, &str)> {
    let (family, alias) = key
        .split_once('.')
        .ok_or(Error::InvalidAccount("unknown vault record family"))?;
    validate_name(alias)?;
    let family = match family {
        "account-local-alias" => VaultFamily::LocalAlias,
        "account" => VaultFamily::Account,
        "yubi-account" => VaultFamily::YubiAccount,
        "bot-account" => VaultFamily::BotAccount,
        "backup" => VaultFamily::Backup,
        "pending" => VaultFamily::Pending,
        "pending-device" => VaultFamily::PendingDevice,
        "pending-kex" => VaultFamily::PendingKex,
        "kex-offer" => VaultFamily::KexOffer,
        "pending-recovery" => VaultFamily::PendingRecovery,
        "pending-yubi" => VaultFamily::PendingYubi,
        "team" => VaultFamily::Team,
        "team-rekey" => VaultFamily::TeamRekey,
        "team-member-edit" => VaultFamily::TeamMemberEdit,
        "federation-expulsion" => VaultFamily::FederationExpulsion,
        "bot-enrollment" => VaultFamily::BotEnrollment,
        "invitation-inbox" => VaultFamily::InvitationInbox,
        "invitation-receipt" => VaultFamily::InvitationCompletionRecord,
        "web-admin" => VaultFamily::WebAdmin,
        _ => {
            return Err(Error::InvalidAccount(
                "unsupported vault record; complete its owning workflow before portability",
            ))
        }
    };
    Ok((family, alias))
}
/// Reject unknown owners and families that can never be terminal in archive v1.
pub(crate) fn validate_archive_vault_key(key: &str) -> Result<()> {
    match parse_key(key)?.0 {
        VaultFamily::Pending
        | VaultFamily::PendingDevice
        | VaultFamily::PendingKex
        | VaultFamily::KexOffer
        | VaultFamily::PendingRecovery
        | VaultFamily::PendingYubi
        | VaultFamily::TeamRekey
        | VaultFamily::TeamMemberEdit
        | VaultFamily::FederationExpulsion => Err(Error::InvalidAccount(
            "nonterminal vault family cannot be archived",
        )),
        _ => Ok(()),
    }
}

impl AccountVault<'_> {
    pub(crate) fn inventory_record(
        &mut self,
        key: &str,
        hard: &HardStateStore,
        host: &[u8],
    ) -> Result<VaultRecordDescriptor> {
        let (family, alias) = parse_key(key)?;
        let exportable = match family {
            VaultFamily::LocalAlias => {
                self.local_account_alias(alias)?;
                true
            }
            VaultFamily::Account => {
                self.account(alias)?.credential.public_material()?;
                true
            }
            VaultFamily::YubiAccount => self.yubi_record_exportable(alias)?,
            VaultFamily::BotAccount => {
                self.bot_selection(alias)?.ok_or(Error::AccountMissing)?;
                true
            }
            VaultFamily::Backup => {
                self.backup(alias)?.ok_or(Error::AccountMissing)?;
                true
            }
            VaultFamily::Pending => {
                self.pending(alias)?;
                false
            }
            VaultFamily::PendingDevice => {
                self.pending_device(alias)?;
                false
            }
            VaultFamily::PendingKex => {
                self.pending_kex(alias)?;
                false
            }
            VaultFamily::KexOffer => {
                self.kex_offer(alias)?;
                false
            }
            VaultFamily::PendingRecovery => {
                self.pending_recovery(alias)?;
                false
            }
            VaultFamily::PendingYubi => {
                self.validate_pending_yubi_record(alias)?;
                false
            }
            VaultFamily::Team => {
                let team = self.team(alias)?;
                team.active
                    && team.local_members.iter().all(|m| m.active)
                    && team.federated_members.iter().all(|m| m.active)
                    && team.invitation_members.iter().all(|m| m.membership.active)
            }
            VaultFamily::TeamRekey => {
                self.team_rekey(alias)?
                    .ok_or(Error::InvalidAccount("missing team rekey"))?;
                false
            }
            VaultFamily::TeamMemberEdit => {
                self.team_member_edit(alias)?
                    .ok_or(Error::InvalidAccount("missing team edit"))?;
                false
            }
            VaultFamily::FederationExpulsion => {
                self.federation_expulsion(alias)?
                    .ok_or(Error::InvalidAccount("missing federation expulsion"))?;
                false
            }
            VaultFamily::BotEnrollment => {
                crate::bot_token::validate_inventory_record(self, alias, hard, host)?
            }
            VaultFamily::InvitationInbox | VaultFamily::InvitationCompletionRecord => {
                crate::invitations::validate_inventory_record(
                    self,
                    if matches!(family, VaultFamily::InvitationInbox) {
                        "invitation-inbox"
                    } else {
                        "invitation-receipt"
                    },
                    alias,
                    hard,
                )?
            }
            VaultFamily::WebAdmin => {
                crate::web_admin::validate_inventory_record(self, alias, host)?;
                true
            }
        };
        Ok(VaultRecordDescriptor {
            key: key.into(),
            exportable,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_and_malformed_authenticated_vault_records_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let hard = HardStateStore::open(&dir.path().join("hard")).unwrap();
        let mut store = foks_keystore::MemorySecretStore::default();
        store.put("unrecognized.owner", b"authenticated").unwrap();
        store.put("account.owner", b"{}").unwrap();
        let mut vault = AccountVault::new(&mut store);
        assert!(vault
            .inventory_record("unrecognized.owner", &hard, &[])
            .is_err());
        assert!(vault.inventory_record("account.owner", &hard, &[]).is_err());
    }
}
