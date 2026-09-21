use super::*;
use foks_keystore::EncryptedFileSecretStore;
use foks_server_testkit::{InProcessServer, TestEnvironment};

struct Fixture {
    _environment: TestEnvironment,
    _server: InProcessServer,
    credentials: ClientCredentials,
    registry: ProfileRegistry,
}

impl Fixture {
    /// Two profiles of one server: the team's owner, and the account that
    /// discovers the team it was added to, each with its own vault, exactly
    /// as two desktop profiles hold them.
    fn start() -> Self {
        let environment = TestEnvironment::new().unwrap();
        let server = environment.start_server().unwrap();
        let state = environment.client_path("discovery", "state").unwrap();
        let root = environment.client_path("discovery", "root.der").unwrap();
        environment.write_probe_root(&root).unwrap();
        let credentials =
            ClientCredentials::initialize(&state, CredentialBackend::PrivateFile).unwrap();
        let mut registry = ProfileRegistry::open(&state).unwrap();
        for name in ["owner", "guest"] {
            registry
                .add(Profile {
                    name: name.into(),
                    label: None,
                    probe: format!(
                        "localhost:{}",
                        environment.addresses().unwrap().probe.port()
                    ),
                    protocol: ProtocolPolicy::V019,
                    trust: TrustRoot::CertificateDer { path: root.clone() },
                })
                .unwrap();
        }
        let fixture = Self {
            _environment: environment,
            _server: server,
            credentials,
            registry,
        };
        for (profile, alias, username) in [
            ("owner", "owner", "discoveryowner"),
            ("guest", "joiner", "discoveryjoiner"),
        ] {
            fixture
                .run(profile, |session, vault, master| {
                    session.probe_and_pin()?;
                    session
                        .create_account(alias, username, "laptop", "", "", None, vault, master)?;
                    Ok(())
                })
                .unwrap();
        }
        fixture
            .run("owner", |session, vault, master| {
                session.create_named_team("owner", "project", "project", vault, master)?;
                session.add_local_team_member(
                    "project",
                    "discoveryjoiner",
                    TeamMemberRole::Member { visibility: 0 },
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
        profile: &str,
        op: impl FnOnce(&CheckedProfileSession<'_>, &mut AccountVault<'_>, &[u8; 32]) -> Result<T>,
    ) -> Result<T> {
        let profile = ProfileSession::open(&self.registry, profile)?;
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
fn discovery_reports_only_the_bindings_it_wrote() {
    let fixture = Fixture::start();
    let first = fixture
        .run("guest", |session, vault, _| {
            session.discover_teams("joiner", vault)
        })
        .unwrap();
    assert_eq!(first.teams.len(), 1);
    assert_eq!(first.bound, vec![first.teams[0].alias.clone()]);
    // The second run rebinds the record it already wrote. Nothing local
    // changed, so it names nothing and a caller need not re-read the catalog.
    let again = fixture
        .run("guest", |session, vault, _| {
            session.discover_teams("joiner", vault)
        })
        .unwrap();
    assert_eq!(
        again.teams.iter().map(|t| &t.alias).collect::<Vec<_>>(),
        first.teams.iter().map(|t| &t.alias).collect::<Vec<_>>()
    );
    assert!(again.bound.is_empty());
}

#[test]
fn discovery_reuses_the_outcome_a_direct_load_would_have_produced() {
    let fixture = Fixture::start();
    fixture
        .run("guest", |session, vault, _| {
            let host = session.pinned_host()?;
            let account = vault.account("joiner")?;
            let user = session
                .client
                .authenticate_and_pin(&host, &account.credential)?;
            let graph = session.client.discover_local_team_graph(
                &host,
                &account.credential,
                &user.verified,
                &user.puks,
            )?;
            assert_eq!(graph.load_paths.len(), graph.teams.len());
            let node = graph
                .teams
                .iter()
                .find(|team| {
                    team.verified.members().iter().any(|member| {
                        member.party == *user.verified.uid() && member.scoped_host.is_none()
                    })
                })
                .expect("the joiner belongs to the team directly");
            // The graph opened this node with the joiner's own credential, so
            // the binding may rest on it.
            assert_eq!(
                graph.load_path(node.verified.team()),
                Some(foks_client::TeamGraphLoadPath::Direct)
            );
            let fresh = session.client.load_and_pin_team(
                &host,
                &account.credential,
                &user.verified,
                &user.puks,
                node.verified.team(),
            )?;
            assert_eq!(fresh.verified.team(), node.verified.team());
            assert_eq!(fresh.verified.host(), node.verified.host());
            let reused = StoredTeam::discovered("joiner", node)?;
            let loaded = StoredTeam::discovered("joiner", &fresh)?;
            let identity = |team: &StoredTeam| {
                (
                    team.team_id.clone(),
                    team.kind,
                    team.name.clone(),
                    team.account_alias.clone(),
                    team.origin,
                    team.active,
                )
            };
            assert_eq!(identity(&reused), identity(&loaded));
            Ok(())
        })
        .unwrap();
}
