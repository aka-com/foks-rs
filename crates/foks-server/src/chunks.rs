//! Immutable, content-addressed storage for canonical encrypted upload chunks.
//!
//! Call this synchronous interface from blocking handlers, never while holding
//! the SQLite writer or a read snapshot. Remote calls have one overall deadline.
//! A hung native filesystem can still block a filesystem call (including fsync);
//! the filesystem backend is not a guarantee of bounded failover latency.
//! Objects are never deleted here: retries and backup hydration may still need
//! them after their current database references disappear.

use std::future::Future;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures_util::StreamExt;
use object_store::aws::{AmazonS3Builder, AmazonS3ConfigKey, S3ConditionalPut};
use object_store::client::{ClientConfigKey, ClientOptions, HttpClient, HttpConnector};
use object_store::{ObjectStore, ObjectStoreExt, PutMode, PutOptions, RetryConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

pub const MAXIMUM_ENCODED_CHUNK_BYTES: usize = 9 * 1024 * 1024 + 1024;

/// Configuration contains no credentials. AWS access keys/session tokens and
/// workload identity settings come from their standard environment variables.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChunkStoreConfig {
    Filesystem {
        directory: PathBuf,
    },
    S3 {
        bucket: String,
        region: String,
        #[serde(default)]
        endpoint: Option<String>,
        #[serde(default)]
        prefix: String,
        #[serde(default = "default_timeout_seconds")]
        timeout_seconds: u64,
        /// Development/test endpoints must use a literal loopback IP address.
        #[serde(default)]
        allow_http_loopback: bool,
    },
}

fn default_timeout_seconds() -> u64 {
    10
}

impl ChunkStoreConfig {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Filesystem { directory } => {
                if !directory.is_absolute() {
                    return Err(Error::Config("chunk directory must be absolute"));
                }
            }
            Self::S3 {
                bucket,
                region,
                endpoint,
                prefix,
                timeout_seconds,
                allow_http_loopback,
            } => {
                if bucket.is_empty()
                    || bucket.len() > 63
                    || !bucket
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c))
                    || region.is_empty()
                    || region.len() > 128
                    || !region
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                    || !(1..=60).contains(timeout_seconds)
                    || prefix.len() > 768
                {
                    return Err(Error::Config("invalid S3 chunk store configuration"));
                }
                object_store::path::Path::parse(prefix)
                    .map_err(|_| Error::Config("invalid chunk object prefix"))?;
                if let Some(endpoint) = endpoint {
                    let url = url::Url::parse(endpoint)
                        .map_err(|_| Error::Config("invalid chunk endpoint"))?;
                    let loopback = match url.host() {
                        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
                        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
                        _ => false,
                    };
                    if url.host().is_none()
                        || !url.username().is_empty()
                        || url.password().is_some()
                        || url.query().is_some()
                        || url.fragment().is_some()
                        || url.path() != "/"
                        || !(url.scheme() == "https"
                            || (url.scheme() == "http" && *allow_http_loopback && loopback))
                    {
                        return Err(Error::Config(
                            "chunk endpoint must use HTTPS or explicit loopback HTTP",
                        ));
                    }
                } else if *allow_http_loopback {
                    return Err(Error::Config(
                        "loopback HTTP requires an explicit chunk endpoint",
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn build(&self) -> Result<Arc<dyn ChunkStore>> {
        self.validate()?;
        match self {
            Self::Filesystem { directory } => Ok(Arc::new(FilesystemChunks::open(directory)?)),
            Self::S3 { .. } => Ok(Arc::new(S3Chunks::build(self, credential_builder()?)?)),
        }
    }
}

/// Exact immutable object publication and bounded, integrity-checked retrieval.
pub trait ChunkStore: Send + Sync {
    /// Canonical non-secret placement descriptor; credentials/timeouts are excluded.
    fn locator(&self) -> Result<String>;
    /// Missing is distinct from an unavailable, malformed or oversized marker.
    fn read_namespace_marker(&self) -> Result<Option<Vec<u8>>>;
    fn create_namespace_marker_if_absent(&self, marker: &[u8]) -> Result<()>;
    fn put(&self, digest: &[u8; 32], bytes: &[u8]) -> Result<()>;
    fn get(&self, digest: &[u8; 32], encoded_length: u64) -> Result<Vec<u8>>;
}

pub fn chunk_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn checked_length(length: u64) -> Result<usize> {
    let length = usize::try_from(length).map_err(|_| Error::Chunk("invalid object length"))?;
    if length == 0 || length > MAXIMUM_ENCODED_CHUNK_BYTES {
        return Err(Error::Chunk("invalid object length"));
    }
    Ok(length)
}

fn verify(digest: &[u8; 32], bytes: &[u8], length: usize) -> Result<()> {
    if bytes.len() != length || chunk_digest(bytes) != *digest {
        return Err(Error::Chunk("object length or digest mismatch"));
    }
    Ok(())
}

fn object_name(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut name = String::with_capacity(64);
    for byte in digest {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 15)]));
    }
    name
}

struct FilesystemChunks {
    directory: PathBuf,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl FilesystemChunks {
    fn open(directory: &Path) -> Result<Self> {
        create_directory_durable(directory)?;
        let metadata = std::fs::symlink_metadata(directory)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(Error::Chunk("chunk root is not a regular directory"));
        }
        let directory = directory.canonicalize()?;
        // Sync the root and its parent so a newly created root survives a crash.
        std::fs::File::open(&directory)?.sync_all()?;
        if let Some(parent) = directory.parent() {
            std::fs::File::open(parent)?.sync_all()?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self {
                directory,
                identity: (metadata.dev(), metadata.ino()),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = directory;
            Err(Error::Chunk(
                "filesystem chunk identity checks require Unix",
            ))
        }
    }

    fn recheck(&self) -> Result<()> {
        let metadata = std::fs::symlink_metadata(&self.directory)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(Error::Chunk("chunk root changed"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if (metadata.dev(), metadata.ino()) != self.identity {
                return Err(Error::Chunk("chunk root identity changed"));
            }
        }
        Ok(())
    }
}

fn create_directory_durable(directory: &Path) -> Result<()> {
    match std::fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(Error::Chunk("chunk directory is not a regular directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = directory
                .parent()
                .ok_or(Error::Chunk("invalid chunk directory"))?;
            create_directory_durable(parent)?;
            match std::fs::create_dir(directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let metadata = std::fs::symlink_metadata(directory)?;
                    if !metadata.is_dir() || metadata.file_type().is_symlink() {
                        return Err(Error::Chunk("chunk directory changed during creation"));
                    }
                }
                Err(error) => return Err(error.into()),
            }
            std::fs::File::open(directory)?.sync_all()?;
            std::fs::File::open(parent)?.sync_all()?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

const NAMESPACE_MARKER: &str = "_foks_namespace_v1";
const MAXIMUM_MARKER_BYTES: usize = 1024;

fn check_marker_size(bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() || bytes.len() > MAXIMUM_MARKER_BYTES {
        return Err(Error::Chunk("invalid storage namespace marker size"));
    }
    Ok(())
}

impl ChunkStore for FilesystemChunks {
    fn locator(&self) -> Result<String> {
        self.recheck()?;
        let directory = self
            .directory
            .to_str()
            .ok_or(Error::Chunk("chunk directory must be UTF-8"))?;
        serde_json::to_string(&("filesystem", directory))
            .map_err(|_| Error::Chunk("invalid chunk locator"))
    }

    fn read_namespace_marker(&self) -> Result<Option<Vec<u8>>> {
        self.recheck()?;
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = match options.open(self.directory.join(NAMESPACE_MARKER)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > MAXIMUM_MARKER_BYTES as u64
        {
            return Err(Error::Chunk("invalid storage namespace marker"));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAXIMUM_MARKER_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        check_marker_size(&bytes)?;
        self.recheck()?;
        Ok(Some(bytes))
    }

    fn create_namespace_marker_if_absent(&self, marker: &[u8]) -> Result<()> {
        check_marker_size(marker)?;
        self.recheck()?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.directory)?;
        temporary.write_all(marker)?;
        temporary.as_file().sync_all()?;
        self.recheck()?;
        match temporary.persist_noclobber(self.directory.join(NAMESPACE_MARKER)) {
            Ok(file) => file.sync_all()?,
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                if self.read_namespace_marker()?.as_deref() != Some(marker) {
                    return Err(Error::Chunk("storage namespace marker conflict"));
                }
            }
            Err(error) => return Err(error.error.into()),
        }
        std::fs::File::open(&self.directory)?.sync_all()?;
        self.recheck()
    }

    fn put(&self, digest: &[u8; 32], bytes: &[u8]) -> Result<()> {
        let length = checked_length(bytes.len() as u64)?;
        verify(digest, bytes, length)?;
        self.recheck()?;
        let path = self.directory.join(object_name(digest));
        let mut temporary = tempfile::NamedTempFile::new_in(&self.directory)?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        self.recheck()?;
        match temporary.persist_noclobber(&path) {
            Ok(file) => {
                file.sync_all()?;
                std::fs::File::open(&self.directory)?.sync_all()?;
                self.recheck()?;
                Ok(())
            }
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                // Existing bytes may be from an interrupted earlier attempt.
                self.get(digest, bytes.len() as u64)?;
                std::fs::File::open(&self.directory)?.sync_all()?;
                Ok(())
            }
            Err(error) => Err(error.error.into()),
        }
    }

    fn get(&self, digest: &[u8; 32], encoded_length: u64) -> Result<Vec<u8>> {
        let length = checked_length(encoded_length)?;
        self.recheck()?;
        let path = self.directory.join(object_name(digest));
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // O_NONBLOCK prevents a malicious FIFO leaf from hanging open.
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() != encoded_length {
            return Err(Error::Chunk("invalid stored object type or length"));
        }
        self.recheck()?;
        let mut bytes = Vec::with_capacity(length);
        file.take(encoded_length + 1).read_to_end(&mut bytes)?;
        verify(digest, &bytes, length)?;
        Ok(bytes)
    }
}

/// Read credential settings only: generic AWS_* endpoint/TLS/signature options
/// must not silently override the explicit operator storage configuration.
fn credential_builder() -> Result<AmazonS3Builder> {
    let mut builder = AmazonS3Builder::new();
    for (name, key) in [
        ("AWS_ACCESS_KEY_ID", AmazonS3ConfigKey::AccessKeyId),
        ("AWS_SECRET_ACCESS_KEY", AmazonS3ConfigKey::SecretAccessKey),
        ("AWS_SESSION_TOKEN", AmazonS3ConfigKey::Token),
        (
            "AWS_WEB_IDENTITY_TOKEN_FILE",
            AmazonS3ConfigKey::WebIdentityTokenFile,
        ),
        ("AWS_ROLE_ARN", AmazonS3ConfigKey::RoleArn),
        ("AWS_ROLE_SESSION_NAME", AmazonS3ConfigKey::RoleSessionName),
        (
            "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
            AmazonS3ConfigKey::ContainerCredentialsRelativeUri,
        ),
        (
            "AWS_CONTAINER_CREDENTIALS_FULL_URI",
            AmazonS3ConfigKey::ContainerCredentialsFullUri,
        ),
        (
            "AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE",
            AmazonS3ConfigKey::ContainerAuthorizationTokenFile,
        ),
    ] {
        match std::env::var(name) {
            Ok(value) => builder = builder.with_config(key, value),
            Err(std::env::VarError::NotPresent) => {}
            Err(_) => return Err(Error::Config("invalid chunk credential environment")),
        }
    }
    Ok(builder)
}

#[derive(Debug)]
struct NoRedirectConnector {
    timeout: Duration,
}

impl HttpConnector for NoRedirectConnector {
    fn connect(&self, options: &ClientOptions) -> object_store::Result<HttpClient> {
        // Credential providers require their own HTTP metadata endpoint policy;
        // object requests get the stricter endpoint policy validated above.
        let allow_http = options
            .get_config_value(&ClientConfigKey::AllowHttp)
            .is_some_and(|value| value == "true");
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .https_only(!allow_http)
            .timeout(self.timeout)
            .connect_timeout(self.timeout.min(Duration::from_secs(3)))
            .build()
            .map_err(|error| object_store::Error::Generic {
                store: "FOKS chunk HTTP client",
                source: Box::new(error),
            })?;
        Ok(HttpClient::new(client))
    }
}

struct S3Chunks {
    locator: String,
    store: Arc<dyn ObjectStore>,
    prefix: object_store::path::Path,
    timeout: Duration,
}

impl S3Chunks {
    fn build(config: &ChunkStoreConfig, mut builder: AmazonS3Builder) -> Result<Self> {
        config.validate()?;
        let ChunkStoreConfig::S3 {
            bucket,
            region,
            endpoint,
            prefix,
            timeout_seconds,
            allow_http_loopback,
        } = config
        else {
            return Err(Error::Config("expected S3 chunk configuration"));
        };
        let timeout = Duration::from_secs(*timeout_seconds);
        builder = builder
            .with_bucket_name(bucket)
            .with_region(region)
            .with_client_options(ClientOptions::new().with_allow_http(*allow_http_loopback))
            .with_http_connector(NoRedirectConnector { timeout })
            .with_conditional_put(S3ConditionalPut::ETagMatch)
            .with_retry(RetryConfig {
                max_retries: 2,
                retry_timeout: timeout,
                ..RetryConfig::default()
            });
        if let Some(endpoint) = endpoint {
            builder = builder.with_endpoint(endpoint);
        }
        let store = builder
            .build()
            .map_err(|_| Error::Chunk("cannot initialize S3 chunk backend"))?;
        let canonical_endpoint = endpoint
            .as_deref()
            .map(url::Url::parse)
            .transpose()
            .map_err(|_| Error::Chunk("invalid chunk endpoint"))?
            .map(|url| url.to_string());
        let canonical_prefix = object_store::path::Path::parse(prefix)
            .map_err(|_| Error::Chunk("invalid chunk prefix"))?
            .to_string();
        let locator =
            serde_json::to_string(&("s3", bucket, region, canonical_endpoint, canonical_prefix))
                .map_err(|_| Error::Chunk("invalid chunk locator"))?;
        Ok(Self {
            locator,
            store: Arc::new(store),
            prefix: object_store::path::Path::parse(prefix)
                .map_err(|_| Error::Config("invalid object prefix"))?,
            timeout,
        })
    }

    fn path(&self, digest: &[u8; 32]) -> object_store::path::Path {
        self.prefix.clone().join(object_name(digest))
    }

    fn run<T: Send + 'static>(
        &self,
        future: impl Future<Output = Result<T>> + Send + 'static,
    ) -> Result<T> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let timeout = self.timeout;
        // Keep payload and result memory charged even when the waiter cancels.
        let task = remote_runtime()?.spawn(crate::net::memory::retain_future(async move {
            let result = tokio::time::timeout(timeout, future)
                .await
                .unwrap_or(Err(Error::Chunk("object operation timed out")));
            let _ = sender.send(result);
        }));
        // This also bounds waiting if runtime workers are overloaded. Abort is
        // cancellation, not a claim that an in-flight PUT could not complete.
        let result = receiver
            .recv_timeout(timeout)
            .unwrap_or(Err(Error::Chunk("object operation timed out")));
        task.abort();
        result
    }
}

fn remote_runtime() -> Result<&'static tokio::runtime::Runtime> {
    static RUNTIME: OnceLock<std::result::Result<tokio::runtime::Runtime, String>> =
        OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("foks-chunk-io")
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|_| Error::Chunk("cannot initialize chunk runtime"))
}

async fn read_remote(
    store: &dyn ObjectStore,
    path: &object_store::path::Path,
    digest: &[u8; 32],
    length: usize,
) -> Result<Vec<u8>> {
    let result = store
        .get(path)
        .await
        .map_err(|_| Error::Chunk("object read failed"))?;
    if result.meta.size != length as u64 || result.range != (0..length as u64) {
        return Err(Error::Chunk("stored object length mismatch"));
    }
    let mut stream = result.into_stream();
    let mut bytes = Vec::with_capacity(length);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::Chunk("object body read failed"))?;
        if chunk.len() > length - bytes.len() {
            return Err(Error::Chunk("object body exceeds expected length"));
        }
        bytes.extend_from_slice(&chunk);
    }
    verify(digest, &bytes, length)?;
    Ok(bytes)
}

async fn read_remote_marker(
    store: &dyn ObjectStore,
    path: &object_store::path::Path,
) -> Result<Option<Vec<u8>>> {
    let result = match store.get(path).await {
        Ok(result) => result,
        Err(object_store::Error::NotFound { .. }) => return Ok(None),
        Err(_) => return Err(Error::Chunk("storage namespace marker read failed")),
    };
    let length = result.meta.size;
    if length == 0 || length > MAXIMUM_MARKER_BYTES as u64 || result.range != (0..length) {
        return Err(Error::Chunk("invalid storage namespace marker size"));
    }
    let mut stream = result.into_stream();
    let mut bytes = Vec::with_capacity(length as usize);
    while let Some(part) = stream.next().await {
        let part = part.map_err(|_| Error::Chunk("storage namespace marker read failed"))?;
        if part.len() > length as usize - bytes.len() {
            return Err(Error::Chunk("oversized storage namespace marker"));
        }
        bytes.extend_from_slice(&part);
    }
    if bytes.len() != length as usize {
        return Err(Error::Chunk("truncated storage namespace marker"));
    }
    Ok(Some(bytes))
}

impl ChunkStore for S3Chunks {
    fn locator(&self) -> Result<String> {
        Ok(self.locator.clone())
    }

    fn read_namespace_marker(&self) -> Result<Option<Vec<u8>>> {
        let store = Arc::clone(&self.store);
        let path = self.prefix.clone().join(NAMESPACE_MARKER);
        self.run(async move { read_remote_marker(store.as_ref(), &path).await })
    }

    fn create_namespace_marker_if_absent(&self, marker: &[u8]) -> Result<()> {
        check_marker_size(marker)?;
        let marker = marker.to_vec();
        let store = Arc::clone(&self.store);
        let path = self.prefix.clone().join(NAMESPACE_MARKER);
        self.run(async move {
            match store
                .put_opts(
                    &path,
                    marker.clone().into(),
                    PutOptions {
                        mode: PutMode::Create,
                        ..PutOptions::default()
                    },
                )
                .await
            {
                Ok(_) => Ok(()),
                Err(
                    object_store::Error::AlreadyExists { .. }
                    | object_store::Error::Precondition { .. },
                ) => {
                    if read_remote_marker(store.as_ref(), &path).await?.as_deref()
                        == Some(marker.as_slice())
                    {
                        Ok(())
                    } else {
                        Err(Error::Chunk("storage namespace marker conflict"))
                    }
                }
                Err(_) => Err(Error::Chunk("storage namespace marker creation failed")),
            }
        })
    }

    fn put(&self, digest: &[u8; 32], bytes: &[u8]) -> Result<()> {
        let length = checked_length(bytes.len() as u64)?;
        verify(digest, bytes, length)?;
        let digest = *digest;
        let store = Arc::clone(&self.store);
        let path = self.path(&digest);
        let payload = bytes.to_vec();
        self.run(async move {
            let options = PutOptions {
                mode: PutMode::Create,
                ..PutOptions::default()
            };
            match store.put_opts(&path, payload.into(), options).await {
                Ok(_) => Ok(()),
                Err(
                    object_store::Error::AlreadyExists { .. }
                    | object_store::Error::Precondition { .. },
                ) => {
                    read_remote(store.as_ref(), &path, &digest, length).await?;
                    Ok(())
                }
                // An ambiguous transport result may have stored an orphan;
                // return retryable failure and never replace or delete it.
                Err(_) => Err(Error::Chunk("conditional object publication failed")),
            }
        })
    }

    fn get(&self, digest: &[u8; 32], encoded_length: u64) -> Result<Vec<u8>> {
        let length = checked_length(encoded_length)?;
        let digest = *digest;
        let store = Arc::clone(&self.store);
        let path = self.path(&digest);
        self.run(async move { read_remote(store.as_ref(), &path, &digest, length).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Instant;

    #[test]
    fn filesystem_namespace_marker_is_bounded_immutable_and_not_a_chunk() {
        let temporary = tempfile::tempdir().unwrap();
        let store = FilesystemChunks::open(temporary.path()).unwrap();
        assert!(store.read_namespace_marker().unwrap().is_none());
        store
            .create_namespace_marker_if_absent(b"marker-a")
            .unwrap();
        store
            .create_namespace_marker_if_absent(b"marker-a")
            .unwrap();
        assert!(store
            .create_namespace_marker_if_absent(b"marker-b")
            .is_err());
        assert_eq!(store.read_namespace_marker().unwrap().unwrap(), b"marker-a");
        assert!(store.get(&chunk_digest(b"marker-a"), 8).is_err());
        assert!(store
            .create_namespace_marker_if_absent(&vec![0; MAXIMUM_MARKER_BYTES + 1])
            .is_err());
        std::fs::write(
            temporary.path().join(NAMESPACE_MARKER),
            vec![0; MAXIMUM_MARKER_BYTES + 1],
        )
        .unwrap();
        assert!(store.read_namespace_marker().is_err());
        #[cfg(unix)]
        {
            std::fs::remove_file(temporary.path().join(NAMESPACE_MARKER)).unwrap();
            std::os::unix::fs::symlink("missing", temporary.path().join(NAMESPACE_MARKER)).unwrap();
            assert!(store.read_namespace_marker().is_err());
            assert!(store
                .create_namespace_marker_if_absent(b"marker-a")
                .is_err());
        }
    }

    #[test]
    fn s3_namespace_marker_missing_replay_conflict_and_unavailable() {
        let mock = MockS3::start();
        let store = mock.store();
        assert!(store.read_namespace_marker().unwrap().is_none());
        store
            .create_namespace_marker_if_absent(b"marker-a")
            .unwrap();
        store
            .create_namespace_marker_if_absent(b"marker-a")
            .unwrap();
        assert!(store
            .create_namespace_marker_if_absent(b"marker-b")
            .is_err());
        assert_eq!(store.read_namespace_marker().unwrap().unwrap(), b"marker-a");
        assert_eq!(
            mock.state.puts.load(Ordering::Acquire),
            mock.state.conditional.load(Ordering::Acquire)
        );
        mock.state.oversized.store(true, Ordering::Release);
        assert!(store.read_namespace_marker().is_err());
        mock.state.oversized.store(false, Ordering::Release);
        mock.state.reject_put.store(true, Ordering::Release);
        assert!(store
            .create_namespace_marker_if_absent(b"marker-a")
            .is_err());
    }

    #[test]
    fn filesystem_immutable_replay_bounds_and_integrity() {
        let temporary = tempfile::tempdir().unwrap();
        let config = ChunkStoreConfig::Filesystem {
            directory: temporary.path().join("one/two/chunks"),
        };
        let store = config.build().unwrap();
        let payload = b"canonical encrypted upload";
        let digest = chunk_digest(payload);
        store.put(&digest, payload).unwrap();
        store.put(&digest, payload).unwrap();
        assert_eq!(store.get(&digest, payload.len() as u64).unwrap(), payload);
        assert!(store.put(&digest, b"wrong").is_err());
        assert!(store.get(&digest, 0).is_err());
        assert!(store
            .get(&digest, MAXIMUM_ENCODED_CHUNK_BYTES as u64 + 1)
            .is_err());
        assert!(store.get(&[0; 32], payload.len() as u64).is_err());
        let ChunkStoreConfig::Filesystem { directory } = config else {
            unreachable!()
        };
        std::fs::write(directory.join(object_name(&digest)), vec![0; payload.len()]).unwrap();
        assert!(store.get(&digest, payload.len() as u64).is_err());
        assert!(store.put(&digest, payload).is_err());
        assert_eq!(
            std::fs::read(directory.join(object_name(&digest))).unwrap(),
            vec![0; payload.len()]
        );
    }

    #[cfg(unix)]
    #[test]
    fn filesystem_rejects_symlink_leaves_and_root_replacement() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("objects");
        let store = FilesystemChunks::open(&directory).unwrap();
        let payload = b"secret ciphertext";
        let digest = chunk_digest(payload);
        let source = temporary.path().join("source");
        std::fs::write(&source, payload).unwrap();
        std::os::unix::fs::symlink(&source, directory.join(object_name(&digest))).unwrap();
        assert!(store.get(&digest, payload.len() as u64).is_err());
        assert!(store.put(&digest, payload).is_err());
        std::fs::rename(&directory, temporary.path().join("old")).unwrap();
        std::fs::create_dir(&directory).unwrap();
        assert!(store.put(&digest, payload).is_err());
        assert!(store.get(&digest, payload.len() as u64).is_err());
    }

    #[test]
    fn filesystem_concurrent_publication_converges() {
        let temporary = tempfile::tempdir().unwrap();
        let store = Arc::new(FilesystemChunks::open(temporary.path()).unwrap());
        let barrier = Arc::new(std::sync::Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    store
                        .put(&chunk_digest(b"same bytes"), b"same bytes")
                        .unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(
            store.get(&chunk_digest(b"same bytes"), 10).unwrap(),
            b"same bytes"
        );
    }

    fn s3_config(endpoint: String) -> ChunkStoreConfig {
        ChunkStoreConfig::S3 {
            bucket: "test-bucket".into(),
            region: "auto".into(),
            endpoint: Some(endpoint),
            prefix: "tenant/chunks".into(),
            timeout_seconds: 1,
            allow_http_loopback: true,
        }
    }

    #[test]
    fn explicit_endpoint_policy_and_secret_free_configuration() {
        for endpoint in [
            "http://example.com",
            "http://localhost",
            "https://user:secret@example.com",
            "https://example.com/?token=secret",
            "https://example.com/bucket",
        ] {
            assert!(s3_config(endpoint.into()).validate().is_err(), "{endpoint}");
        }
        assert!(s3_config("http://127.0.0.1:9000".into()).validate().is_ok());
        assert!(s3_config("https://example.com".into()).validate().is_ok());
        let input = r#"kind = "s3"
bucket = "test"
region = "auto"
secret_access_key = "not-accepted"
"#;
        assert!(toml::from_str::<ChunkStoreConfig>(input).is_err());
    }

    #[derive(Default)]
    struct MockState {
        objects: Mutex<BTreeMap<String, Vec<u8>>>,
        puts: AtomicUsize,
        gets: AtomicUsize,
        conditional: AtomicUsize,
        reject_put: AtomicBool,
        redirect: AtomicBool,
        redirect_port: AtomicUsize,
        slow: AtomicBool,
        delay_millis: AtomicU64,
        oversized: AtomicBool,
    }

    struct MockS3 {
        address: SocketAddr,
        state: Arc<MockState>,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl MockS3 {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let state = Arc::new(MockState::default());
            let stop = Arc::new(AtomicBool::new(false));
            let worker_state = Arc::clone(&state);
            let worker_stop = Arc::clone(&stop);
            let thread = std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if worker_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(stream) = stream else {
                        break;
                    };
                    serve_mock(stream, &worker_state);
                }
            });
            Self {
                address,
                state,
                stop,
                thread: Some(thread),
            }
        }

        fn store(&self) -> S3Chunks {
            S3Chunks::build(
                &s3_config(format!("http://{}", self.address)),
                AmazonS3Builder::new()
                    .with_access_key_id("test-key")
                    .with_secret_access_key("test-secret"),
            )
            .unwrap()
        }
    }

    impl Drop for MockS3 {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            let _ = TcpStream::connect(self.address);
            if let Some(thread) = self.thread.take() {
                thread.join().unwrap();
            }
        }
    }

    fn serve_mock(mut stream: TcpStream, state: &MockState) {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        if reader.read_line(&mut request).is_err() {
            return;
        }
        let mut parts = request.split_whitespace();
        let method = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("").to_owned();
        let mut length = 0usize;
        let mut conditional = false;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() || line.len() > 8192 {
                return;
            }
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                if key.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap();
                }
                if key.eq_ignore_ascii_case("if-none-match") && value.trim() == "*" {
                    conditional = true;
                }
            }
        }
        assert!(length <= MAXIMUM_ENCODED_CHUNK_BYTES);
        let mut body = vec![0; length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        if state.slow.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(2500));
        }
        std::thread::sleep(Duration::from_millis(
            state.delay_millis.load(Ordering::Acquire),
        ));
        if state.redirect.load(Ordering::Acquire) {
            let port = state.redirect_port.load(Ordering::Acquire);
            let response = format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:{port}/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(response.as_bytes());
            return;
        }
        let (status, response) = match method {
            "PUT" => {
                state.puts.fetch_add(1, Ordering::AcqRel);
                if conditional {
                    state.conditional.fetch_add(1, Ordering::AcqRel);
                }
                if state.reject_put.load(Ordering::Acquire) {
                    ("403 Forbidden", Vec::new())
                } else if !conditional {
                    ("400 Bad Request", Vec::new())
                } else {
                    let mut objects = state.objects.lock().unwrap();
                    if let std::collections::btree_map::Entry::Vacant(entry) = objects.entry(path) {
                        entry.insert(body);
                        ("200 OK", Vec::new())
                    } else {
                        ("412 Precondition Failed", Vec::new())
                    }
                }
            }
            "GET" => {
                state.gets.fetch_add(1, Ordering::AcqRel);
                match state.objects.lock().unwrap().get(&path) {
                    Some(bytes) => ("200 OK", bytes.clone()),
                    None => ("404 Not Found", Vec::new()),
                }
            }
            _ => ("400 Bad Request", Vec::new()),
        };
        let response_length = if state.oversized.load(Ordering::Acquire) && method == "GET" {
            MAXIMUM_ENCODED_CHUNK_BYTES + 1
        } else {
            response.len()
        };
        let header = format!("HTTP/1.1 {status}\r\nContent-Length: {response_length}\r\nETag: \"test-etag\"\r\nLast-Modified: Sun, 06 Nov 1994 08:49:37 GMT\r\nConnection: close\r\n\r\n");
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&response);
    }

    #[test]
    fn s3_conditional_publication_replay_corruption_and_missing_objects() {
        let mock = MockS3::start();
        let store = mock.store();
        let payload = b"canonical encrypted upload";
        let digest = chunk_digest(payload);
        store.put(&digest, payload).unwrap();
        store.put(&digest, payload).unwrap();
        assert_eq!(store.get(&digest, payload.len() as u64).unwrap(), payload);
        assert_eq!(mock.state.puts.load(Ordering::Acquire), 2);
        assert_eq!(mock.state.conditional.load(Ordering::Acquire), 2);
        assert!(store.get(&[0; 32], payload.len() as u64).is_err());
        for bytes in mock.state.objects.lock().unwrap().values_mut() {
            bytes[0] ^= 1;
        }
        assert!(store.get(&digest, payload.len() as u64).is_err());
        assert!(store.put(&digest, payload).is_err());
        for bytes in mock.state.objects.lock().unwrap().values_mut() {
            *bytes = payload.to_vec();
        }
        mock.state.oversized.store(true, Ordering::Release);
        assert!(store.get(&digest, payload.len() as u64).is_err());
    }

    #[test]
    fn s3_errors_never_fall_back_to_unconditional_writes() {
        let mock = MockS3::start();
        let store = mock.store();
        mock.state.reject_put.store(true, Ordering::Release);
        assert!(store.put(&chunk_digest(b"payload"), b"payload").is_err());
        assert!(mock.state.objects.lock().unwrap().is_empty());
        assert_eq!(
            mock.state.puts.load(Ordering::Acquire),
            mock.state.conditional.load(Ordering::Acquire)
        );
        let redirect_trap = TcpListener::bind("127.0.0.1:0").unwrap();
        redirect_trap.set_nonblocking(true).unwrap();
        mock.state.redirect_port.store(
            usize::from(redirect_trap.local_addr().unwrap().port()),
            Ordering::Release,
        );
        mock.state.redirect.store(true, Ordering::Release);
        assert!(store.get(&chunk_digest(b"payload"), 7).is_err());
        assert!(
            matches!(redirect_trap.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }

    #[test]
    fn s3_whole_operation_deadline_includes_retry_and_response_time() {
        let mock = MockS3::start();
        let store = mock.store();
        mock.state.slow.store(true, Ordering::Release);
        let started = Instant::now();
        assert!(store.put(&chunk_digest(b"payload"), b"payload").is_err());
        assert!(started.elapsed() < Duration::from_millis(1800));
    }

    #[test]
    fn s3_replay_verification_shares_the_put_deadline() {
        let mock = MockS3::start();
        let store = mock.store();
        let payload = b"payload";
        let digest = chunk_digest(payload);
        store.put(&digest, payload).unwrap();
        // Each individual request fits its one-second timeout, but the PUT
        // precondition response plus verification GET cannot both fit.
        mock.state.delay_millis.store(900, Ordering::Release);
        let started = Instant::now();
        assert!(store.put(&digest, payload).is_err());
        assert!(started.elapsed() < Duration::from_millis(1600));
    }

    /// Explicit opt-in qualification against an operator-selected bucket. Uses
    /// standard AWS credential environment and intentionally leaves one immutable
    /// object; the backend has no deletion API. Never runs in ordinary suites.
    #[test]
    #[ignore = "set FOKS_CHUNK_LIVE_CONFIG to a TOML S3 config path and run explicitly"]
    fn live_s3_immutable_roundtrip() {
        let path =
            std::env::var_os("FOKS_CHUNK_LIVE_CONFIG").expect("FOKS_CHUNK_LIVE_CONFIG required");
        let config: ChunkStoreConfig =
            toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert!(matches!(config, ChunkStoreConfig::S3 { .. }));
        let store = config.build().unwrap();
        let mut payload = vec![0u8; 256];
        getrandom::fill(&mut payload).unwrap();
        let digest = chunk_digest(&payload);
        store.put(&digest, &payload).unwrap();
        store.put(&digest, &payload).unwrap();
        assert_eq!(store.get(&digest, payload.len() as u64).unwrap(), payload);
    }
}
