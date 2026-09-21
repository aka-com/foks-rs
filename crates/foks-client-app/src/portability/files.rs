use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Read as _;
use std::path::{Path, PathBuf};

pub(super) const MAX_ENTRIES: usize = 8192;
pub(super) const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
pub(super) const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Artifact {
    pub path: String,
    pub size: u64,
    pub mode: u32,
    pub sha256: [u8; 32],
    pub soft: bool,
}
pub(super) fn private_directory(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return Err(Error::StatePathChanged);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if m.mode() & 0o077 != 0 || m.uid() != rustix::process::getuid().as_raw() {
            return Err(Error::InvalidConfig(
                "snapshot directory is not private and owned",
            ));
        }
    }
    Ok(())
}
pub(super) fn entries(directory: &Path) -> Result<Vec<(String, PathBuf)>> {
    private_directory(directory)?;
    let mut entries = Vec::new();
    for item in fs::read_dir(directory)? {
        if entries.len() == MAX_ENTRIES {
            return Err(Error::InvalidConfig(
                "snapshot inventory exceeds entry limit",
            ));
        }
        let item = item?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| Error::InvalidConfig("snapshot filename is not UTF8"))?;
        entries.push((name, item.path()));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}
pub(super) fn regular(path: &Path) -> Result<fs::Metadata> {
    regular_bounded(path, MAX_FILE)
}
pub(super) fn regular_bounded(path: &Path, maximum: u64) -> Result<fs::Metadata> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_file() || m.file_type().is_symlink() {
        return Err(Error::InvalidConfig("snapshot entry is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if m.nlink() != 1 || m.uid() != rustix::process::getuid().as_raw() {
            return Err(Error::InvalidConfig(
                "snapshot entry is linked or not owned",
            ));
        }
        if m.len() > 4096 && m.blocks().saturating_mul(512) < m.len() {
            return Err(Error::InvalidConfig("snapshot entry is sparse"));
        }
    }
    if m.len() > maximum {
        return Err(Error::InvalidConfig("snapshot file exceeds limit"));
    }
    Ok(m)
}
/// Everything the portability change detector knows about a file's contents,
/// and nothing else.
///
/// [`changed`] is defined as inequality of this value, so the two can never
/// drift apart: two observations of one path carry equal identities exactly
/// when `changed` reports the file as unchanged between them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContentIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    ctime: i64,
    #[cfg(unix)]
    ctime_nsec: i64,
    len: u64,
    modified: Option<std::time::SystemTime>,
}
impl ContentIdentity {
    pub(crate) fn of(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt as _;
        Self {
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            ctime: metadata.ctime(),
            #[cfg(unix)]
            ctime_nsec: metadata.ctime_nsec(),
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }
    pub(super) fn len(&self) -> u64 {
        self.len
    }
}
pub(crate) fn changed(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    ContentIdentity::of(a) != ContentIdentity::of(b)
}
pub(super) fn open_regular(path: &Path) -> Result<File> {
    open_regular_bounded(path, MAX_FILE)
}
pub(super) fn open_regular_bounded(path: &Path, maximum: u64) -> Result<File> {
    let before = regular_bounded(path, maximum)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    if changed(&before, &file.metadata()?) {
        return Err(Error::StatePathChanged);
    }
    Ok(file)
}
/// Hashes `path` and also returns the content identity the hash was taken
/// under, which this has just proven equal to the identity observed after the
/// last byte was read. A cache may record the pair; it must never pair a digest
/// with an identity read at any other moment.
fn identified_artifact(
    root: &Path,
    path: &Path,
    soft: bool,
) -> Result<(Artifact, ContentIdentity)> {
    let file = open_regular(path)?;
    let before = file.metadata()?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut total = 0u64;
    let mut reader = &file;
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total = total.checked_add(n as u64).ok_or(Error::StatePathChanged)?;
        if total > before.len() {
            return Err(Error::StatePathChanged);
        }
        hash.update(&buffer[..n]);
    }
    if total != before.len()
        || changed(&before, &file.metadata()?)
        || changed(&before, &regular(path)?)
    {
        return Err(Error::StatePathChanged);
    }
    Ok((
        Artifact {
            path: relative_path(root, path)?,
            size: total,
            mode: 0o600,
            sha256: hash.finalize().into(),
            soft,
        },
        ContentIdentity::of(&before),
    ))
}
fn relative_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| Error::StatePathChanged)?
        .to_str()
        .ok_or(Error::StatePathChanged)?
        .to_owned();
    if relative.len() > 512 {
        return Err(Error::InvalidConfig("snapshot path exceeds limit"));
    }
    Ok(relative)
}

/// Counts of what an [`ArtifactCache`] hashed and what it reused. Kept for the
/// tests that assert the reduction; nothing in the maintenance paths reads it.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ArtifactCacheCounters {
    pub hashed: u64,
    pub hashed_bytes: u64,
    pub reused: u64,
    pub reused_bytes: u64,
}

/// File digests already computed under one maintenance guard, keyed by the path
/// they were read from and the [`ContentIdentity`] they were read at.
///
/// Correctness rests on one equivalence: a cached digest is returned only when
/// a fresh observation of the same path yields the identity recorded with the
/// digest, which by the definition of [`changed`] is exactly the case in which
/// the change detector reports the file as unchanged. Every caller of this
/// cache already refuses to proceed when `changed` reports a file as changed,
/// so reuse never admits a file those callers would have rejected; it drops
/// only the second full read of a file they would have accepted.
///
/// The weakening, stated plainly: a writer that can alter a file's bytes
/// without moving its inode, size, mtime or ctime -- raw device access, or a
/// write landing inside the filesystem's ctime granularity -- defeats the
/// reuse where a re-hash would have caught it. That writer already defeats
/// `changed`, which every open, inspection and archive write here relies on, so
/// this widens an existing window rather than opening a new one. The cache also
/// lives only inside a [`super::ClientStateMaintenanceGuard`], which holds the
/// exclusive path reservation and the state root's own lock files for its whole
/// lifetime, so no cooperating writer exists while entries are held.
#[derive(Default)]
pub(super) struct ArtifactCache {
    entries: std::collections::HashMap<PathBuf, (ContentIdentity, [u8; 32])>,
    #[cfg(test)]
    counters: ArtifactCacheCounters,
}

impl ArtifactCache {
    /// The artifact for `path`, hashing it only if no entry recorded under this
    /// path still matches the file's current content identity.
    ///
    /// The non-hash validation is never skipped: [`regular`] re-runs the
    /// regular-file, ownership, link-count, sparseness and size checks on every
    /// call, hit or miss.
    pub(super) fn artifact(&mut self, root: &Path, path: &Path, soft: bool) -> Result<Artifact> {
        let identity = ContentIdentity::of(&regular(path)?);
        if let Some((recorded, sha256)) = self.entries.get(path) {
            if *recorded == identity {
                #[cfg(test)]
                {
                    self.counters.reused += 1;
                    self.counters.reused_bytes += identity.len();
                }
                return Ok(Artifact {
                    path: relative_path(root, path)?,
                    size: identity.len(),
                    mode: 0o600,
                    sha256: *sha256,
                    soft,
                });
            }
        }
        let (artifact, identity) = identified_artifact(root, path, soft)?;
        #[cfg(test)]
        {
            self.counters.hashed += 1;
            self.counters.hashed_bytes += artifact.size;
        }
        // A path whose file keeps changing replaces its own entry, so the map
        // is bounded by the number of distinct paths. The cap only guards
        // against a guard that walks more roots than one snapshot may hold.
        if self.entries.len() >= 4 * MAX_ENTRIES {
            self.entries.clear();
        }
        self.entries
            .insert(path.to_owned(), (identity, artifact.sha256));
        Ok(artifact)
    }

    /// Re-keys every entry under `from` to the same relative location under
    /// `to`, after a rename that moved a whole tree.
    ///
    /// Only the key moves: each entry keeps the content identity it was hashed
    /// at, so a later reuse still requires the file now at the new path to
    /// present the same device, inode, ctime, size and mtime. A rename
    /// preserves all of those for the files inside the moved tree -- and,
    /// because `rename(2)` cannot cross a filesystem, preserves the device in
    /// particular -- while anything substituted at the new path differs in at
    /// least the inode or the ctime and is re-hashed.
    pub(super) fn rebase(&mut self, from: &Path, to: &Path) {
        let mut moved = std::collections::HashMap::with_capacity(self.entries.len());
        let mut untouched = Vec::new();
        for (path, entry) in std::mem::take(&mut self.entries) {
            match path.strip_prefix(from) {
                Ok(relative) => {
                    moved.insert(to.join(relative), entry);
                }
                Err(_) => untouched.push((path, entry)),
            }
        }
        // An entry the rename moved wins over one recorded at the same
        // destination path before it, because after the rename that earlier
        // file is no longer there.
        for (path, entry) in untouched {
            moved.entry(path).or_insert(entry);
        }
        self.entries = moved;
    }

    #[cfg(test)]
    pub(super) fn counters(&self) -> ArtifactCacheCounters {
        self.counters
    }
}

/// Presence checks on authority metadata must not follow dangling symlinks.
pub(super) fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            crate::prepare_private_directory(parent).unwrap();
        }
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        use std::io::Write as _;
        options.open(path).unwrap().write_all(bytes).unwrap();
    }

    fn on_disk(path: &Path) -> [u8; 32] {
        Sha256::digest(std::fs::read(path).unwrap()).into()
    }

    /// The safety property the cache is built on: every way of changing a
    /// file's bytes moves its content identity, so no key can stay equal across
    /// different bytes.
    #[test]
    fn every_content_change_moves_the_content_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let path = root.join("file");
        write(&path, b"original contents");
        let original = ContentIdentity::of(&regular(&path).unwrap());

        // Same length, different bytes.
        write(&path, b"replaced contents");
        let rewritten = ContentIdentity::of(&regular(&path).unwrap());
        assert_ne!(original, rewritten);

        // Shorter, and longer.
        write(&path, b"short");
        let truncated = ContentIdentity::of(&regular(&path).unwrap());
        assert_ne!(rewritten, truncated);
        write(&path, b"short and then some more");
        assert_ne!(truncated, ContentIdentity::of(&regular(&path).unwrap()));

        // A different file renamed over the path: a new inode, so a new
        // identity even if its size and mtime happened to match.
        let replacement = root.join("replacement");
        write(&replacement, b"short and then some more");
        let before = ContentIdentity::of(&regular(&path).unwrap());
        std::fs::rename(&replacement, &path).unwrap();
        assert_ne!(before, ContentIdentity::of(&regular(&path).unwrap()));

        // And the identity is exactly what the change detector compares.
        let a = std::fs::symlink_metadata(&path).unwrap();
        write(&path, b"moved again");
        let b = std::fs::symlink_metadata(&path).unwrap();
        assert!(changed(&a, &b));
        assert_eq!(
            changed(&a, &a),
            ContentIdentity::of(&a) != ContentIdentity::of(&a)
        );
    }

    /// Whatever the cache returns -- reused or freshly hashed -- is the digest
    /// of the bytes on disk at that moment.
    #[test]
    fn a_reused_digest_is_the_digest_of_the_same_bytes() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let path = root.join("file");
        let mut cache = ArtifactCache::default();

        for contents in [
            b"one".as_slice(),
            b"one".as_slice(),
            b"two".as_slice(),
            b"a much longer body of bytes".as_slice(),
            b"a much longer body of byteS".as_slice(),
        ] {
            write(&path, contents);
            for _ in 0..3 {
                let artifact = cache.artifact(&root, &path, false).unwrap();
                assert_eq!(artifact.sha256, on_disk(&path));
                assert_eq!(artifact.size, contents.len() as u64);
                assert_eq!(artifact.path, "file");
            }
        }
        let counters = cache.counters();
        assert_eq!(counters.hashed, 5, "one hash per distinct file state");
        assert_eq!(counters.reused, 10, "the repeats within a state are reused");
    }

    /// The measured reduction: a repeat inspection re-reads only the files
    /// whose content identity moved.
    #[test]
    fn a_repeat_inspection_rehashes_only_what_moved() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let sizes = [64 * 1024usize, 256 * 1024, 1024];
        let paths: Vec<_> = sizes
            .iter()
            .enumerate()
            .map(|(index, size)| {
                let path = root.join(format!("file-{index}"));
                write(&path, &vec![index as u8; *size]);
                path
            })
            .collect();
        let total: u64 = sizes.iter().map(|size| *size as u64).sum();

        let mut cache = ArtifactCache::default();
        let inspect = |cache: &mut ArtifactCache| -> Vec<Artifact> {
            paths
                .iter()
                .map(|path| cache.artifact(&root, path, false).unwrap())
                .collect()
        };

        let first = inspect(&mut cache);
        assert_eq!(cache.counters().hashed_bytes, total);
        assert_eq!(cache.counters().reused_bytes, 0);

        // Three further inspections under the same guard read nothing.
        for _ in 0..3 {
            assert_eq!(inspect(&mut cache), first);
        }
        assert_eq!(
            cache.counters().hashed_bytes,
            total,
            "an unchanged state is never re-hashed"
        );
        assert_eq!(cache.counters().reused_bytes, 3 * total);

        // Touching one file costs exactly that file, not the whole state.
        write(&paths[1], &vec![0xab; sizes[1]]);
        let after = inspect(&mut cache);
        assert_eq!(cache.counters().hashed_bytes, total + sizes[1] as u64);
        assert_ne!(after[1].sha256, first[1].sha256);
        assert_eq!(after[1].sha256, on_disk(&paths[1]));
        assert_eq!(after[0], first[0]);
        assert_eq!(after[2], first[2]);
    }

    /// Rebasing after a rename moves the keys and nothing else, so a file that
    /// really did move with the tree is reused and one that was substituted at
    /// the new path is re-read.
    #[test]
    fn rebasing_after_a_rename_still_checks_every_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("source");
        let destination = temporary.path().join("destination");
        write(&source.join("kept"), b"carried across the rename");
        write(&source.join("nested/deep"), b"also carried");
        let paths = ["kept", "nested/deep"];

        let mut cache = ArtifactCache::default();
        let before: Vec<_> = paths
            .iter()
            .map(|name| cache.artifact(&source, &source.join(name), false).unwrap())
            .collect();
        let hashed = cache.counters().hashed;

        std::fs::rename(&source, &destination).unwrap();
        // Without the rebase the new paths are simply unknown keys.
        let mut cold = ArtifactCache::default();
        cold.artifact(&destination, &destination.join("kept"), false)
            .unwrap();
        assert_eq!(cold.counters().hashed, 1);

        cache.rebase(&source, &destination);
        let after: Vec<_> = paths
            .iter()
            .map(|name| {
                cache
                    .artifact(&destination, &destination.join(name), false)
                    .unwrap()
            })
            .collect();
        assert_eq!(after, before, "the same files, at the same relative paths");
        assert_eq!(cache.counters().hashed, hashed, "nothing was re-read");

        // A file substituted at the destination has a new inode, so the carried
        // identity no longer matches and the digest is recomputed.
        write(&destination.join("kept"), b"substituted after the rename");
        let substituted = cache
            .artifact(&destination, &destination.join("kept"), false)
            .unwrap();
        assert_eq!(cache.counters().hashed, hashed + 1);
        assert_ne!(substituted.sha256, before[0].sha256);
        assert_eq!(substituted.sha256, on_disk(&destination.join("kept")));
    }

    /// Replays the inspection sequence a relocation performs -- inspect,
    /// verify for trust normalization, re-inspect, inspect again before the
    /// rename, then inspect the destination after it -- over a synthetic state
    /// directory, and measures what each pass reads.
    ///
    /// Before this cache every pass read every byte. The assertion is on bytes
    /// rather than wall time so it states the same reduction deterministically.
    #[test]
    fn a_relocation_shaped_sequence_reads_the_state_once() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("state");
        let destination = temporary.path().join("moved");
        let layout = [
            ("profiles/local/hard.sqlite3", 1024 * 1024),
            ("profiles/local/soft.sqlite3", 512 * 1024),
            ("profiles/local/credentials/account.work.fks", 2048),
            ("profiles.toml", 512),
            ("client-state.toml", 256),
        ];
        for (index, (name, size)) in layout.iter().enumerate() {
            write(&source.join(name), &vec![index as u8; *size]);
        }
        let total: u64 = layout.iter().map(|(_, size)| *size as u64).sum();

        let mut cache = ArtifactCache::default();
        let inspect = |cache: &mut ArtifactCache, root: &Path| -> Vec<Artifact> {
            layout
                .iter()
                .map(|(name, _)| cache.artifact(root, &root.join(name), false).unwrap())
                .collect()
        };

        // The three passes relocate makes before the rename.
        let prepared = inspect(&mut cache, &source);
        assert_eq!(inspect(&mut cache, &source), prepared);
        assert_eq!(inspect(&mut cache, &source), prepared);

        std::fs::rename(&source, &destination).unwrap();
        cache.rebase(&source, &destination);

        // And the verification pass after it, at the new paths.
        assert_eq!(inspect(&mut cache, &destination), prepared);
        assert_eq!(
            cache.counters().hashed_bytes,
            total,
            "the state is read once across the whole relocation"
        );
        assert_eq!(cache.counters().reused_bytes, 3 * total);

        // The reduction never costs detection: a file altered at the
        // destination is read again and reports its new digest.
        let altered = destination.join(layout[0].0);
        write(&altered, &vec![0xff; layout[0].1]);
        let after = inspect(&mut cache, &destination);
        assert_eq!(
            cache.counters().hashed_bytes,
            total + layout[0].1 as u64,
            "and only that file is read again"
        );
        assert_ne!(after[0].sha256, prepared[0].sha256);
        assert_eq!(after[0].sha256, on_disk(&altered));
    }

    /// A file changed between two phases of one maintenance operation is still
    /// detected, because the cache never answers from a stale identity.
    #[test]
    fn a_file_changed_between_phases_is_still_detected() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("state");
        let path = root.join("file");
        write(&path, b"phase one bytes...");
        let mut cache = ArtifactCache::default();
        let phase_one = cache.artifact(&root, &path, false).unwrap();

        // Same length, different bytes: the comparison a verification pass
        // makes against the recorded artifact must fail.
        write(&path, b"phase two bytes...");
        let phase_two = cache.artifact(&root, &path, false).unwrap();
        assert_eq!(phase_one.size, phase_two.size);
        assert_ne!(phase_one, phase_two);
        assert_eq!(phase_two.sha256, on_disk(&path));

        // The non-hash validation still runs on a reused entry: a second hard
        // link to the file is rejected even though the digest is cached.
        let reused = cache.artifact(&root, &path, false).unwrap();
        assert_eq!(reused, phase_two);
        assert_eq!(cache.counters().reused, 1);
        std::fs::hard_link(&path, root.join("link")).unwrap();
        assert!(matches!(
            cache.artifact(&root, &path, false),
            Err(Error::InvalidConfig(
                "snapshot entry is linked or not owned"
            ))
        ));
    }
}
