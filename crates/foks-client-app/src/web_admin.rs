use super::*;
use foks_client::AdminDestination;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    host: Vec<u8>,
    uid: Vec<u8>,
    destination: String,
}
fn policy_key(alias: &str) -> String {
    format!("web-admin.{alias}")
}
pub struct AdminHandoff {
    pub host_id: String,
    pub uid: String,
    pub destination: String,
    pub navigation: foks_client::WebAdminHandoff,
}
impl CheckedProfileSession<'_> {
    pub fn configure_web_admin(
        &self,
        alias: &str,
        destination: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<()> {
        validate_name(alias)?;
        let destination = AdminDestination::new(destination)?;
        let host = self.pinned_host()?;
        let uid = match vault.account(alias) {
            Ok(a) => a.credential.uid,
            Err(Error::AccountMissing) => vault.yubi_account(alias)?.uid,
            Err(e) => return Err(e),
        };
        let policy = Policy {
            host: host.host_id().as_bytes().to_vec(),
            uid: uid.as_bytes().to_vec(),
            destination: destination.as_str().into(),
        };
        vault.store.put(
            &policy_key(alias),
            &Zeroizing::new(serde_json::to_vec(&policy)?),
        )?;
        Ok(())
    }
    pub fn web_admin_policy(
        &self,
        alias: &str,
        vault: &mut AccountVault<'_>,
    ) -> Result<(String, String, String)> {
        validate_name(alias)?;
        let host = self.pinned_host()?;
        let bytes = vault.store.get(&policy_key(alias)).map_err(|e| {
            if matches!(e, foks_keystore::Error::Missing) {
                Error::InvalidAccount(
                    "configure the HTTPS admin destination supplied by your host operator",
                )
            } else {
                e.into()
            }
        })?;
        let policy: Policy = serde_json::from_slice(&bytes)?;
        let uid = match vault.account(alias) {
            Ok(a) => a.credential.uid,
            Err(Error::AccountMissing) => vault.yubi_account(alias)?.uid,
            Err(e) => return Err(e),
        };
        if policy.host != host.host_id().as_bytes() || policy.uid != uid.as_bytes() {
            return Err(foks_client::WebAdminError::WrongAccount.into());
        }
        let destination = AdminDestination::new(&policy.destination)?;
        Ok((
            hex(host.host_id().as_bytes()),
            hex(uid.as_bytes()),
            destination.as_str().into(),
        ))
    }
    pub fn web_admin_handoff(
        &self,
        alias: &str,
        parent: Option<&dyn foks_crypto::YubiDevice>,
        vault: &mut AccountVault<'_>,
    ) -> Result<AdminHandoff> {
        self.profile.require(Capability::UserSync)?;
        validate_name(alias)?;
        let host = self.pinned_host()?;
        let policy: Policy = match vault.store.get(&policy_key(alias)) {
            Ok(v) => serde_json::from_slice(&v)?,
            Err(foks_keystore::Error::Missing) => {
                return Err(Error::InvalidAccount(
                    "configure the HTTPS admin destination supplied by your host operator",
                ))
            }
            Err(e) => return Err(e.into()),
        };
        let destination = AdminDestination::new(&policy.destination)?;
        self.with_account_credential(alias, parent, vault, |credential| {
            if policy.host != host.host_id().as_bytes() || policy.uid != credential.uid().as_bytes()
            {
                return Err(foks_client::WebAdminError::WrongAccount.into());
            }
            let navigation = self
                .client
                .new_web_admin_handoff(&host, credential, &destination)?;
            Ok(AdminHandoff {
                host_id: hex(host.host_id().as_bytes()),
                uid: hex(credential.uid().as_bytes()),
                destination: destination.as_str().into(),
                navigation,
            })
        })
    }
}

pub(crate) fn validate_inventory_record(
    vault: &mut AccountVault<'_>,
    alias: &str,
    host: &[u8],
) -> Result<()> {
    let raw = vault.store.get(&policy_key(alias))?;
    let policy: Policy = serde_json::from_slice(&raw)?;
    let uid = match vault.account(alias) {
        Ok(a) => a.credential.uid,
        Err(Error::AccountMissing) => vault.yubi_account(alias)?.uid,
        Err(e) => return Err(e),
    };
    if policy.host != host || policy.uid != uid.as_bytes() {
        return Err(foks_client::WebAdminError::WrongAccount.into());
    }
    AdminDestination::new(&policy.destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::AccountFixture;
    #[test]
    fn configured_rust_host_uses_the_account_bound_native_handoff() {
        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let listen = reservation.local_addr().unwrap();
        drop(reservation);
        let f = AccountFixture::start_with_admin(Some(foks_server_testkit::WebAdminConfig {
            origin: "https://admin.example".into(),
            listen,
        }));
        f.run(|s, v, k| s.create_account("work", "adminenabled", "laptop", "", "", None, v, k));
        f.run(|s, v, _| {
            s.configure_web_admin("work", "https://admin.example/", v)?;
            let handoff = s.web_admin_handoff("work", None, v)?;
            assert_eq!(handoff.destination, "https://admin.example/");
            assert!(handoff
                .navigation
                .expose()
                .starts_with("https://admin.example/?session="));
            Ok(())
        });
    }
    #[test]
    fn admin_policy_is_bound_to_account_and_disabled_server_is_unsupported() {
        let f = AccountFixture::start();
        f.run(|s, v, k| s.create_account("work", "adminowner", "laptop", "", "", None, v, k));
        f.run(|s, v, _| {
            assert!(s.web_admin_policy("work", v).is_err());
            assert!(s
                .configure_web_admin("work", "http://localhost/", v)
                .is_err());
            s.configure_web_admin("work", "https://admin.example/", v)?;
            let p = s.web_admin_policy("work", v)?;
            assert_eq!(p.2, "https://admin.example/");
            assert!(matches!(
                s.web_admin_handoff("work", None, v),
                Err(Error::WebAdmin(foks_client::WebAdminError::Unsupported))
            ));
            Ok(())
        });
        f.run(|s, v, _| {
            let raw = v.store.get(&policy_key("work"))?;
            let mut p: Policy = serde_json::from_slice(&raw)?;
            p.uid[1] ^= 1;
            v.store.put(
                &policy_key("work"),
                &Zeroizing::new(serde_json::to_vec(&p)?),
            )?;
            assert!(matches!(
                s.web_admin_policy("work", v),
                Err(Error::WebAdmin(foks_client::WebAdminError::WrongAccount))
            ));
            Ok(())
        });
    }
}
