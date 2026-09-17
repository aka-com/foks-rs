use super::*;
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::{InProcessServer, TestEnvironment};
use std::cell::Cell;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Boundary {
    IntentStored,
    RemoteVerified,
    RootCreated,
}

thread_local! {
    static INTERRUPT: Cell<Option<Boundary>> = const { Cell::new(None) };
}

pub(super) fn interrupt_at(boundary: Boundary) -> Result<()> {
    INTERRUPT.with(|point| {
        if point.get() == Some(boundary) {
            point.set(None);
            Err(Error::InvalidAccount("test interrupted team creation"))
        } else {
            Ok(())
        }
    })
}

struct Fixture {
    _environment: TestEnvironment,
    _server: InProcessServer,
    credentials: ClientCredentials,
    registry: ProfileRegistry,
}

impl Fixture {
    fn start() -> Self {
        let environment = TestEnvironment::new().unwrap();
        let server = environment.start_server().unwrap();
        let state = environment.client_path("creation", "state").unwrap();
        let root = environment.client_path("creation", "root.der").unwrap();
        environment.write_probe_root(&root).unwrap();
        let credentials =
            ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        registry
            .add(Profile {
                name: "local".into(),
                label: None,
                probe: format!(
                    "localhost:{}",
                    environment.addresses().unwrap().probe.port()
                ),
                protocol: ProtocolPolicy::V019,
                trust: TrustRoot::CertificateDer { path: root },
            })
            .unwrap();
        let fixture = Self {
            _environment: environment,
            _server: server,
            credentials,
            registry,
        };
        fixture
            .run(|session, vault, master| {
                session.probe_and_pin()?;
                session.create_account(
                    "owner",
                    "creationowner",
                    "laptop",
                    "",
                    "",
                    None,
                    vault,
                    master,
                )?;
                Ok(())
            })
            .unwrap();
        fixture
    }

    fn run<T>(
        &self,
        op: impl FnOnce(&CheckedProfileSession<'_>, &mut AccountVault<'_>, &[u8; 32]) -> Result<T>,
    ) -> Result<T> {
        let profile = ProfileSession::open(&self.registry, "local")?;
        self.credentials.with_checked_session(&profile, |session| {
            let master = self.credentials.master_key()?;
            let mut store = EncryptedFileSecretStore::open(
                &session.paths.credential_store,
                derive_vault_key(&master),
            )?;
            op(session, &mut AccountVault::new(&mut store), &master)
        })
    }
}

#[test]
fn creation_reopens_each_durable_boundary_without_changing_identity_or_root() {
    let fixture = Fixture::start();
    for (index, boundary) in [
        Boundary::IntentStored,
        Boundary::RemoteVerified,
        Boundary::RootCreated,
    ]
    .into_iter()
    .enumerate()
    {
        for named in [true, false] {
            let alias = format!("creation{}{}", index, if named { "named" } else { "adhoc" });
            INTERRUPT.with(|point| point.set(Some(boundary)));
            let result = fixture.run(|s, v, k| {
                if named {
                    s.create_named_team("owner", &alias, &alias, v, k)
                } else {
                    s.create_adhoc_team("owner", &alias, v, k)
                }
            });
            assert!(matches!(
                result,
                Err(Error::InvalidAccount("test interrupted team creation"))
            ));
            let (original_id, original_secrets) = fixture
                .run(|s, v, _| {
                    let team = v.team(&alias)?;
                    assert!(!team.active);
                    let summaries = s.list_teams(v)?;
                    let summary = summaries.iter().find(|team| team.alias == alias).unwrap();
                    assert_eq!(
                        summary.creation_phase.as_deref(),
                        Some(if boundary == Boundary::IntentStored {
                            "preparing"
                        } else {
                            "remote-verified"
                        })
                    );
                    Ok((
                        team.team_id.clone(),
                        [team.member_min, team.member, team.admin, team.owner],
                    ))
                })
                .unwrap();
            let report = fixture
                .run(|s, v, k| s.resume_team_creation(&alias, v, k))
                .unwrap();
            assert_eq!(report.team_id_hex, hex(&original_id));
            assert_eq!(report.team_chain_sequence, 1);
            assert_eq!(report.directories, 1);
            fixture
                .run(|s, v, k| {
                    let team = v.team(&alias)?;
                    assert!(team.active);
                    assert_eq!(
                        [team.member_min, team.member, team.admin, team.owner],
                        original_secrets
                    );
                    assert_eq!(
                        team.creation.as_ref().unwrap().phase,
                        StoredCreationPhase::Complete
                    );
                    assert!(s.resume_team_creation(&alias, v, k).is_err());
                    Ok(())
                })
                .unwrap();
        }
    }
}

#[test]
fn journaled_creation_wins_over_preparing_intent_after_lost_return_value() {
    let fixture = Fixture::start();
    fixture
        .run(|s, v, _| {
            let account = v.account("owner")?;
            let mut team = StoredTeam::random_named("lostreply", "owner", "lostreply")?;
            s.persist_creation_intent(&mut team, &account, v)?;
            // The RPC commits and journals its verified result, but the application
            // never receives that return value or advances its protected intent.
            s.client.create_single_owner_named_team(
                &s.pinned_host()?,
                &account.credential,
                "lostreply",
                &team.named_secrets()?,
            )?;
            assert_eq!(
                v.team("lostreply")?.creation.as_ref().unwrap().phase,
                StoredCreationPhase::Preparing
            );
            assert_eq!(
                s.list_teams(v)?[0].creation_phase.as_deref(),
                Some("remote-verified")
            );
            let discovered = s.discover_teams("owner", v)?;
            assert!(
                !discovered.teams[0].active,
                "discovery must not bypass root setup"
            );
            assert!(!v.team("lostreply")?.active);
            Ok(())
        })
        .unwrap();
    fixture
        .run(|s, v, k| {
            let before = v.team("lostreply")?.team_id.clone();
            let report = s.resume_team_creation("lostreply", v, k)?;
            assert_eq!(report.team_id_hex, hex(&before));
            assert_eq!(report.team_chain_sequence, 1);
            Ok(())
        })
        .unwrap();
}

#[test]
fn legacy_missing_journal_never_prepares_a_new_creation() {
    let fixture = Fixture::start();
    fixture
        .run(|s, v, k| {
            let team = StoredTeam::random_named("legacy", "owner", "legacy")?;
            let id = team.named_secrets()?.operation_id()?;
            v.put_team(&team)?;
            assert_eq!(
                s.list_teams(v)?[0].creation_phase.as_deref(),
                Some("legacy-unknown")
            );
            assert!(s.resume_team_creation("legacy", v, k).is_err());
            assert!(HardStateStore::open(&s.paths.hard_database)?
                .team_mutation(&id)?
                .is_none());
            assert_eq!(v.team("legacy")?.team_id, team.team_id);
            assert!(!v.team("legacy")?.active);
            Ok(())
        })
        .unwrap();
}

#[test]
fn legacy_reconciliation_authenticates_existing_creation_and_original_secrets() {
    let fixture = Fixture::start();
    for named in [true, false] {
        let alias = if named { "legacynamed" } else { "legacyadhoc" };
        let original_id = fixture
            .run(|s, v, _| {
                let account = v.account("owner")?;
                let team = if named {
                    StoredTeam::random_named(alias, "owner", alias)?
                } else {
                    StoredTeam::random_adhoc(alias, "owner")?
                };
                v.put_team(&team)?;
                let host = s.pinned_host()?;
                let (table, operation_id) = if named {
                    let created = s.client.create_single_owner_named_team(
                        &host,
                        &account.credential,
                        alias,
                        &team.named_secrets()?,
                    )?;
                    ("team_mutation_operations", created.operation_id)
                } else {
                    let created = s.client.create_single_owner_adhoc_team(
                        &host,
                        &account.credential,
                        &team.adhoc_secrets()?,
                    )?;
                    ("adhoc_team_operations", created.operation_id)
                };
                // Construct a historical fixture before the checked-session boundary
                // records it: remote creation exists, but neither local journal nor
                // new intent format exists. Production code never removes journals.
                let db = rusqlite::Connection::open(&s.paths.hard_database).unwrap();
                db.execute(
                    &format!("DELETE FROM {table} WHERE operation_id = ?1"),
                    [operation_id.as_slice()],
                )
                .unwrap();
                assert_eq!(
                    s.list_teams(v)?
                        .iter()
                        .find(|team| team.alias == alias)
                        .unwrap()
                        .creation_phase
                        .as_deref(),
                    Some("legacy-unknown")
                );
                Ok(team.team_id.clone())
            })
            .unwrap();
        let recovered = fixture
            .run(|s, v, k| s.resume_team_creation(alias, v, k))
            .unwrap();
        assert_eq!(recovered.team_id_hex, hex(&original_id));
        assert_eq!(recovered.team_chain_sequence, 1);
        assert_eq!(recovered.directories, 1);
    }
}

#[test]
fn abandoning_forgets_an_unfinishable_creation_and_frees_its_alias() {
    let fixture = Fixture::start();
    // A creation resuming cannot finish: its intent names an actor this
    // device no longer is, which is the shape the invalid-name case leaves too.
    fixture
        .run(|s, v, k| {
            let account = v.account("owner")?;
            let mut team = StoredTeam::random_named("stuckteam", "owner", "stuckteam")?;
            s.persist_creation_intent(&mut team, &account, v)?;
            team.creation.as_mut().unwrap().actor_id[1] ^= 1;
            v.put_team(&team)?;
            assert!(s.resume_team_creation("stuckteam", v, k).is_err());
            Ok(())
        })
        .unwrap();
    let report = fixture
        .run(|s, v, _| s.abandon_team_creation("stuckteam", v))
        .unwrap();
    assert_eq!(report.alias, "stuckteam");
    assert_eq!(report.phase.as_deref(), Some("preparing"));
    fixture
        .run(|s, v, _| {
            assert!(!v.contains_team("stuckteam")?);
            assert!(!s.list_teams(v)?.iter().any(|t| t.alias == "stuckteam"));
            // Gone means gone: there is nothing left to forget or resume.
            assert!(s.abandon_team_creation("stuckteam", v).is_err());
            Ok(())
        })
        .unwrap();
    // The alias is free again, and a team created under it completes.
    fixture
        .run(|s, v, k| s.create_named_team("owner", "stuckteam", "stuckteam", v, k))
        .unwrap();
    fixture
        .run(|s, v, _| {
            assert!(v.team("stuckteam")?.active);
            // A team that finished creating is not removed this way.
            assert!(matches!(
                s.abandon_team_creation("stuckteam", v),
                Err(Error::InvalidAccount(
                    "team has no incomplete local creation"
                ))
            ));
            Ok(())
        })
        .unwrap();
}

#[test]
fn preparing_intent_rejects_changed_actor_before_submission() {
    let fixture = Fixture::start();
    fixture
        .run(|s, v, k| {
            let account = v.account("owner")?;
            let mut team = StoredTeam::random_named("changedactor", "owner", "changedactor")?;
            s.persist_creation_intent(&mut team, &account, v)?;
            team.creation.as_mut().unwrap().actor_id[1] ^= 1;
            v.put_team(&team)?;
            assert!(matches!(
                s.resume_team_creation("changedactor", v, k),
                Err(Error::InvalidAccount("team creation actor or host changed"))
            ));
            assert!(HardStateStore::open(&s.paths.hard_database)?
                .team_mutation(&team.named_secrets()?.operation_id()?)?
                .is_none());
            Ok(())
        })
        .unwrap();
}
