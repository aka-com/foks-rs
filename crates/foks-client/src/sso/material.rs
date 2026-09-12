use super::*;
pub(super) struct Material {
    pub(super) device: EntityId,
    pub(super) expires_at_ms: u64,
    pub(super) config: SsoConfig,
    pub(super) binding: OAuth2Binding,
    pub(super) init: InitOAuth2SessionArgument,
    pub(super) issuer: String,
}
impl Material {
    pub(super) fn encoded(&self) -> Result<Zeroizing<Vec<u8>>> {
        Ok(Zeroizing::new(encode(&Value::Array(vec![
            Value::Binary(self.config.encoded()?),
            Value::Binary(self.binding.encoded()?),
            Value::Binary(self.init.encoded()?),
            Value::Text(self.issuer.as_bytes().to_vec()),
            Value::Binary(self.device.as_bytes().to_vec()),
            Value::Unsigned(self.expires_at_ms),
        ]))?))
    }
    pub(super) fn decode(bytes: &[u8]) -> Result<Self> {
        let v = decode(bytes)?;
        let Value::Array(f) = v else {
            return Err(Error::Sso("invalid protected session"));
        };
        let [Value::Binary(config), Value::Binary(binding), Value::Binary(init), Value::Text(issuer), Value::Binary(device), Value::Unsigned(expires_at_ms)] =
            f.as_slice()
        else {
            return Err(Error::Sso("invalid protected session fields"));
        };
        Ok(Self {
            device: EntityId::from_bytes(device.clone())?,
            expires_at_ms: *expires_at_ms,
            config: SsoConfig::decode(config)?,
            binding: OAuth2Binding::decode(binding)?,
            init: InitOAuth2SessionArgument::decode(init)?,
            issuer: String::from_utf8(issuer.clone()).map_err(|_| Error::Sso("invalid issuer"))?,
        })
    }
}
pub(super) fn material_key(id: &[u8; 16], stage: u8) -> Vec<u8> {
    [b"foks-client-sso-v1".as_slice(), id, &[stage]].concat()
}
pub(super) fn config_hash(config: &SsoConfig) -> Result<[u8; 32]> {
    Ok(foks_crypto::prefixed_hash(
        CONFIG_HASH,
        &config.public().encoded()?,
    ))
}
pub(super) fn get(
    store: &mut impl ProtectedMutationStore,
    id: &[u8; 16],
    stage: u8,
) -> Result<Zeroizing<Vec<u8>>> {
    store
        .get(&material_key(id, stage))
        .map_err(|_| Error::Sso("protected session material unavailable"))
}
pub(super) fn put(
    store: &mut impl ProtectedMutationStore,
    id: &[u8; 16],
    stage: u8,
    bytes: &[u8],
) -> Result<()> {
    store
        .put_if_absent(&material_key(id, stage), bytes)
        .map_err(|_| Error::Sso("protected session material could not be retained"))
}
pub(super) fn public_flow(host: &PinnedHost, id: &[u8; 16]) -> Result<SsoFlow> {
    let flow = HardStateStore::open(&host.database_path)?
        .sso_flow(id)?
        .ok_or(Error::Sso("unknown local session"))?;
    if flow.host != host.host_id().as_bytes() {
        return Err(Error::Sso("session host mismatch"));
    }
    Ok(flow)
}
pub(super) fn erase_flow_material(
    store: &mut impl ProtectedMutationStore,
    id: &[u8; 16],
) -> Result<()> {
    for stage in 0..=3 {
        match store.remove(&material_key(id, stage)) {
            Ok(()) | Err(crate::ProtectedStoreError::Missing) => {}
            Err(_) => return Err(Error::Sso("terminal session cleanup failed")),
        }
    }
    Ok(())
}
pub(super) fn load(
    host: &PinnedHost,
    id: &[u8; 16],
    store: &mut impl ProtectedMutationStore,
) -> Result<(SsoFlow, Material)> {
    let flow = HardStateStore::open(&host.database_path)?
        .sso_flow(id)?
        .ok_or(Error::Sso("unknown local session"))?;
    if flow.host != host.host_id().as_bytes() {
        return Err(Error::Sso("session host mismatch"));
    }
    let bytes = get(store, id, 0)?;
    if foks_crypto::prefixed_hash(MATERIAL_HASH, &bytes) != flow.material_hash {
        return Err(Error::Sso("protected session fingerprint mismatch"));
    }
    let m = Material::decode(&bytes)?;
    if m.binding.uid.as_bytes() != flow.uid
        || m.binding.host != *host.host_id()
        || m.device.as_bytes() != flow.device
        || m.expires_at_ms != flow.expires_at_ms
        || config_hash(&m.config)? != flow.config_hash
        || m.init.uid.as_ref().map(EntityId::as_bytes)
            != flow.for_login.then_some(flow.uid.as_slice())
        || foks_crypto::oauth2_binding_nonce(&m.binding)? != m.init.nonce.expose()
    {
        return Err(Error::Sso("protected session scope mismatch"));
    }
    Ok((flow, m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EncryptedFileMutationStore;
    #[test]
    fn protected_flow_binds_all_public_scope_fields_and_terminal_cleanup_is_retryable() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("hard.sqlite3");
        let verified = foks_verify::verify_public_host(
            "foks.app",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../foks-snowpack/tests/fixtures/foks-v0.1.9/foks.app/probe-response.snowp"
            )),
        )
        .unwrap();
        let mut db = HardStateStore::open(&path).unwrap();
        db.accept_verified_host(&verified.snapshot).unwrap();
        let client = FoksClient::webpki();
        let host = client.pinned_host("foks.app", &path).unwrap();
        let fixture = |name: &str| {
            std::fs::read(format!(
                "../foks-snowpack/tests/fixtures/foks-v0.1.9/sso/{name}.snowp"
            ))
            .unwrap()
        };
        let mut binding = OAuth2Binding::decode(&fixture("binding")).unwrap();
        binding.host = host.host_id().clone();
        let mut init = InitOAuth2SessionArgument::decode(&fixture("init-signup")).unwrap();
        init.nonce = OAuth2Secret::new(foks_crypto::oauth2_binding_nonce(&binding).unwrap());
        let device = foks_proto::OAuth2IdTokenBinding::decode(&fixture("signed-binding"))
            .unwrap()
            .key;
        let m = Material {
            config: SsoConfig::decode(&fixture("public-config")).unwrap(),
            binding,
            init,
            issuer: "https://idp.example".into(),
            device,
            expires_at_ms: 1,
        };
        let bytes = m.encoded().unwrap();
        let id = [3; 16];
        let mut store = EncryptedFileMutationStore::open(
            temp.path().join("protected"),
            Zeroizing::new([4; 32]),
        )
        .unwrap();
        let flow = SsoFlow {
            id,
            host: host.host_id().as_bytes().to_vec(),
            uid: m.binding.uid.as_bytes().to_vec(),
            device: m.device.as_bytes().to_vec(),
            for_login: false,
            state: SsoFlowState::Prepared,
            material_hash: foks_crypto::prefixed_hash(MATERIAL_HASH, &bytes),
            config_hash: config_hash(&m.config).unwrap(),
            expires_at_ms: 1,
            final_operation: None,
        };
        db.sso_record_with_material::<Error>(&flow, 0, || put(&mut store, &id, 0, &bytes))
            .unwrap();
        load(&host, &id, &mut store).unwrap();
        let sql = rusqlite::Connection::open(&path).unwrap();
        for (change, restore) in [
            ("expires_at=2", "expires_at=1"),
            ("for_login=1", "for_login=0"),
            ("device_id=zeroblob(33)", "device_id=?2"),
            ("config_hash=zeroblob(32)", "config_hash=?2"),
        ] {
            sql.execute(
                &format!("UPDATE sso_flows SET {change} WHERE operation_id=?1"),
                [id.as_slice()],
            )
            .unwrap();
            assert!(load(&host, &id, &mut store).is_err(), "{change}");
            if restore.contains("?2") {
                let original = if change.starts_with("device") {
                    flow.device.as_slice()
                } else {
                    flow.config_hash.as_slice()
                };
                sql.execute(
                    &format!("UPDATE sso_flows SET {restore} WHERE operation_id=?1"),
                    rusqlite::params![id.as_slice(), original],
                )
                .unwrap();
            } else {
                sql.execute(
                    &format!("UPDATE sso_flows SET {restore} WHERE operation_id=?1"),
                    [id.as_slice()],
                )
                .unwrap();
            }
        }
        put(&mut store, &id, 2, b"sensitive token result").unwrap();
        assert_eq!(
            client.sso_progress(&host, id, &mut store).unwrap().state,
            SsoFlowState::Expired
        );
        assert!(matches!(
            store.get(&material_key(&id, 0)),
            Err(crate::ProtectedStoreError::Missing)
        ));
        assert!(matches!(
            store.get(&material_key(&id, 2)),
            Err(crate::ProtectedStoreError::Missing)
        ));
        assert_eq!(
            client.sso_progress(&host, id, &mut store).unwrap().state,
            SsoFlowState::Expired
        );
        let progress = SsoProgress {
            operation_id: id,
            state: SsoFlowState::AwaitingBrowser,
            browser_url: Some("https://host/o/secret-session".into()),
            expires_at_ms: 1,
        };
        assert!(!format!("{progress:?}").contains("secret-session"));
    }
}
