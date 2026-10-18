//! Local, independent-directory loss experiments. These are not AZ emulation:
//! all processes, files and loopback endpoints still share the developer host.

use std::path::Path;
use std::time::{Duration, Instant};

use foks_client::{CreatedSoftwareAccount, PinnedHost};
use foks_server::BackupArtifacts;
use foks_server_testkit::{
    BinaryServer, RecoveryLedger, TestAccountSpec, TestClient, TestEnvironment,
};

use super::{authenticate, binary, budget, verify_all, write_file};

struct Installation {
    source: TestEnvironment,
    server: Option<BinaryServer>,
    client: TestClient,
    host: PinnedHost,
    account: CreatedSoftwareAccount,
    backup: BackupArtifacts,
    ledger: RecoveryLedger,
    // Simulated off-volume recovery material/controller evidence. Kept alive
    // until the experiment ends; never copied from the failed source afterward.
    _vault: tempfile::TempDir,
}

impl Installation {
    fn prepare(label: &str) -> Self {
        let source = TestEnvironment::new().unwrap();
        let server = BinaryServer::start(&source, binary()).unwrap();
        let client = TestClient::new(&source, "volume-controller").unwrap();
        let host = client.probe_and_pin().unwrap().pinned;
        let account = client
            .create_account(&host, &TestAccountSpec::new("volumeuser", 0xa1))
            .unwrap();
        let vault = tempfile::tempdir().unwrap();
        let evidence = std::env::var_os("FOKS_RECOVERY_EVIDENCE_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| vault.path().to_owned());
        std::fs::create_dir_all(&evidence).unwrap();
        let mut ledger =
            RecoveryLedger::create(evidence.join(format!("{label}.jsonl")), budget()).unwrap();
        write_file(
            &client,
            &host,
            &account.credential,
            account.kv_projection[0].root_directory_id,
            "before.txt",
            b"acknowledged before snapshot",
        );
        ledger
            .acknowledged("before.txt", b"acknowledged before snapshot")
            .unwrap();
        let local_backup = server.backup_named("survivor").unwrap();
        let destination = vault.path().join("surviving-backup");
        copy_fixture_tree(local_backup.database.parent().unwrap(), &destination);
        let backup = BackupArtifacts {
            database: destination.join("foks-server.sqlite"),
            key_directory: destination.join("keys"),
            key_manifest: destination.join("key-manifest.txt"),
        };
        ledger
            .milestone("complete-snapshot-copied-outside-source-directory")
            .unwrap();
        Self {
            source,
            server: Some(server),
            client,
            host,
            account,
            backup,
            ledger,
            _vault: vault,
        }
    }

    fn write(&mut self, name: &str, bytes: &[u8]) {
        write_file(
            &self.client,
            &self.host,
            &self.account.credential,
            self.account.kv_projection[0].root_directory_id,
            name,
            bytes,
        );
        self.ledger.acknowledged(name, bytes).unwrap();
    }

    fn verify(&mut self, expected: usize) {
        verify_all(
            &self.client,
            &self.host,
            &self.account.credential,
            expected,
            &mut self.ledger,
        );
    }

    fn destroy_source(&mut self) {
        let server = self.server.take().unwrap();
        assert!(!server.force_shutdown().unwrap().status.success());
        // Delete only fixture-owned server material. Client hard/soft/protected
        // state remains outside these paths, as does the controller's backup.
        for directory in ["database", "keys", "backup", "logs"] {
            std::fs::remove_dir_all(self.source.root().join(directory)).unwrap();
        }
        assert!(!self.source.database_path().exists());
        self.ledger
            .milestone("source-process-killed-and-server-files-removed")
            .unwrap();
    }

    fn restore_after_loss(&mut self) -> (TestEnvironment, BinaryServer) {
        assert!(self.server.is_none());
        assert!(!self.source.database_path().exists());
        self.ledger.begin_recovery_attempt().unwrap();
        let addresses = self.source.addresses().unwrap();
        let restored =
            TestEnvironment::restore_backup_with_binary(binary(), &self.backup, addresses).unwrap();
        assert_ne!(restored.database_path(), self.source.database_path());
        self.ledger
            .milestone("snapshot-restored-to-independent-directory")
            .unwrap();
        let server = BinaryServer::start(&restored, binary()).unwrap();
        self.ledger.milestone("replacement-probe-ready").unwrap();
        self.client = TestClient::new(&self.source, "volume-controller").unwrap();
        assert_eq!(
            self.client.pinned_host().unwrap().host_id(),
            self.host.host_id()
        );
        (restored, server)
    }

    fn recover_complete_snapshot(mut self) -> Duration {
        let (_restored, server) = self.restore_after_loss();
        self.verify(1);
        self.write("after.txt", b"post-loss durable write");
        self.verify(2);
        let elapsed = self.ledger.recovered().unwrap();
        assert!(server.graceful_shutdown().unwrap().status.success());
        elapsed
    }
}

#[test]
#[ignore = "requires freshly built FOKS_SERVER_BIN; local volume-loss experiment only"]
fn complete_snapshot_recovers_after_source_server_files_are_destroyed() {
    let mut installation = Installation::prepare("volume-complete");
    installation
        .ledger
        .fault("local-source-volume-loss-complete-snapshot")
        .unwrap();
    installation.destroy_source();
    let elapsed = installation.recover_complete_snapshot();
    eprintln!("local complete-snapshot volume-loss recovery: {elapsed:?}");
}

#[test]
#[ignore = "requires freshly built FOKS_SERVER_BIN; expected data-loss counterexample"]
fn stale_surviving_snapshot_fails_recovery_even_when_replacement_accepts_writes() {
    let mut installation = Installation::prepare("volume-stale");
    // The only surviving server copy predates this acknowledged insertion.
    installation.write("lost.txt", b"acknowledged after snapshot");
    installation.verify(2);
    installation
        .ledger
        .fault("local-source-volume-loss-stale-snapshot")
        .unwrap();
    installation.destroy_source();
    let (_restored, server) = installation.restore_after_loss();
    installation.verify(1);
    installation.write("replacement.txt", b"replacement accepts writes");
    installation.verify(2);
    assert_eq!(
        installation.ledger.recovered().unwrap_err().to_string(),
        "recovery needs all acknowledged reads and a new write"
    );
    eprintln!(
        "stale surviving snapshot: recovery correctly REJECTED (acknowledged insertion lost)"
    );
    assert!(server.graceful_shutdown().unwrap().status.success());
}

#[test]
#[ignore = "requires freshly built FOKS_SERVER_BIN; expected missing-fencing counterexample"]
fn former_owner_accepts_divergent_writes_after_a_replacement_has_served_clients() {
    let mut installation = Installation::prepare("unfenced-owner");
    // Independent pre-pinned observers retain their own durable checkpoints.
    // Reusing one soft store across divergent branches would itself trigger
    // a resurrection conflict and conceal the missing server ownership fence.
    let former_client = TestClient::new(&installation.source, "former-owner-controller").unwrap();
    let former_host = former_client.probe_and_pin().unwrap().pinned;
    assert_eq!(former_host.host_id(), installation.host.host_id());
    authenticate(
        &former_client,
        &former_host,
        &installation.account.credential,
    );
    let addresses = installation.server.as_ref().unwrap().addresses();
    // Prestage a second copy; there is no replication between the two databases.
    let replacement =
        TestEnvironment::restore_backup_with_binary(binary(), &installation.backup, addresses)
            .unwrap();
    installation
        .ledger
        .fault("local-copy-handoff-without-distributed-fence")
        .unwrap();
    installation
        .server
        .take()
        .unwrap()
        .force_shutdown()
        .unwrap();
    let replacement_server = BinaryServer::start(&replacement, binary()).unwrap();
    installation.verify(1);
    installation.write("successor.txt", b"acknowledged on successor");
    installation.verify(2);
    installation
        .ledger
        .milestone("successor-acknowledged-write")
        .unwrap();
    // Same endpoint is reused sequentially. This isolates missing ownership
    // enforcement; it does NOT model simultaneous network reachability/partitions.
    replacement_server.force_shutdown().unwrap();
    let former_owner = BinaryServer::start(&installation.source, binary()).unwrap();
    installation.ledger.begin_recovery_attempt().unwrap();
    write_file(
        &former_client,
        &former_host,
        &installation.account.credential,
        installation.account.kv_projection[0].root_directory_id,
        "former-owner.txt",
        b"acknowledged on former owner",
    );
    installation
        .ledger
        .acknowledged("former-owner.txt", b"acknowledged on former owner")
        .unwrap();
    installation
        .ledger
        .milestone("former-owner-incorrectly-accepted-new-write")
        .unwrap();
    verify_all(
        &former_client,
        &former_host,
        &installation.account.credential,
        2,
        &mut installation.ledger,
    );
    former_owner.force_shutdown().unwrap();
    let replacement_server = BinaryServer::start(&replacement, binary()).unwrap();
    installation.ledger.begin_recovery_attempt().unwrap();
    // Both copies have accepted writes, but neither contains the union.
    // Use fresh RPC results with the original client's rollback anchors.
    let authentication = authenticate(
        &installation.client,
        &installation.host,
        &installation.account.credential,
    );
    let tree = installation
        .client
        .foks()
        .sync_user_kv(
            &installation.host,
            &installation.account.credential,
            &authentication.verified,
            &authentication.puks,
            installation.client.soft_state_path(),
        )
        .unwrap();
    assert_eq!(tree[0].entries.len(), 2);
    assert!(tree[0]
        .entries
        .iter()
        .any(|entry| entry.name == b"successor.txt"));
    assert!(!tree[0]
        .entries
        .iter()
        .any(|entry| entry.name == b"former-owner.txt"));
    // Observations from different owners must not be unioned into a false pass.
    // A successful recovery requires the selected owner to contain *every* ack.
    installation
        .ledger
        .milestone("successor-missing-former-owner-acknowledgment")
        .unwrap();
    installation.write(
        "successor-final.txt",
        b"new write on selected recovery target",
    );
    installation.verify(3);
    assert_eq!(
        installation.ledger.recovered().unwrap_err().to_string(),
        "recovery needs all acknowledged reads and a new write"
    );
    eprintln!("former-owner experiment: fencing ABSENT; two copies acknowledged divergent writes");
    assert!(replacement_server
        .graceful_shutdown()
        .unwrap()
        .status
        .success());
}

#[test]
#[ignore = "requires freshly built FOKS_SERVER_BIN; small local simultaneous-restore experiment"]
fn simultaneous_complete_snapshot_restores_finish_within_the_shared_budget() {
    let count = std::env::var("FOKS_RECOVERY_TENANTS")
        .map(|value| {
            value
                .parse::<usize>()
                .expect("invalid FOKS_RECOVERY_TENANTS")
        })
        .unwrap_or(4);
    assert!(
        (1..=32).contains(&count),
        "FOKS_RECOVERY_TENANTS must be 1..=32 for this local fixture"
    );
    let mut installations = (0..count)
        .map(|index| Installation::prepare(&format!("fleet-{count}-{index}")))
        .collect::<Vec<_>>();
    let started = Instant::now();
    for installation in &mut installations {
        installation
            .ledger
            .fault("local-simultaneous-volume-loss-complete-snapshots")
            .unwrap();
    }
    // Stop and erase every source before any replacement is allowed to start.
    // No barrier in worker threads: a failed shutdown must not hang its peers.
    for installation in &mut installations {
        installation.destroy_source();
    }
    let times = std::thread::scope(|scope| {
        let handles = installations
            .into_iter()
            .map(|installation| scope.spawn(move || installation.recover_complete_snapshot()))
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    let elapsed = started.elapsed();
    assert!(
        elapsed <= budget(),
        "all {count} local restores took {elapsed:?}, budget {:?}",
        budget()
    );
    eprintln!("{count} local complete-snapshot restores: total {elapsed:?}; per-tenant {times:?}");
}

fn copy_fixture_tree(source: &Path, destination: &Path) {
    std::fs::create_dir(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        assert!(
            !kind.is_symlink(),
            "fixture backup must contain only regular files/directories"
        );
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_fixture_tree(&entry.path(), &target);
        } else {
            assert!(kind.is_file());
            std::fs::copy(entry.path(), &target).unwrap();
            std::fs::File::open(target).unwrap().sync_all().unwrap();
        }
    }
    std::fs::File::open(destination)
        .unwrap()
        .sync_all()
        .unwrap();
}
