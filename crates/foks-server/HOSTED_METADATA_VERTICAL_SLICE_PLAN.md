# Persistent authenticated metadata slice

Draft for review after the authority-kernel qualification. Markdown stays unstaged.
This is the next implementation plan, not a claim that the current fixture is a
production adapter. Do not start implementation before direct and independent
review. Preserve existing FOKS network messages, signed bytes and historical
verification. Local adapter formats and deployment features may be versioned.

## What the previous gate actually establishes

The shared kernel and its authority envelope exercise transactional comparison of
declared installation/activation/owner/revision, bounded intent allocation, exact
outcomes and retirement. The qualification Worker seeds each test vector and
compares full snapshots offline. Its principal table is synthetic. Neither those
snapshots nor that principal table belongs on the production request path.

The next slice must make an actual authenticated FOKS mutation and read through a
persistent cell, then recover its outcome after losing the native frontend. No
disposable SQLite reconstruction, arbitrary SQL over HTTP, local authority fallback,
or per-request full-state digest is permitted. A preprovisioned personal-account
slice is a checkpoint toward M1, not complete KV, team or production coverage.

## A. Real current authority and bounded query bundles

Extract current credential and personal-KV authorization into shared transaction
functions using the pinned statement catalog. Keep certificate parsing and TLS
proof of possession in the native frontend. Supply the exact authenticated peer
certificate and its parsed serial/public-key binding; the cell verifies the exact
stored certificate, validity interval, active owning device and device/subkey
identity, and derives its stored role. Do not trust a role or user ID asserted by
the frontend as the database authorization result.

`ReadDatabase::credential_for_certificate` currently searches exact certificate
bytes without a supporting index. Use the parsed serial to seek the existing
primary key, then compare exact bytes and validity/device predicates. Differential
tests must establish the same binding/error behavior, including wrong serial,
wrong exact bytes, inactive owner, expired/not-yet-valid certificate, delegated
subkeys and bot credentials. DER serial canonicalization must match the existing
issued-certificate representation exactly; certificate times and native creation
times remain microseconds. Do not apply the rename-only interactive owner rule to KV.

Use targeted current-role/key-generation queries rather than loading every device
and historical shared key via `user_authority`. Match the current native role and
key-generation semantics exactly, including visibility. Any aggregate over retained
history needs a measured index/projection or an explicit bound; a smaller result
alone is not evidence of bounded work. Apply directory read/write role and current
generation checks inside the same transaction as the associated query or mutation.

Reject SSO-policy tenants in this early disabled-feature profile after checking
authoritative policy; do not bypass `sso_require_access`. Team/federated token routes
remain explicitly unsupported until their current token, membership and key checks
are ported. Unsupported credentials/features must receive the existing compatible
status rather than silently using local database authorization.

Query bundles carry the exact authority tuple and domain revision. Read necessary
current roots/dirents/nodes/list pages and negative predicates in bounded SQL batches.
Final observation proof rechecks current credential authority and revision before
sync/publication. Mutation execution repeats current authorization atomically with
effects, including revocation/demotion between preparation and execution. Database
or host-side resource failures must abort, not become a durable domain rejection.

Gate A: native/shared-kernel differential fixtures built from real provisioned Rust
and Go credentials; raw requests for revoked/demoted/wrong-role/wrong-user callers;
same-revision read bundles; bounded rows/bytes and SQL VM work across history. Review
the extraction and error precedence twice before tests and its separate commit.

## B. Persistent cell transport and installation bootstrap

Add versioned private command endpoints over the existing kernel. Public FOKS
clients cannot choose SQL IDs, caller authority, owner promotion or activation.
Authenticate distinct controller and frontend capabilities before parsing sensitive
commands; production activation also needs rotation/revocation. For the persistent slice, use a distinct credential per frontend owner. The
authority stores its digest bound to installation, owner instance and epoch;
controller promotion atomically installs the successor binding. Check that binding
in the same transaction as the command. A shared frontend secret plus a caller-
asserted successor fence is insufficient. Keep the controller credential outside
frontend hosts. Qualification may use locally provisioned per-owner secrets, with
production credential rotation/revocation and controller recovery separately gated.

Bootstrap once under controller exclusion, before activation: import a complete
canonical domain database, validate schema and foreign keys, compare the actual
HostID and storage binding, and bind the actual kernel/catalog artifacts. Fixture
hash labels are insufficient. Verify the exact immutable encrypted key bundle is
retrievable from durable storage under its wrapping authority before activating its
digest/generation. Never regenerate a missing signing/decryption key on recovery.
Require a quiesced source with exclusive writer/storage ownership and no
unqualified outstanding publishers. The first small-KV slice rejects any external
chunk references or unresolved physical attempts; external routes stay unsupported.
An existing native filesystem storage binding is rejected even with zero current
references unless a separately reviewed import explicitly retains it as inert audit
metadata and prohibits use. A path/device/inode binding cannot simply be copied
into a cell and considered adopted. Later external-file adoption needs a separately
reviewed ownership/storage migration. The bootstrap path is a bounded maintenance
job; ordinary commands cannot invoke it.

Request processing only opens persistent state and executes targeted kernel commands.
COMMIT plus `storage.sync()` gates successful mutation replies and authoritative
read/error observations. Lost transport/sync replies retain an unknown outcome;
the successor resolves the original intent under current authority and never creates
a replacement identity merely because the result was lost. Registration itself stores bounded canonical command and caller context, charging
the actual encoded bytes plus worst-case outcome space atomically before effects.
Registration must be the first possible durable effect, and sync precedes execution.
After closing the old epoch, a successor enumerates the bounded retained ledger,
resolves committed outcomes and settles provably unexecuted SQL-only registrations.
This makes a separate frontend-local pre-send journal unnecessary: an uncommitted
registration has no obligation; a committed one is discoverable after a lost reply.
Unresolved records cannot retire. Preserve the unchanged public RPC ambiguity and
reconciliation semantics; never infer cross-connection identity from equal hashes
or promise exactly-once behavior without an existing public operation identity.

Within an ongoing RPC, use the original native deadline throughout HTTP connection,
bounded request/response streaming, admission, sync and outcome lookup. A later
authenticated recovery observation has its own bounded deadline and cannot extend
the original execution expiry. Cross-host expiry needs an authenticated
clock-error-qualified conversion; local same-host proof is not a deployment clock
qualification. The celld synchronous clock limitation remains: implement or qualify
an interruption/isolation mechanism before claiming hard execution bounds. Timeout
must leave actual work and its ownership/accounting supervised. The current fixture
only advances its clock floor on committed observations: an expired call followed
by clock rollback is not yet irreversibly expired. Before the persistent service
releases a definitive expiry response, record host-derived qualified time in a
restricted fenced floor transaction and sync it, after rolling back failed domain
work. Failure to prove that floor is an unknown outcome/closed connection. Never
advance it from caller-supplied expiry. SQL-only expired intents can additionally
be settled terminally. Test expired rejection, wall-clock rollback and retry, plus
promotion racing the floor update; old credentials cannot advance authority. A
response deadline after domain commit remains ambiguous, not proof of nonexecution.

Gate B: persistent sequential commands without seeding between requests; real local
celld bucket durability; lost register/execute/promotion replies; paused old owner;
frontend process replacement; exact successor outcomes and bounded retirement;
malformed/oversized private requests; artifact/key mismatch refusal. No production
enablement until bootstrap, caller-context and authority checks pass.

## C. Native TCP/mTLS integration with no fallback

Add a distinct typed hosted-operation boundary at the listener's blocking dispatch
point, before local `Principal::authenticate` or SSO access. It receives the routed
call, authenticated certificate, selected installation and original deadline.
It must not accept a closure over a `Database` or reuse `RpcProof::Projection`.

A hosted listener starts without an ordinary writer, read database or secondary
services that can mutate local state. Native TLS/key bootstrap is pinned to the
validated activation manifest; all dynamic metadata and public protocol responses
come from the hosted boundary. Every route, including poll/special paths, either
uses that boundary or fails closed. Mechanically classify the complete generated
route catalog; additions must fail compilation or a coverage gate until classified.
Management HTTP, startup and background writers have their own explicit inventory.

For the first real client path include its actual discovery/Merkle/user-chain/key
dependencies, small-node creation, PutDirents and authenticated Get/List. Reuse the
existing Rust encoding and cryptographic verification. Public framing, response
memory permits and native deadline transmission remain under the existing listener.
Preserve ambiguity if a mutation may have committed when output allocation or a
connection fails. A data-bearing error also requires an authoritative observation.

Gate C: unmodified Rust and pinned Go clients over real TCP/mTLS, ordinary idempotent
retry behavior, raw-RPC stale authorization, zero local writer/read fallback,
no whole-state reconstruction in request traces, unsupported-route refusal, and
successful acknowledged write/read through a replacement frontend. Only then claim
one complete authenticated vertical slice; continue the explicit M1/M2 coverage list.

## D. Recovery and capacity qualification after the real path exists

Combine frontend and celld owner/volume loss behind one stable frontdoor. The
replacement reads durable configuration/key artifacts, resolves any retained intent,
and completes a fresh authenticated write/read within15 seconds. Count success, not
only a retryable response. Check every acknowledged value for zero loss. Measure
simultaneous recoveries and idle/active tenant density using the declared workload
and growing history/receipt/debt dimensions. Preserve failed runs and exact versions,
load and resource limits. Local emulation does not certify AWS AZ/network/KMS timing.

The five-minute disaster rewind allowance still requires a separate restore-under-
retained-authority test. Raw rollback of epoch/allocation authority is unsupported.
Remote object reclamation remains report-only until outstanding physical PUT
termination is proved. None of these gates is waived by a fast semantic fixture.

## Direct plan review

The key changes from the fixture are real stored credential/role authorization,
actual installation/artifact/key validation, persistent request execution and durable
caller context. The native seam must precede all local authorization and cover async
special routes. Exact-certificate lookup and all-key history scans were identified
as additional cost dependencies. An immutable bootstrap cache is usable only when
its actual generation is activated and fenced; it cannot become a second authority.
Independent recovery-agent review identified and resolved owner-specific capability
binding, discoverable durable registration context and irreversible observed-expiry
handling. Storage-agent review required quiesced bootstrap and explicit rejection
of external objects until their separate adoption protocol is qualified. Those
contracts are incorporated above. Final independent rereview passed; each A/B/C
component still needs its own author and independent code reviews before testing
and committing. This document does not enable a production deployment.
