use super::{CaptureLimits, Control, Result, ScanError, MAX_KEYS};
use nix::{
    dir::Dir,
    fcntl::{openat, OFlag},
    sys::stat::Mode,
};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

const DATABASE: &str = "foks-server.sqlite";
const MANIFEST: &str = "key-manifest.txt";
const SIDECARS: [&str; 3] = [
    "foks-server.sqlite-wal",
    "foks-server.sqlite-shm",
    "foks-server.sqlite-journal",
];
const FLAGS: OFlag = OFlag::O_RDONLY
    .union(OFlag::O_CLOEXEC)
    .union(OFlag::O_NOFOLLOW)
    .union(OFlag::O_NONBLOCK);

fn io(error: nix::errno::Errno) -> ScanError {
    std::io::Error::from_raw_os_error(error as i32).into()
}
fn open_directory(path: &Path) -> Result<Dir> {
    if path.canonicalize()? != path || !path.is_absolute() {
        return Err(ScanError::Unsupported("noncanonical backup directory"));
    }
    let mut dir = Dir::open("/", FLAGS | OFlag::O_DIRECTORY, Mode::empty()).map_err(io)?;
    for part in path.components() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => {
                dir = Dir::openat(
                    &dir,
                    Path::new(name),
                    FLAGS | OFlag::O_DIRECTORY,
                    Mode::empty(),
                )
                .map_err(io)?
            }
            _ => return Err(ScanError::Unsupported("backup directory component")),
        }
    }
    Ok(dir)
}
fn open_file(dir: &Dir, name: &str) -> Result<File> {
    let file = File::from(openat(dir, name, FLAGS, Mode::empty()).map_err(io)?);
    if !file.metadata()?.is_file() {
        return Err(ScanError::Unsupported("nonregular backup file"));
    }
    Ok(file)
}
fn names(dir: &mut Dir, keys: bool, control: &Control) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for next in dir.iter() {
        control.check()?;
        let entry = next.map_err(io)?;
        let raw = entry.file_name().to_bytes();
        if raw == b"." || raw == b".." {
            continue;
        }
        if raw.is_empty() || raw.len() > 96 || !raw.is_ascii() || raw.contains(&b'/') {
            return Err(ScanError::Unsupported("backup filename"));
        }
        let name =
            std::str::from_utf8(raw).map_err(|_| ScanError::Unsupported("backup filename"))?;
        if !keys
            && name != DATABASE
            && name != MANIFEST
            && name != "keys"
            && !SIDECARS.contains(&name)
        {
            return Err(ScanError::Unsupported("unexpected backup root entry"));
        }
        if keys
            && (!name.ends_with(".key")
                || !raw
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.')))
        {
            return Err(ScanError::Unsupported("backup key filename"));
        }
        if names.len() >= if keys { MAX_KEYS } else { 6 } {
            return Err(ScanError::Unsupported("backup file count"));
        }
        names.try_reserve(1).map_err(|_| ScanError::Allocation)?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(name.len())
            .map_err(|_| ScanError::Allocation)?;
        owned.push_str(name);
        names.push(owned);
    }
    names.sort();
    if names.windows(2).any(|v| v[0] == v[1]) {
        return Err(ScanError::Unsupported("duplicate filename"));
    }
    Ok(names)
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct Entry {
    key: bool,
    name: String,
    length: u64,
    hash: [u8; 32],
    identity: (u64, u64),
}
fn file_hash(
    file: &mut File,
    limit: u64,
    control: &Control,
    mut destination: Option<&mut File>,
) -> Result<(u64, [u8; 32])> {
    let length = file.metadata()?.len();
    if length > limit {
        return Err(ScanError::Unsupported("backup file size"));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 32768];
    let mut read = 0u64;
    loop {
        control.check()?;
        // Read at most the captured length plus one byte; an append cannot make
        // copying or hashing unbounded.
        let capacity = usize::try_from((length - read).min(buffer.len() as u64))
            .map_err(|_| ScanError::Allocation)?;
        if capacity == 0 {
            break;
        }
        let count = file.read(&mut buffer[..capacity])?;
        if count == 0 {
            return Err(ScanError::Unsupported("backup file shortened"));
        }
        hash.update(&buffer[..count]);
        if let Some(out) = destination.as_mut() {
            out.write_all(&buffer[..count])?;
        }
        read += count as u64;
    }
    if file.read(&mut buffer[..1])? != 0 || file.metadata()?.len() != length {
        return Err(ScanError::Unsupported("backup file changed"));
    }
    control.check()?;
    Ok((length, hash.finalize().into()))
}
fn inventory(
    root: &mut Dir,
    limits: CaptureLimits,
    control: &Control,
    copy: Option<&Path>,
) -> Result<Vec<Entry>> {
    let root_names = names(root, false, control)?;
    if ![DATABASE, MANIFEST, "keys"]
        .iter()
        .all(|required| root_names.iter().any(|v| v == required))
    {
        return Err(ScanError::Unsupported("incomplete backup package"));
    }
    let mut keys =
        Dir::openat(&*root, "keys", FLAGS | OFlag::O_DIRECTORY, Mode::empty()).map_err(io)?;
    let key_names = names(&mut keys, true, control)?;
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(root_names.len() + key_names.len())
        .map_err(|_| ScanError::Allocation)?;
    for (is_key, name) in root_names
        .into_iter()
        .filter(|v| v != "keys")
        .map(|v| (false, v))
        .chain(key_names.into_iter().map(|v| (true, v)))
    {
        let dir = if is_key { &keys } else { &*root };
        let mut input = open_file(dir, &name)?;
        let metadata = input.metadata()?;
        let identity = (metadata.dev(), metadata.ino());
        if entries.iter().any(|v: &Entry| v.identity == identity) {
            return Err(ScanError::Unsupported("aliased backup files"));
        }
        let limit = if is_key {
            if name == crate::keys::WRAPPING_KEY_FILE {
                80
            } else {
                96
            }
        } else if name == DATABASE {
            limits.maximum_database_bytes
        } else if name == MANIFEST {
            4096
        } else {
            0
        };
        if metadata.len() > limit || (is_key && metadata.len() != limit) {
            return Err(ScanError::Unsupported("backup file or live sidecar size"));
        }
        let mut output = if let Some(destination) = copy {
            let path = if is_key {
                destination.join("keys").join(&name)
            } else {
                destination.join(&name)
            };
            Some(
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(path)?,
            )
        } else {
            None
        };
        let (length, hash) = file_hash(&mut input, limit, control, output.as_mut())?;
        if let Some(output) = output {
            output.sync_all()?;
        }
        entries.push(Entry {
            key: is_key,
            name,
            length,
            hash,
            identity,
        });
    }
    entries.sort_by(|a, b| (a.key, &a.name).cmp(&(b.key, &b.name)));
    Ok(entries)
}
fn digest(entries: &[Entry]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"foks-restricted-finalized-package-v1\0");
    for item in entries {
        hash.update([u8::from(item.key)]);
        hash.update((item.name.len() as u64).to_le_bytes());
        hash.update(item.name.as_bytes());
        hash.update(item.length.to_le_bytes());
        hash.update(item.hash);
    }
    hash.finalize().into()
}
pub(super) struct Package {
    directory: tempfile::TempDir,
    entries: Vec<Entry>,
    limits: CaptureLimits,
    digest: [u8; 32],
    header_modes: [u8; 2],
}
impl Package {
    pub fn capture(
        artifacts: &crate::BackupArtifacts,
        limits: CaptureLimits,
        control: &Control,
    ) -> Result<Self> {
        let source = artifacts
            .database
            .parent()
            .ok_or(ScanError::Unsupported("backup parent"))?;
        if artifacts.database != source.join(DATABASE)
            || artifacts.key_directory != source.join("keys")
            || artifacts.key_manifest != source.join(MANIFEST)
        {
            return Err(ScanError::Unsupported("backup artifact layout"));
        }
        let mut root = open_directory(source)?;
        let directory = tempfile::Builder::new()
            .prefix("foks-restricted-")
            .tempdir_in(std::env::temp_dir().canonicalize()?)?;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(directory.path().join("keys"))?;
        let before = inventory(&mut root, limits, control, None)?;
        #[cfg(test)]
        AFTER_INVENTORY.with(|slot| {
            if let Some(action) = slot.borrow_mut().take() {
                action();
            }
        });
        let copied = inventory(&mut root, limits, control, Some(directory.path()))?;
        let after = inventory(&mut root, limits, control, None)?;
        if before != copied || before != after {
            return Err(ScanError::Unsupported("backup changed during capture"));
        }
        File::open(directory.path().join("keys"))?.sync_all()?;
        File::open(directory.path())?.sync_all()?;
        let digest = digest(&before);
        let private = open_directory(directory.path())?;
        let mut database_file = open_file(&private, DATABASE)?;
        let mut header = [0; 100];
        database_file.read_exact(&mut header)?;
        if &header[..16] != b"SQLite format 3\0"
            || !matches!(header[18], 1 | 2)
            || !matches!(header[19], 1 | 2)
        {
            return Err(ScanError::Unsupported("SQLite file header"));
        }
        let mut package = Self {
            directory,
            entries: before,
            limits,
            digest,
            header_modes: [header[18], header[19]],
        };
        package.verify(control)?;
        Ok(package)
    }
    pub fn database(&self) -> PathBuf {
        self.directory.path().join(DATABASE)
    }
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub fn header_modes(&self) -> [u8; 2] {
        self.header_modes
    }
    pub fn key_names(&self) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter(|v| v.key)
            .map(|v| v.name.as_str())
    }
    pub fn manifest(&self) -> Result<Vec<u8>> {
        let root = open_directory(self.directory.path())?;
        let file = open_file(&root, MANIFEST)?;
        let length = file.metadata()?.len();
        if length > 4096 {
            return Err(ScanError::Unsupported("manifest size"));
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length as usize)
            .map_err(|_| ScanError::Allocation)?;
        file.take(4097).read_to_end(&mut bytes)?;
        // Existing fixed manifest decoder slices hex by byte positions.
        if bytes.len() != length as usize || !bytes.is_ascii() {
            return Err(ScanError::Unsupported("manifest changed or non-ASCII"));
        }
        Ok(bytes)
    }
    #[cfg(any(test, feature = "celld-proof"))]
    pub(super) fn copy_keys(&mut self, destination: &Path, control: &Control) -> Result<()> {
        self.verify(control)?;
        std::fs::DirBuilder::new().mode(0o700).create(destination)?;
        let root = open_directory(self.directory.path())?;
        let keys =
            Dir::openat(&root, "keys", FLAGS | OFlag::O_DIRECTORY, Mode::empty()).map_err(io)?;
        for entry in self.entries.iter().filter(|entry| entry.key) {
            control.check()?;
            let mut source = open_file(&keys, &entry.name)?;
            let mut target = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(destination.join(&entry.name))?;
            let observed = file_hash(&mut source, entry.length, control, Some(&mut target))?;
            if observed != (entry.length, entry.hash) {
                return Err(ScanError::Unsupported("key changed during import"));
            }
            target.sync_all()?;
            let mut check = open_file(&open_directory(destination)?, &entry.name)?;
            if file_hash(&mut check, entry.length, control, None)? != observed {
                return Err(ScanError::Unsupported("copied key mismatch"));
            }
        }
        File::open(destination)?.sync_all()?;
        self.verify(control)
    }
    pub fn verify(&mut self, control: &Control) -> Result<()> {
        let mut root = open_directory(self.directory.path())?;
        let current = inventory(&mut root, self.limits, control, None)?;
        if digest(&current) != self.digest {
            return Err(ScanError::Unsupported("private captured package changed"));
        }
        Ok(())
    }
}

#[cfg(test)]
thread_local! { static AFTER_INVENTORY: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
pub(super) fn after_inventory(action: impl FnOnce() + 'static) {
    AFTER_INVENTORY.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(action));
    });
}
