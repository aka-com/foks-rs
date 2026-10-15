//! Preprovisioned fixture import and immutable encrypted-key recovery.
use super::*;
use foks_server::keys::{DirectoryKeyProvider, KeyGenerationManifest, MANIFEST_PURPOSES};
use serde_json::json;
use std::sync::Arc;

const KEY_FILES: [&str; 8] = [
    "host.key",
    "metadata.key",
    "merkle.key",
    "client-ca.key",
    "delegated-tls.key",
    "recovery.key",
    "capability.key",
    "key-encryption.key",
];
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyFile {
    name: String,
    bytes: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyBundle {
    #[serde(deserialize_with = "bounded::<_, _, 8>")]
    files: Vec<KeyFile>,
}

pub struct FrozenInstallation {
    pub schema: Schema,
    pub snapshot: Snapshot,
    pub host_id: String,
    pub key_bundle: String,
}
impl FrozenInstallation {
    pub fn capture(environment: &crate::TestEnvironment) -> Result<Self> {
        let running = environment
            .inner
            .running
            .lock()
            .map_err(|_| invalid("fixture lifecycle lock"))?;
        if *running {
            return Err(invalid("stop native server before proof import"));
        }
        let connection = Connection::open_with_flags(
            environment.inner.paths.database(),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let schema = Schema::export(&connection)?;
        let snapshot = schema.snapshot(&connection)?;
        let reader = environment.read_database()?;
        let bootstrap = reader
            .host_bootstrap()?
            .ok_or_else(|| invalid("proof fixture has no host"))?;
        let directory = environment.inner.paths.keys();
        for entry in std::fs::read_dir(directory)? {
            let name = entry?.file_name();
            if name != ".key-encryption.lock" && !KEY_FILES.iter().any(|allowed| name == *allowed) {
                return Err(invalid("rotated or additional keys unsupported in proof"));
            }
        }
        let mut files = Vec::new();
        for name in KEY_FILES {
            let path = directory.join(name);
            let metadata = std::fs::symlink_metadata(&path)?;
            if !metadata.is_file() || metadata.len() > 4096 {
                return Err(invalid("invalid proof key file"));
            }
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(4097)
                .read_to_end(&mut bytes)?;
            if bytes.is_empty() || bytes.len() > 4096 {
                return Err(invalid("invalid proof key file size"));
            }
            files.push(KeyFile {
                name: name.to_owned(),
                bytes: base64::engine::general_purpose::STANDARD.encode(bytes),
            });
        }
        let key_bundle =
            base64::engine::general_purpose::STANDARD.encode(encode(&KeyBundle { files })?);
        Ok(Self {
            schema,
            snapshot,
            host_id: base64::engine::general_purpose::STANDARD.encode(bootstrap.host_id),
            key_bundle,
        })
    }
    pub fn initialize(&self, controller: &CellEndpoint) -> Result<()> {
        let _: serde_json::Value = controller.call(
            "initialize",
            &json!({"snapshot":self.snapshot,"host_id":self.host_id,"key_bundle":self.key_bundle}),
        )?;
        Ok(())
    }
}

/// The controller capability is kept separate from the data adapter. Claim is
/// explicit; rejected stale adapters cannot automatically acquire a new epoch.
pub fn claim(controller: &CellEndpoint, expected_epoch: &str) -> Result<(String, String)> {
    use foks_server::Entropy;
    let mut bytes = [0; 32];
    foks_server::OsEntropy.fill(&mut bytes)?;
    let owner = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let request = json!({"expected_epoch":expected_epoch,"owner":owner});
    let response: serde_json::Value = controller
        .call("claim", &request)
        .or_else(|_| controller.call("claim", &request))?;
    let epoch = expected_epoch
        .parse::<i64>()?
        .checked_add(1)
        .ok_or_else(|| invalid("proof epoch exhausted"))?
        .to_string();
    if response.get("epoch").and_then(|v| v.as_str()) != Some(epoch.as_str())
        || response.get("owner").and_then(|v| v.as_str()) != Some(owner.as_str())
    {
        return Err(invalid("invalid proof claim receipt"));
    }
    Ok((epoch, owner))
}

pub struct CellServer {
    server: Option<foks_server::net::RunningServer>,
    pub bridge: Arc<NativeBridge>,
    directory: tempfile::TempDir,
    environment: crate::TestEnvironment,
}
impl CellServer {
    /// Restores every native database/key artifact from cell authority. Only the
    /// controller's operator root secret, public probe TLS fixture and addresses
    /// survive outside the cell; they are not native data recovery artifacts.
    pub fn start(
        environment: &crate::TestEnvironment,
        endpoint: CellEndpoint,
        schema: Schema,
        epoch: String,
        owner: String,
    ) -> Result<Self> {
        let mut running = environment
            .inner
            .running
            .lock()
            .map_err(|_| invalid("fixture lifecycle lock"))?;
        if *running {
            return Err(invalid("stop fixture server before starting cell adapter"));
        }
        let initial: CellSnapshot =
            endpoint.call("snapshot", &json!({"epoch":epoch,"owner":owner}))?;
        if initial.epoch != epoch {
            return Err(invalid("incorrect proof snapshot epoch"));
        }
        schema.validate_snapshot(&initial.snapshot)?;
        let bridge = Arc::new(NativeBridge::new(
            endpoint,
            schema.clone(),
            epoch,
            owner,
            &initial.key_bundle,
        )?);
        let directory = tempfile::tempdir()?;
        let database_path = directory.path().join("bootstrap.sqlite");
        schema.materialize(&initial.snapshot, &database_path)?;
        let reader =
            foks_server_db::ReadDatabase::open(&database_path, foks_server_db::Config::default())?;
        let bootstrap = reader
            .host_bootstrap()?
            .ok_or_else(|| invalid("proof cell has no host"))?;
        if initial.key_bundle.len() > (512usize * 1024).div_ceil(3) * 4 {
            return Err(invalid("proof key bundle byte limit"));
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(&initial.key_bundle)?;
        if bytes.len() > 512 * 1024
            || base64::engine::general_purpose::STANDARD.encode(&bytes) != initial.key_bundle
        {
            return Err(invalid("invalid proof key bundle"));
        }
        let bundle: KeyBundle = read_json(bytes.as_slice())?;
        if bundle.files.len() != KEY_FILES.len() {
            return Err(invalid("incomplete proof keys"));
        }
        let keys_path = directory.path().join("keys");
        std::fs::create_dir(&keys_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&keys_path, std::fs::Permissions::from_mode(0o700))?;
        }
        for (file, expected) in bundle.files.iter().zip(KEY_FILES) {
            if file.name != expected {
                return Err(invalid("invalid proof key filename"));
            }
            let bytes = base64::engine::general_purpose::STANDARD.decode(&file.bytes)?;
            if bytes.is_empty() || bytes.len() > 4096 {
                return Err(invalid("invalid proof key bytes"));
            }
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(keys_path.join(expected))?.write_all(&bytes)?;
        }
        let keys = Arc::new(DirectoryKeyProvider::open(
            &keys_path,
            environment.inner.root_key,
        )?);
        let manifest = KeyGenerationManifest::decode(&bootstrap.key_manifest)?;
        for purpose in MANIFEST_PURPOSES {
            if Some(keys.load_existing(purpose)?.generation()) != manifest.generation(purpose) {
                return Err(invalid("proof key manifest mismatch"));
            }
        }
        let tls = foks_server::pki::build_host_tls(keys.as_ref(), &bootstrap.canonical_name)?;
        let [probe_address, public_address, authenticated_address] =
            environment.configured_addresses();
        let server = foks_server::net::start_proof(
            foks_server::Config {
                chunk_store: None,
                vhost_management_host: String::new(),
                web_admin: None,
                sso: None,
                probe_address,
                public_address,
                authenticated_address,
                probe_tls: Arc::clone(&environment.inner.tls.public),
                public_tls: tls.public,
                authenticated_tls: tls.authenticated,
                probe_response: Arc::from(bootstrap.probe_response),
                read_database: Some(foks_server::ReadDatabaseConfig {
                    path: database_path,
                    database: foks_server_db::Config::default(),
                }),
                writer: None,
                clock: environment.inner.clock.clone(),
                entropy: Arc::new(foks_server::OsEntropy),
                key_provider: Some(keys),
                session_faults: Some(Arc::clone(&environment.inner.session_faults)),
                diagnostics: None,
                metrics: Arc::new(foks_server::ServerMetrics::default()),
                rate_limits: environment.inner.rate_limits,
                limits: environment.inner.session_limits,
            },
            bridge.clone(),
        )?;
        if Some(server.addresses()) != environment.addresses() {
            server.shutdown()?;
            return Err(invalid("proof fixture address changed"));
        }
        *running = true;
        drop(running);
        Ok(Self {
            server: Some(server),
            bridge,
            directory,
            environment: environment.clone(),
        })
    }
    pub fn addresses(&self) -> foks_server::ServerAddresses {
        self.server
            .as_ref()
            .expect("running proof server")
            .addresses()
    }
    pub fn projection_directory(&self) -> &Path {
        self.directory.path()
    }
    pub fn shutdown(mut self) -> Result<()> {
        self.stop()
    }
    fn stop(&mut self) -> Result<()> {
        if let Some(server) = self.server.take() {
            let result = server.shutdown();
            *self
                .environment
                .inner
                .running
                .lock()
                .map_err(|_| invalid("fixture lifecycle lock"))? = false;
            result?;
        }
        Ok(())
    }
}
impl Drop for CellServer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
