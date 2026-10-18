//! A serial native adapter for one cell. The local SQLite files are disposable.
use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use foks_server::{
    proof::{Projection, RpcProof},
    Entropy,
};
use serde_json::json;

#[derive(Clone)]
pub struct CellEndpoint {
    endpoint: url::Url,
    token: Arc<zeroize::Zeroizing<String>>,
    client: reqwest::blocking::Client,
}
impl CellEndpoint {
    /// This qualification helper deliberately refuses non-loopback HTTP endpoints.
    pub fn new(endpoint: &str, tenant: &str, token: String) -> Result<Self> {
        let mut endpoint = url::Url::parse(endpoint)?;
        let loopback = endpoint
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        if endpoint.scheme() != "http"
            || !loopback
            || endpoint.port().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.path() != "/"
            || token.len() < 32
            || token.len() > 1024
            || tenant.is_empty()
            || tenant.len() > 64
            || !tenant
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        {
            return Err(invalid("invalid local proof endpoint"));
        }
        endpoint.query_pairs_mut().append_pair("tenant", tenant);
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(4))
            .build()?;
        Ok(Self {
            endpoint,
            token: Arc::new(zeroize::Zeroizing::new(token)),
            client,
        })
    }
    pub fn call<T: serde::de::DeserializeOwned>(
        &self,
        operation: &str,
        input: &impl Serialize,
    ) -> Result<T> {
        if !matches!(
            operation,
            "initialize" | "claim" | "snapshot" | "apply" | "prove"
        ) {
            return Err(invalid("invalid proof operation"));
        }
        let mut url = self.endpoint.clone();
        url.set_path(&format!("/{operation}"));
        let response = self
            .client
            .post(url)
            .bearer_auth(self.token.as_str())
            .header("content-type", "application/json")
            .body(encode(input)?)
            .send()?;
        if !response.status().is_success() {
            return Err(invalid("proof authority rejected request"));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BYTES as u64)
        {
            return Err(invalid("proof response byte limit"));
        }
        read_json(response)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellSnapshot {
    pub epoch: String,
    pub revision: String,
    pub snapshot: Snapshot,
    pub key_bundle: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    epoch: String,
    owner: String,
    request_hash: String,
    revision: String,
    state_hash: String,
    output_hash: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    epoch: String,
    revision: String,
    state_hash: String,
}

#[derive(Default)]
struct Counters {
    requests: AtomicU64,
    snapshot_bytes: AtomicU64,
    delta_bytes: AtomicU64,
    microseconds: AtomicU64,
    rejected: AtomicU64,
}
#[derive(Debug, Serialize)]
pub struct BridgeMetrics {
    pub requests: u64,
    pub snapshot_bytes: u64,
    pub delta_bytes: u64,
    pub microseconds: u64,
    pub rejected: u64,
}
/// Scope: one serial adapter process per tenant, with fencing against a second
/// adapter. Neither local native admission nor its timeout grants cell ownership.
pub struct NativeBridge {
    endpoint: CellEndpoint,
    schema: Schema,
    epoch: String,
    owner: String,
    key_bundle_hash: String,
    serial: Mutex<()>,
    metrics: Counters,
    fail_after_commit: AtomicBool,
    before_publish: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl NativeBridge {
    pub fn new(
        endpoint: CellEndpoint,
        schema: Schema,
        epoch: String,
        owner: String,
        key_bundle: &str,
    ) -> Result<Self> {
        if epoch
            .parse::<i64>()
            .ok()
            .filter(|v| *v > 0)
            .map(|v| v.to_string())
            .as_ref()
            != Some(&epoch)
            || owner.len() < 32
            || owner.len() > 128
            || !owner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(invalid("invalid proof owner"));
        }
        // Validate exact native schema even before the first network call.
        schema.require_native_schema()?;
        Ok(Self {
            endpoint,
            schema,
            epoch,
            owner,
            key_bundle_hash: digest(&key_bundle)?,
            serial: Mutex::new(()),
            metrics: Counters::default(),
            fail_after_commit: AtomicBool::new(false),
            before_publish: Mutex::new(None),
        })
    }
    pub fn snapshot(&self) -> Result<CellSnapshot> {
        let snapshot: CellSnapshot = self
            .endpoint
            .call("snapshot", &json!({"epoch":self.epoch,"owner":self.owner}))?;
        if snapshot.epoch != self.epoch
            || digest(&snapshot.key_bundle)? != self.key_bundle_hash
            || snapshot
                .revision
                .parse::<i64>()
                .ok()
                .filter(|v| *v >= 0)
                .map(|v| v.to_string())
                .as_ref()
                != Some(&snapshot.revision)
        {
            return Err(invalid("proof identity or revision changed"));
        }
        self.schema.validate_snapshot(&snapshot.snapshot)?;
        Ok(snapshot)
    }
    /// Simulate an ambiguous response after authoritative publication. It never
    /// changes or bypasses authority acceptance and consumes exactly one write.
    pub fn lose_next_committed_response(&self) {
        self.fail_after_commit.store(true, Ordering::SeqCst);
    }
    /// Controller-only fault hook, invoked once after a real native mutation
    /// finishes but before its first authoritative apply. Useful for promoting
    /// another owner while a former owner's candidate remains outstanding.
    pub fn before_next_publish(&self, hook: Box<dyn FnOnce() + Send>) -> Result<()> {
        let mut pending = self
            .before_publish
            .lock()
            .map_err(|_| invalid("proof hook lock"))?;
        if pending.is_some() {
            return Err(invalid("proof hook already pending"));
        }
        *pending = Some(hook);
        Ok(())
    }
    pub fn metrics(&self) -> BridgeMetrics {
        let read = |value: &AtomicU64| value.load(Ordering::Relaxed);
        BridgeMetrics {
            requests: read(&self.metrics.requests),
            snapshot_bytes: read(&self.metrics.snapshot_bytes),
            delta_bytes: read(&self.metrics.delta_bytes),
            microseconds: read(&self.metrics.microseconds),
            rejected: read(&self.metrics.rejected),
        }
    }
    fn run(
        &self,
        operation: Box<dyn FnOnce(Projection) -> foks_server::Result<Vec<u8>> + '_>,
    ) -> Result<Vec<u8>> {
        // Refuse queued calls: a timed-out operation may retain this guard while
        // its writer drains. No replacement operation reads that projection.
        let _serial = self
            .serial
            .try_lock()
            .map_err(|_| invalid("proof adapter busy"))?;
        let before = self.snapshot()?;
        self.metrics
            .snapshot_bytes
            .fetch_add(encode(&before.snapshot)?.len() as u64, Ordering::Relaxed);
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("projection.sqlite");
        self.schema.materialize(&before.snapshot, &path)?;
        let database = foks_server_db::Config::default();
        let writer = foks_server::Writer::start(path.clone(), database.clone(), 8)?;
        let candidate = operation(Projection {
            read_database: foks_server::ReadDatabaseConfig {
                path: path.clone(),
                database,
            },
            writer: writer.handle(),
        });
        // Even failure or a caller timeout must wait for accepted native work.
        writer.shutdown()?;
        let candidate = candidate?;
        let connection = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let after = self.schema.snapshot(&connection)?;
        let delta = self.schema.delta(&before.snapshot, &after)?;
        let after_hash = delta.after_hash.clone();
        let revision = if delta.before_hash != delta.after_hash {
            let hook = self
                .before_publish
                .lock()
                .map_err(|_| invalid("proof hook lock"))?
                .take();
            if let Some(hook) = hook {
                hook();
            }
            let mut nonce = [0; 32];
            foks_server::OsEntropy.fill(&mut nonce)?;
            let attempt = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(nonce);
            let output_hash = format!("{:x}", Sha256::digest(&candidate));
            let request = json!({"epoch":self.epoch,"owner":self.owner,"revision":before.revision,"attempt":attempt,"output_hash":output_hash,"delta":delta});
            self.metrics
                .delta_bytes
                .fetch_add(encode(&request)?.len() as u64, Ordering::Relaxed);
            // If the transport outcome is ambiguous, retry the identical receipt
            // request once. Never rerun the native handler or regenerate its IDs.
            let receipt: Receipt = self
                .endpoint
                .call("apply", &request)
                .or_else(|_| self.endpoint.call("apply", &request))?;
            let expected_revision = before
                .revision
                .parse::<i64>()?
                .checked_add(1)
                .ok_or_else(|| invalid("proof revision exhausted"))?
                .to_string();
            if receipt.epoch != self.epoch
                || receipt.owner != self.owner
                || receipt.revision != expected_revision
                || receipt.output_hash != output_hash
                || receipt.state_hash != after_hash
                || receipt.request_hash != digest(&request)?
            {
                return Err(invalid("invalid authoritative receipt"));
            }
            if self.fail_after_commit.swap(false, Ordering::SeqCst) {
                return Err(invalid("injected lost committed response"));
            }
            receipt.revision
        } else {
            before.revision
        };
        // A receipt for an earlier operation is insufficient after ownership
        // changes. Recheck both epoch and current revision immediately before
        // releasing every data/error reply, including immutable identity reads.
        // This is the observation's linearization point, not an atomic fence
        // around subsequent TCP output. A later promotion may overlap transport
        // of this already proved response; its writes remain authoritative.
        let proof:Proof=self.endpoint.call("prove",&json!({"epoch":self.epoch,"owner":self.owner,"revision":revision,"state_hash":after_hash}))?;
        if proof.epoch != self.epoch || proof.revision != revision || proof.state_hash != after_hash
        {
            return Err(invalid("invalid output proof"));
        }
        Ok(candidate)
    }
}
impl RpcProof for NativeBridge {
    fn execute(
        &self,
        _route: Option<foks_server::rpc::RouteId>,
        operation: Box<dyn FnOnce(Projection) -> foks_server::Result<Vec<u8>> + '_>,
    ) -> foks_server::Result<Vec<u8>> {
        let started = Instant::now();
        let result = self.run(operation);
        self.metrics.requests.fetch_add(1, Ordering::Relaxed);
        self.metrics.microseconds.fetch_add(
            started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
        if result.is_err() {
            self.metrics.rejected.fetch_add(1, Ordering::Relaxed);
        }
        // No details or SQL values leak through an ungated protocol error.
        result.map_err(|_| {
            foks_server::Error::Config("celld proof authority unavailable or rejected operation")
        })
    }
}
