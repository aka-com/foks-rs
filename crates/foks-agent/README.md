# foks-agent

Bounded resident process for standalone FOKS clients. It listens on a private
Unix-domain socket below an explicit `--state-dir`, defaulting to
`foks-rs.sock`, and holds `.foks-rs.lock` for its process lifetime. It runs
blocking client work through a bounded worker semaphore, limits active
connections and requests per connection, and applies read/write/dispatch
deadlines. Dispatch cancellation is propagated into the synchronous FOKS
network client; a timed-out operation
closes its local connection because its durable outcome can be ambiguous, and
any surviving blocking work retains its worker permit until it exits.

KV reads require the catalog version and large-file chunks echo the store,
path, version, and offset they answer. Creating or updating account-store entries
requires explicit create or exact-version preconditions; existing native
read/write roles are preserved on edit. A large upload holds mutation single-flight while a
four-frame in-memory channel feeds the existing FOKS writer, never a plaintext
staging file. Socket loss before the final local commit frame prevents the
namespace mutation; post-commit uncertainty keeps the mutation permit with the
worker and asks the caller to refresh before another write.

The process can bind before a new state root has credentials. In that restricted
bootstrap mode it accepts only `AgentStatus` and `InitializeState`; every normal read or
mutation fails with `BootstrapRequired`. Successful initialization verifies access to the
new master key and atomically moves the process to `Ready`, after which scheduled work and
compatibility polling begin. The packaged desktop launches this process with detached
standard streams and does not retain a child handle, so closing the desktop window does
not stop the agent.

The resident scheduler polls durable `FoksScheduler` state and runs due user
refreshes and federation reconciliation only for profiles whose protocol
policy permits them. Federation jobs resolve their public wake-up ID against
the encrypted local-team record and nonblockingly acquire the remote profile;
public SQLite aliases or roles never authorize a cross-host operation.
Scheduler sweeps are single-flight, and a profile already in use by another
frontend or scheduler is skipped rather than tying up a worker.
The agent separately polls each current profile's stable HTTPS compatibility
lease. Fetches are single-flight, limited to four concurrent requests and 64
KiB per response, use WebPKI with bounded redirects and deadlines, then require
the profile-pinned Ed25519 signer, exact target and protocol digest, and a
strictly newer generation before an atomic registry update. Fetch failures
retain the existing short lease; expiry still revokes, while an authenticated
drift artifact revokes on the next successful poll.
The agent accepts software signup, including an optional invite and PPE
passphrase, so the Tauri desktop can complete onboarding without opening
credentials or SQLite. It also dispatches interactive software-device KEX,
passphrase set, change, and public
challenge verification, protected remote-team admission/listing, and
federation-aware due-job runs. Cross-host admission locks both profiles in
canonical order and advances both native rollback checkpoints. It generates
device/PUK secrets inside the checked
profile session and receives invite/passphrase values only over its
authenticated private Unix socket; serialized request frames and decoded
secret strings are zeroized. Owner-device provision/resume, owner backup and
recovery, device listing, and YubiKey passphrase operations delegate to the
same checked `foks-client-app` flows used by the CLI. A newly enrolled backup
phrase is returned once over the private socket for offline storage; recovery
phrase inputs are redacted and zeroized like other request secrets.
Device listing returns an optional authenticated display name. Roster demotion
and removal select a local-user row by its authenticated party ID and, when the
roster contains admitted teams, verify those scoped recipients through the
existing cross-profile federation path before rotating any PTK.

The same checked account boundary now owns Basic named-team chat polling and
durable sends, OIDC signup and reauthentication, local/remote invitation
workflows, durable username changes, resident bot credentials, and hosted
web-administration handoff. Long chat polls use separate bounded admission;
browser and bearer secrets remain inside native/application handlers rather
than general agent state.

Team creation dispatches the existing native named/ad-hoc workflows and stores
their recovery material in the account vault before the remote mutation. A
separate resume operation continues that durable record after interruption;
the local protocol does not invent team rename or closure semantics.

```text
foks-agent --state-dir /private/client
```

macOS and Linux are supported. Windows is currently unsupported because
authenticated named-pipe transport and ACL validation are not yet implemented.
The resident-agent and desktop workflows therefore have no supported Windows
path. Lower-level direct-client use requires an embedder-supplied credential and
rollback boundary.

On Unix, the socket authenticates the operating-system user ID. It is not a
sandbox boundary between mutually untrusted processes running under the same
UID; deployments that include such processes must isolate the agent under a
separate OS identity. A stronger same-UID client capability would require a
different launch and credential-brokering design.
