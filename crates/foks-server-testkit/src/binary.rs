use std::io::Read;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::{TestClient, TestEnvironment, TestProfile};

const MAXIMUM_CAPTURE_BYTES: usize = 1024 * 1024;

struct BinaryFiles {
    root_key: PathBuf,
    probe_certificates: Vec<PathBuf>,
    probe_private_key: PathBuf,
}

pub struct BinaryExit {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub struct BinaryServer {
    environment: TestEnvironment,
    binary: PathBuf,
    child: Option<Child>,
    stdout: Option<JoinHandle<Vec<u8>>>,
    stderr: Option<JoinHandle<Vec<u8>>>,
}

impl BinaryServer {
    pub fn start(
        environment: &TestEnvironment,
        binary: impl AsRef<Path>,
    ) -> foks_server::Result<Self> {
        let binary = validate_binary(binary.as_ref())?;
        claim_environment(environment)?;
        let result = Self::start_claimed(environment.clone(), binary);
        if result.is_err() {
            release_environment(environment);
        }
        result
    }

    fn start_claimed(environment: TestEnvironment, binary: PathBuf) -> foks_server::Result<Self> {
        let addresses = reserve_addresses(&environment)?;
        let files = prepare_binary_files(&environment)?;
        let mut command = serve_command(&binary, &environment, addresses, &files);
        let mut child = command.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or(foks_server::Error::Config("binary stdout is not piped"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or(foks_server::Error::Config("binary stderr is not piped"))?;
        let stdout = Some(capture(stdout));
        let stderr = Some(capture(stderr));
        let mut server = Self {
            environment,
            binary,
            child: Some(child),
            stdout,
            stderr,
        };
        if let Err(error) = server.await_readiness() {
            let _ = server.force_shutdown();
            return Err(error);
        }
        Ok(server)
    }

    pub fn addresses(&self) -> foks_server::ServerAddresses {
        self.environment
            .addresses()
            .expect("binary server reserved addresses")
    }

    pub fn backup_named(&self, name: &str) -> foks_server::Result<foks_server::BackupArtifacts> {
        if !valid_name(name) {
            return Err(foks_server::Error::Config("invalid binary backup name"));
        }
        let files = prepare_binary_files(&self.environment)?;
        let destination = self.environment.inner.paths.backup().join(name);
        let mut command = Command::new(&self.binary);
        command
            .arg("backup")
            .arg("--database")
            .arg(self.environment.inner.paths.database())
            .arg("--key-directory")
            .arg(self.environment.inner.paths.keys())
            .arg("--root-key-file")
            .arg(files.root_key)
            .arg("--destination")
            .arg(&destination)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let status = run_bounded(&mut command, Duration::from_secs(10))?;
        if !status.success() {
            return Err(foks_server::Error::Config("binary backup command failed"));
        }
        Ok(foks_server::BackupArtifacts {
            database: destination.join("foks-server.sqlite"),
            key_directory: destination.join("keys"),
            key_manifest: destination.join("key-manifest.txt"),
        })
    }

    pub fn graceful_shutdown(mut self) -> foks_server::Result<BinaryExit> {
        let child = self
            .child
            .as_mut()
            .ok_or(foks_server::Error::Config("binary server is not running"))?;
        #[cfg(unix)]
        {
            let status = Command::new("kill")
                .arg("-TERM")
                .arg(child.id().to_string())
                .status()?;
            if !status.success() {
                return Err(foks_server::Error::Config("failed to signal binary server"));
            }
        }
        #[cfg(not(unix))]
        child.kill()?;
        self.finish(Duration::from_secs(5), false)
    }

    pub fn force_shutdown(mut self) -> foks_server::Result<BinaryExit> {
        self.finish(Duration::from_secs(5), true)
    }

    fn await_readiness(&mut self) -> foks_server::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let client = TestClient::new(&self.environment, "binary-readiness")?;
        loop {
            if self
                .child
                .as_mut()
                .ok_or(foks_server::Error::Config("binary server is not running"))?
                .try_wait()?
                .is_some()
            {
                return Err(foks_server::Error::Config(
                    "binary server exited before readiness",
                ));
            }
            if client.probe_and_pin().is_ok() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(foks_server::Error::Config("binary readiness timed out"));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn finish(&mut self, timeout: Duration, force: bool) -> foks_server::Result<BinaryExit> {
        let child = self
            .child
            .as_mut()
            .ok_or(foks_server::Error::Config("binary server is not running"))?;
        if force {
            child.kill()?;
        }
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill()?;
                let _ = child.wait();
                return Err(foks_server::Error::Config("binary shutdown timed out"));
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        self.child = None;
        let stdout = join_capture(self.stdout.take())?;
        let stderr = join_capture(self.stderr.take())?;
        release_environment(&self.environment);
        Ok(BinaryExit {
            status,
            stdout,
            stderr,
        })
    }
}

impl Drop for BinaryServer {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
            self.child = None;
            release_environment(&self.environment);
        }
        let _ = join_capture(self.stdout.take());
        let _ = join_capture(self.stderr.take());
    }
}

impl TestEnvironment {
    pub fn restore_backup_with_binary(
        binary: impl AsRef<Path>,
        artifacts: &foks_server::BackupArtifacts,
        addresses: foks_server::ServerAddresses,
    ) -> foks_server::Result<Self> {
        let binary = validate_binary(binary.as_ref())?;
        let environment =
            Self::new_with_installation([0x51; 32], Some(addresses), TestProfile::Default)?;
        let backup_directory = artifacts
            .database
            .parent()
            .ok_or(foks_server::Error::Config("backup has no directory"))?;
        if artifacts.key_directory != backup_directory.join("keys")
            || artifacts.key_manifest != backup_directory.join("key-manifest.txt")
        {
            return Err(foks_server::Error::Config(
                "backup artifacts do not share the declared directory",
            ));
        }
        let mut command = Command::new(binary);
        command
            .arg("restore")
            .arg("--backup-directory")
            .arg(backup_directory)
            .arg("--database")
            .arg(environment.inner.paths.database())
            .arg("--key-directory")
            .arg(environment.inner.paths.keys())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let status = run_bounded(&mut command, Duration::from_secs(10))?;
        if !status.success() {
            return Err(foks_server::Error::Config("binary restore command failed"));
        }
        Ok(environment)
    }
}

fn serve_command(
    binary: &Path,
    environment: &TestEnvironment,
    addresses: foks_server::ServerAddresses,
    files: &BinaryFiles,
) -> Command {
    let mut command = Command::new(binary);
    command
        .arg("serve")
        .arg("--canonical-name")
        .arg("localhost")
        .arg("--database")
        .arg(environment.inner.paths.database())
        .arg("--key-directory")
        .arg(environment.inner.paths.keys())
        .arg("--root-key-file")
        .arg(&files.root_key)
        .arg("--probe-certificate-der");
    for certificate in &files.probe_certificates {
        command.arg(certificate);
    }
    command
        .arg("--probe-private-key-der")
        .arg(&files.probe_private_key)
        .arg("--probe-address")
        .arg(addresses.probe.to_string())
        .arg("--public-address")
        .arg(addresses.public_services.to_string())
        .arg("--authenticated-address")
        .arg(addresses.authenticated.to_string())
        .arg("--worker-threads")
        .arg("8")
        .arg("--maximum-pending-connections")
        .arg("32")
        .arg("--maximum-pending-writes")
        .arg("16")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn reserve_addresses(
    environment: &TestEnvironment,
) -> foks_server::Result<foks_server::ServerAddresses> {
    if let Some(addresses) = environment.addresses() {
        return Ok(addresses);
    }
    let listeners = (0..3)
        .map(|_| TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))))
        .collect::<std::io::Result<Vec<_>>>()?;
    let addresses = foks_server::ServerAddresses {
        probe: listeners[0].local_addr()?,
        public_services: listeners[1].local_addr()?,
        authenticated: listeners[2].local_addr()?,
    };
    drop(listeners);
    *environment
        .inner
        .addresses
        .lock()
        .expect("test address lock") = Some(addresses);
    Ok(addresses)
}

fn prepare_binary_files(environment: &TestEnvironment) -> foks_server::Result<BinaryFiles> {
    let directory = environment.inner.paths.logs().join("binary-config");
    std::fs::create_dir_all(&directory)?;
    let root_key = directory.join("root-key.bin");
    write_secure(&root_key, &environment.inner.root_key)?;
    let probe_private_key = directory.join("probe-private-key.der");
    write_secure(&probe_private_key, &environment.inner.tls.private_key)?;
    let mut probe_certificates = Vec::new();
    for (index, certificate) in environment.inner.tls.certificate_chain.iter().enumerate() {
        let path = directory.join(format!("probe-certificate-{index}.der"));
        write_secure(&path, certificate)?;
        probe_certificates.push(path);
    }
    Ok(BinaryFiles {
        root_key,
        probe_certificates,
        probe_private_key,
    })
}

fn write_secure(path: &Path, bytes: &[u8]) -> foks_server::Result<()> {
    match std::fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => return Err(foks_server::Error::Config("binary secret file changed")),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        Err(_) => {}
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    use std::io::Write as _;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn validate_binary(path: &Path) -> foks_server::Result<PathBuf> {
    let path = path.canonicalize()?;
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(foks_server::Error::Config(
            "server binary is not a regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(foks_server::Error::Config(
                "server binary is not executable",
            ));
        }
    }
    Ok(path)
}

fn claim_environment(environment: &TestEnvironment) -> foks_server::Result<()> {
    let mut running = environment
        .inner
        .running
        .lock()
        .expect("test server lifecycle lock");
    if *running {
        return Err(foks_server::Error::Config("test server already running"));
    }
    *running = true;
    Ok(())
}

fn release_environment(environment: &TestEnvironment) {
    *environment
        .inner
        .running
        .lock()
        .expect("test server lifecycle lock") = false;
}

fn capture(mut reader: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut captured = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            let read = match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            let remaining = MAXIMUM_CAPTURE_BYTES.saturating_sub(captured.len());
            captured.extend_from_slice(&buffer[..read.min(remaining)]);
        }
        captured
    })
}

fn run_bounded(command: &mut Command, timeout: Duration) -> foks_server::Result<ExitStatus> {
    let mut child = command.spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let _ = child.wait();
            return Err(foks_server::Error::Config("binary command timed out"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn join_capture(thread: Option<JoinHandle<Vec<u8>>>) -> foks_server::Result<Vec<u8>> {
    thread
        .map(|thread| thread.join().map_err(|_| foks_server::Error::Thread))
        .unwrap_or_else(|| Ok(Vec::new()))
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
