# Phase A: shared current personal authority

Draft 2026-10-15. Parent contract: `HOSTED_METADATA_VERTICAL_SLICE_PLAN.md`.
Plan only; no production edits or tests authorized by this document. Markdown
stays unstaged. Preserve FOKS messages, signatures, historical verifiers and Go
compatibility. This phase does not enable a hosted listener.

## Findings and scope

`auth/principal.rs` checks exact issued certificate bytes, validity and Ed25519
credential binding. `certificates.rs::credential_for_certificate` currently scans
certificate bytes, while `services/kv.rs::resolve_authority` materializes all
current devices and every historical shared key through `read::user_authority`.
The writer guard compares the entire role-to-maximum-generation map. A role is
not just Owner/Admin/Member: member visibility is an i16. Neither the 64-member
cap nor a small typical account bounds the number of historical key roles.

A1 extracts reusable scalar authentication and personal authorization primitives,
point reads, and authorization of the existing PutDirents primitive. A2 completes
bounded read bundles and the public-page-size path before phase C can claim a
complete authenticated slice. Native existing KV/Principal APIs remain the
comparison baseline during extraction; no broad replacement of their behavior
or dependency on the synthetic `metadata_principals` table is introduced.

Use the existing fenced tenant `domain_revision` for prepared-state validation.
Every supported auth/key/domain mutation and controller change must advance it;
registration, observation and retirement do not. Phase B's mutation inventory and
exclusion of legacy writers are prerequisites. No second per-user revision or
schema57 is proposed for A1. Unrelated concurrent mutations may invalidate a
hosted preparation; they must never allow stale authorization. Do not claim the
legacy native history scans have been removed merely because the new path is bounded.

## A1 types and trusted boundaries

Proposed module `foks-metadata-core::personal` contains no TLS implementation and
no owner-promotion logic. It accepts an already-open transaction through `Sql`.
Types use existing Role/RoleAndGeneration and exact bounded byte representations:

- `PeerCertificate`: exact DER (maximum4096 bytes for this qualified profile),
  canonical unsigned serial magnitude (1..20 bytes), parsed Ed25519 key32, and
  parsed expiry in microseconds. Constructor/validation checks widths and exact
  canonical integer form. A zero serial has a defined one-byte zero form; do not
  accidentally narrow the existing parser's accepted representation.
- `CurrentCredential`: UID33, actual TLS credential in the existing EntityId shape, canonical owner device33/34,
  stored Role, credential kind, and certificate expiry. It is a query result, not
  caller-supplied authorization. Backup credentials authenticate but cannot enter
  ordinary KV; bot credentials remain permitted by the existing KV policy.
- `PersonalContext`: authenticated credential plus the wrapper's installation,
  activation/epoch/owner and domain revision. No arbitrary party/role supplied by
  a public request can replace the queried UID/role.
- `KeyRequirement`: exact RoleAndGeneration plus Read or Write. Read requires a
  positive generation no greater than the current maximum and role <= caller;
  Write requires equality to the current maximum and role <= caller.
- `PersonalError`: distinguish unauthenticated/permission, missing KV object,
  unsupported hosted profile, malformed stored data and resource/storage failure.
  The native service adapter maps these to existing RpcStatus/error behavior.
  Infrastructure failure is never a durable domain rejection.

The native frontend validates TLS proof of possession and parses the certificate
once. The owner-authenticated envelope carries those exact parsed inputs. The
cell is not proving TLS possession from a public certificate; only the currently
fenced trusted frontend may present that context. The cell still compares exact
stored DER, current certificate interval, current owning device/subkey and public
key/entity binding in its transaction. The public key extracted by the native
parser is compared with the entity bytes; a freely supplied UID or role is ignored.
Certificate validity uses microseconds, distinct from authority-envelope milliseconds.

Proposed APIs (names may change, contracts may not):

```
authenticate_peer(sql, peer, now_micros) -> CurrentCredential
require_personal_profile(sql, credential) -> ()
current_key_generation(sql, uid, role) -> Option<u64>
require_key(sql, credential, requirement) -> ()
read_personal_point(sql, context, point) -> exact bounded result
prepare_personal_put(sql, context, parsed_put) -> bounded prepared requirements
execute_personal_put(sql, context, prepared_put, now_micros) -> result
```

Key requirements are internal query helpers, not a caller-controlled declaration
that all necessary checks were performed. The operation resolver derives every
required directory/key/role check from its parsed operation and stored exact rows.
Preparation is not permission to execute later. Execute repeats current certificate,
credential, parent directory/key/role and revision checks within the same transaction
as effects. Read completion requires the wrapper's current-authority/revision proof
before sync and publication. A1 functions themselves neither commit nor publish.
The authority wrapper owns installation/fence/epoch/deadline/retirement and invokes
these functions via a real caller resolver; do not duplicate its state machine.
Register and Execute both resolve current credential state; Observe/Resolve enforce
current read/outcome access. Durable intent context preserves the original exact
authenticated credential while a successor separately proves its recovery authority.
Principal equality is never equality of an untrusted supplied UID.
The existing synthetic resolver stays only in its explicit qualification fixture;
callers cannot select synthetic versus real resolution.
Coordinate that seam with the authority-kernel owner before editing its files.

## Pinned scalar statements

Append stable statement IDs after the frozen authority catalog; never renumber
existing IDs or use a broad numeric `>= AuthorityState` dispatch for new groups.
Place the new SQL in `personal/catalog.rs` with explicit dispatch membership.
Native and Wasm execute the identical catalog strings and typed bindings.

1. `IssuedCredentialBySerial`: primary-key seek on issued_certificates.serial;
   join devices by the stored device_id AND uid. Compare exact DER as a bound
   predicate, require not_before <= now < not_after and active=1, and require
   c.credential_id equals that same row's device_id OR current subkey_id. Return
   only fixed identity/role/times. Never return the stored DER blob to JS first.
2. Serial handling: native issuance stores20 random bytes, but rcgen/yasna removes
   leading zeros when encoding DER (confirmed in installed source). Canonicalize
   the parsed positive magnitude, then try its bounded possible zero-padded storage
   representations through length20. At most20 primary-key probes, no scan and no
   schema migration. Include high-bit DER sign padding and zero in fixtures. Reject
   multiple matching exact-certificate bindings as corrupt rather than choosing
   an arbitrary identity. Check every candidate before returning a unique match.
3. `CurrentCredentialByOwner`: exact canonical owner device PK plus UID, active,
   and actual credential equality/subkey condition. Avoid the unindexed-by-UID
   OR scan and ensure a rotated delegated subkey cannot borrow another row's role.
4. `PersonalAnchor`: reproduce the native `user_chain_heads h` join to
   `tree_locations t ON t.uid=h.uid AND t.seqno=h.seqno`, singleton
   `merkle_root_heads rh`, then `merkle_roots r ON r.epoch=rh.epoch`, filtered by
   h.uid. Do not silently add a users join unless its equivalence is established
   from the foreign-key/bootstrap invariants. No complete chain, tree or device list.
5. `PersonalSsoPolicy`: indexed host policy existence. Any configured/persisted
   policy makes this hosted profile unsupported, including disabled/blocked policy;
   do not interpret absence of currently usable provider configuration as bypass.
6. `CurrentRoleGeneration`: `WHERE uid=? AND role_type=? AND visibility=? ORDER BY
   generation DESC LIMIT1` using the existing shared_keys composite primary key.
   Return just the integer; no aggregate over all roles or old HEPK payloads.
   Decode the catalog canonical signed integer with checked nonnegative conversion;
   malformed/negative/out-of-range stored values remain infrastructure/corruption
   failures mapped to the native retry status, not a missing role or PermissionDenied.
   Preserve positive-generation checks for reads and exact-current equality for
   writes rather than introducing one shared positivity/error rule.
7. Point rows: root by UID; active directory by UID/id and exact head-version join;
   small/symlink node by UID/id; dirent head by its full key; existing constant-time
   usage counters. Return length before CASE-bounded exact payload. Large-file
   nodes and external references are unsupported in this profile.

No new indexes are expected for these point predicates; EXPLAIN and VM-step tests
are mandatory, including the serial candidate loop's worst case. If a predicate
scans retained rows, stop and review an index/migration plan instead of raising a
work budget. The historical name lookup retains its exact native semantics and
index/tie behavior; it is not substituted with the live-name projection.

## Invariant qualification and native parity

Targeted key reads intentionally do not rediscover malformed unrelated key roles
on every request. Bootstrap must fully validate every stored shared-key role and
visibility, generation and identity/credential relationship before activation;
subsequent typed writes preserve them. Invalid role0/noncanonical admin visibility
or out-of-i16 member visibility is rejected, matching the old full-map failure.
A raw SQL controller or unchecked legacy native writer invalidates this assumption
and cannot coexist with the hosted profile. No online full-history audit is added.

Keep the native old map computation as a test oracle, not a production fallback.
Compare each requested key decision against that map across all role boundaries,
missing roles, older/current/future generations and device/subkey changes. The
hosted domain-revision guard supplies the old full-map preparation protection;
it must reject an unrelated key rotation too. Validate this through real wrapper
mutations, not by setting a test-only revision token independently of mutation.

Preserve route error order explicitly: TLS binding before route authorization;
ordinary-credential check; personal auth shape; preconditions; directory/key/role;
mutation metadata; transaction recheck. Resolve differences against current service
behavior before committing. Native code sometimes maps insufficient dirent write
role to BadArguments, whereas directory access maps PermissionDenied. Do not flatten
these into one error just because the shared kernel uses a simpler internal type.

## A2 read bundle boundary (required before phase C)

Root/GetDir/GetNode are scalar. Get may try64 historical name alternatives and
follow a node; preserve omitted ctime in Get and included ctime in List. List
supports4096 entries and optional small-file bodies, including omission of an
unreadable optional body while retaining its dirent. Public cursor, duplicate-name,
ctime ties and final-page behavior remain unchanged.

The M0 ABI permits one row/SQL call,2048 calls and64KiB JSON output. It cannot
honestly support a default4096-entry public List. Do not lower the public page cap
or silently truncate results to fit the fixture. A2 must review a bounded page ABI
and public-response assembly separately: internal pages use the schema56 indexes
with private tie-break continuations, carry one fixed authority/revision, and have
row/byte limits before SQL-to-JS copies. Native response assembly retains existing
request memory permits and withholds the entire public frame until final proof.
An intervening mutation rejects the whole bundle; no snapshot is held across HTTP.
Review binary payload handling rather than multiplying large ciphertext into JSON
number arrays. Qualify maximum4096/load-small behavior under the actual Wasm and
native memory caps. Historical-name work must also be measured across retained
versions before claiming bounded complete Get.

## Differential fixtures and acceptance

Build fixtures through real TestEnvironment account creation, device provisioning,
certificate issuance and encrypted KV operations; preserve exact certificate and
signed bytes. Add a test-support comparison seam for the native service's current
authority decisions. Do not populate a synthetic principal row and call that real
authentication. Export the same initialized domain schema and inputs for native,
Node/actual-celld Wasm tests. Include an unmodified pinned Go-client provisioned
credential and its read/write flow, not merely Rust-generated scalar rows.

Test certificate serial lengths1..20, all leading-zero variants, sign padding,
wrong serial/DER/key, validity boundaries, unsupported algorithms, duplicate binding,
active/revoked owner, rotated subkey and different-owner subkey, bot owner/member,
interactive roles with visibility extremes, backup/recovery-only credentials,
wrong user, missing authority anchor, malformed unrelated stored roles, SSO policy
states, and corrupted/oversized stored payloads. Actual TLS fixtures establish
possession; database differential fixtures only test current binding decisions.

Pause between preparation and execution, then revoke/demote/rotate subkey or PUK,
including an unrelated role's key; require rejection with unchanged KV state and
no successful outcome. Require one transaction for authorization and mutation;
injected SQL errors/traps after effects roll back. Preserve exact existing retry
identity/normalization behavior. Error/status parity is tested, not inferred.

Measure production catalog SQL with1/100/10,000 issued certificates, devices and
historical generations; fixed requested roles have constant VM shape. Exercise
2048-call/4096-byte/input bounds and allocation-before-copy rejection. Do not claim
constant disk latency from VM-step counts. Native+Wasm equal replies/state, maximum
query work, pinned Go flows, existing native conformance and all-target Clippy must
pass after author and independent code reviews. Keep failed-run evidence.

## Ownership and sequencing

1. Author/direct plan review, then root and independent plan review.
2. Freeze/commit the current authority fixture first; create a QMUX worktree from
   that reviewed baseline. No shared main catalog edits while it is frozen.
3. Storage owner: new personal module/catalog, real fixture/differential tests,
   scalar native adapter seam; coordinate serverDB read-only test access explicitly.
   Authority owner: real caller resolver hook and wrapper transaction integration,
   preserving synthetic fixtures separately. Root: frontend TLS parsed-context seam
   and native service/status comparison wiring. Agree exact interfaces before edits.
4. A1 author review, independent review, then focused native/Wasm/Go tests and a
   separate commit. No hosted listener enabled and no claims of complete List yet.
5. A2 gets its own concrete ABI/type/memory plan and two code reviews before testing.
   B/C cannot skip A2 or discovery/Merkle/user-chain/key-bootstrap dependencies.

Author review: verified native serial storage versus DER normalization, same-row
subkey checks, role/visibility and whole-map semantics, SSO persistence, native
error precedence, scalar-host limits and4096-page incompatibility. No tests run.
Independent review PASS: recovery reviewed same-row binding, bounded serial probes,
role/generation semantics, transaction rechecks and A2 compatibility limits. Root
direct review PASS after the exact native PersonalAnchor joins and checked generation
conversion/status semantics above were made explicit. Actual issued Rust/Go DER
fixtures must still validate zero/sign/leading-zero normalization; the4096-byte DER
limit is an explicit qualified-profile boundary, not a new public protocol limit.
Implementation waits for the frozen authority commit and coordinated interfaces.


## Agreed parser interface proposal (before implementation)

Storage owns `foks-metadata-core/src/personal/mod.rs` and the opaque type below.
Root owns a new native helper in `server/src/auth/principal.rs` and its parser tests.
Do not replace `Principal::authenticate` yet; the current native path remains the
baseline. Root may add the ordinary server dependency on metadata-core when the
new module is ready; no native SQL feature is needed just to construct these values.

```rust
// foks_metadata_core::personal
pub const MAX_PEER_CERTIFICATE_BYTES: usize = 4096;
#[derive(Clone, Eq, PartialEq)]
pub struct PeerCertificate { /* private fields */ }
impl PeerCertificate {
    pub fn from_frontend_parts(
        exact_der: Vec<u8>,
        canonical_serial_magnitude: Vec<u8>,
        ed25519_public_key: [u8; 32],
        parsed_expiry_micros: u64,
    ) -> crate::Result<Self>;
    pub fn exact_der(&self) -> &[u8];
    pub fn serial_magnitude(&self) -> &[u8];
    pub fn public_key(&self) -> &[u8; 32];
    pub fn expires_at_micros(&self) -> u64;
}
// server::auth::principal; callable only by native routing/qualification code
pub(crate) fn parse_hosted_peer_certificate(
    certificate: &rustls_pki_types::CertificateDer<'_>,
) -> crate::Result<foks_metadata_core::personal::PeerCertificate>;
```

The constructor validates only structural bounds/canonical magnitude/time integer
range; its name explicitly marks the trusted-frontend boundary. It does not claim
to prove TLS possession or parse DER. No UID/role argument or setter exists. The
native helper checks the DER size before allocating, parses its Ed25519 SPKI with
zero unused bits and32 key bytes, derives canonical unsigned serial magnitude,
and computes checked expiry seconds times1,000,000. Keep one zero byte for zero;
remove DER sign padding and storage leading zeros. Any serial sign/form limitation
must be covered by real issuance/native-parity tests and documented as a hosted
profile restriction. Never silently truncate a serial or expiry.

Private transport encoding will preserve opaque raw JSON command bytes as today,
or use canonical decimal text for time if any intermediary parses JSON. No unsafe
JS number round trip. Deserialization must run the same structural validation and
reject unknown fields; it cannot reconstruct a CurrentCredential from supplied UID
or role. CurrentCredential has private fields and is returned only by SQL resolution;
its getters expose UID, actual credential, canonical owner, kind, stored Role and
expiry. The resolver remains a separate transaction-local function from the parser.

## Explicit historical rebinding restriction

Same-row owner/subkey binding is not unconditionally equivalent to the current
native path. `credential_for_certificate` joins the certificate's recorded owner
but does not check its current subkey; later `active_credential_owner` can find the
credential on another active row. `UserReplayState` retains current device state
and shared-key history, not historical subkey uniqueness; provisioning checks a
new device identity and the presence of a Yubi subkey without globally forbidding
historical reuse. The SQL UNIQUE subkey constraint only covers current rows.

Therefore qualify an explicit hosted restriction: a certificate whose recorded
owner no longer owns that credential is rejected, even if another active owner now
uses the same subkey. A fresh certificate must bind the current owner. Preserve
native behavior as the differential baseline and assert this one deliberate
fail-closed difference for a rotated-and-rebound subkey, rather than claiming an
impossible historical invariant. Do not tighten the generic historical verifier or
change signed links. Native certificate reuse/renewal across such rebinding is a
separate review candidate; do not silently modify it in this extraction.


### Frontend parser implementation review

The native helper remains behind tests or the explicit `celld-proof` qualification
feature; ordinary Principal authentication is unchanged. It parses one bounded
certificate, retains exact DER, checks Ed25519 width/unused bits, canonicalizes
positive serial magnitude without truncation, and checks expiry conversion.
This profile rejects trailing bytes and negative ASN.1 serials even though the
generic x509 parser may accept them; these are explicit hosted restrictions.
The helper requires a trusted TLS frontend and does not establish possession or
current SQL authorization by itself.

Root author review covered pre-allocation bounds, no authority from UID/role input,
serial zero/sign padding, expiry units, and preserving the native baseline.
Five parser regressions use the real bounded native certificate issuer for all
serial widths, leading zeros, sign padding and zero, plus malformed/oversized DER,
trailing data, unsupported keys, unused bits and negative expiry. Independent
review requested before running them. Actual Rust/Go account authorization
fixtures and transaction-time revocation remain separate A1 acceptance gates.

## A1 core implementation review, 2026-10-15

The author reviewed the five personal core/catalog/test files listed in
`/private/tmp/foks-personal-core-manifest.txt` before validation; final results
are recorded below.
The existing authority wrapper owns the context/fence/revision state; a separate
`PersonalContext` or competing prepare/execute state machine was not introduced.
The shared `authorize_personal_put` hook derives permissions from parsed exact
metadata and is called by the real resolver at registration and execution.
Scalar QuotaCounters returns the four namespace charging counters as an internal
reply. It does not implement the public KvUsage RPC, whose ciphertext/file counters
have different semantics. Public Usage remains an A2 counter-design gate.

Nine source-reviewed tests cover structurally bounded peer input, all serial
padding widths, current certificate/device/subkey and role changes, the explicit
historical-subkey-rebinding restriction, native full-map key decisions, scalar
read/profile/corruption handling, Put metadata authorization and exact replay,
native version-check statuses and deltas, and production SQL VM work at
1/100/10,000 rows including the whole20-probe serial candidate loop. Stored-row
fixtures do not prove TLS possession or Go certificate parsing. Those gates belong
to the separate native parser and real adapter fixtures before A1 qualification.
Malformed unrelated roles still require activation/bootstrap invariant validation;
these targeted queries neither silently repair nor audit all history online.


Root direct core review PASS after the private Usage point was renamed
`QuotaCounters`. The actual public KvUsage requires different semantic aggregates
and remains explicitly deferred to A2. Reviewed bounded serial probes/current
owner binding, role and generation decisions, exact anchor joins, stored payload
bounds, native precondition ordering/deltas and operation-derived Put checks.
Storage's author review preceded this pass. Recovery's independent integration
review and subsequent validation results are recorded below.

Recovery independently reviewed the native parser, native key-decision oracle
and optional Go public-context hook: PASS before testing. The Go hook runs only
after the existing pinned Go signup/provision/encrypted KV/rename/bot flow succeeds.
It writes an exclusive0600 fixture with public certificate DER and exact identity
bytes, never private keys. The comparison oracle delegates to existing native
authority/key predicates and returns booleans without inventing route error
precedence. No hosted listener is enabled by these seams.

## A1 real resolver and private Wasm boundary (implementation review)

The shared authority kernel now has a sealed resolver seam. Existing synthetic
fixture invocations retain their exact original binding format. A distinct
`run_personal` entry resolves UID, credential, canonical owner and current role
from the issued-certificate/device rows inside the same transaction. Public input
cannot select a synthetic principal or controller caller. The native helper is
behind `celld-proof` and does not install a listener route.

Registration/execution bind the exact validated peer context. Principal identity
for outcome lookup instead binds the queried UID, credential and canonical owner,
so an authorized renewed certificate can resolve an earlier receipt but cannot
silently execute an intent registered under different DER. Known committed results
are reconciled before rechecking mutable destination state, while current
credential/profile checks still run. Original context retrieval after frontend
loss and the complete inventory of domain-revision bumps remain phase B gates.
A direct test revision increment proves the rejection mechanism, not that inventory.

Separate `run_personal` and `read_personal` Wasm exports share the typed native
resolver. Point reads prove owner/activation/revision and read current data in one
transaction. The immutable qualification Worker selects frontend capability for
these vectors; arbitrary HTTP request bodies cannot select a different entry.
Structured outer errors cannot become nested domain rejection tokens. Stale-cache
errors carry exact Snowpack bytes as canonical hex; a precise64KiB envelope check
rejects overflow as `UnsupportedProfile` before hex expansion. This is an explicit
A1 qualification restriction, not a changed public RPC status. Complete large
precondition/error vectors still require A2's bounded binary/page ABI.

Author review covered resolver freshness, receipt ordering, renewal semantics,
rollback/error provenance, output bounds and fixture false positives. Storage's
independent review passed all13 resolver/host paths before tests. The live Rust
fixture creates an actual account and encrypted small file, then obtains a real
renewed certificate by advancing only its server test clock six days while the
original remains valid. Initial traffic establishes actual TLS possession; renewal
vectors establish current stored binding, not wall-clock TLS acceptance of a
future-dated certificate. The Go gate runs the existing pinned full public flow
and imports only its optional public certificate/identity fixture.

The full Go flow also exercises a large file, so its fixture uses the existing
filesystem chunk backend before server startup. This preserves the public flow
and all metadata while keeping encrypted payloads out of scalar vector snapshots.
The A1 qualification reads/mutates only its small-node namespace entries; untouched
external references do not qualify hosted file operations or satisfy phase B's
bootstrap/storage restrictions. Root and storage independently reviewed this
fixture correction before tests. Final validation results follow.


## A1 validation completed, 2026-10-15

All reviewed gates passed on the frozen27-file source overlay at base
`f30c70c8`, schema56. Native tests:24 core (including11 existing authority
regressions),5 parser,1 live Rust fixture, and1 explicitly invoked ignored Go
fixture. The latter runs the full pinned Go public flow before exporting its
public certificate context. Node host/durability tests:14 passed. Native core
all-target lint, server default/qualification lint, focused testkit lint and
Wasm lint passed with warnings denied; the bounded Wasm build and18 changed
Rust-file formatting checks passed.

Actual celld0.6.2 with the same Wasm/catalog passed96 exact state/reply/outcome
vectors:29 Rust personal,27 Go personal,28 prior authority,5 domain-error and
7 infrastructure/COMMIT/trap regressions. Each vector also rejected the wrong
capability. Negative-case HTTP/error outcomes are expected assertions; they are
not availability measurements. Full comparison covers rollback and durable
rejection behavior; direct fixture revocation/revision edits do not establish
complete production mutation coverage.

Evidence: `/private/tmp/foks-personal-final-qualification.json` summarizes the
five qualification files and verifies their recorded artifact hashes. Frozen
source hashes are `/private/tmp/foks-personal-a1-source-sha256.json`; isolated
source is `personal-authority-validation`. All27 isolated files match; main
source matches except its intentionally newer D3 Cargo.lock, so commit the tested
isolated lock blob without overwriting D3's working copy. Wasm SHA256 is
`1ba1269c54f5d21ac185dbb9186ac4073c4a2a7817bfa233ecc3f501914c12c6`;
shared catalog SHA256 is
`e5897111b41e408733ee993f700a495ef5334eadd7542c61848f3206e2dffb31`.
The initial compile failures remain in core-a/core-b logs: unsupported fixed-array
Serde export, an untyped test byte literal, and a native-oracle method receiver.
Each narrow correction received author and independent review before rerunning;
no behavioral failure was discarded. Final logs share the
`/private/tmp/foks-personal-` prefix.

This qualifies shared current personal authorization, exact peer/renewal binding,
selected scalar reads and PutDirents within the explicit A1 limits. Per-vector
seed/snapshot comparison is test setup, not the persistent phase B service.
Installation/artifact/key retrievability, complete revision hooks, durable original
caller context, production admission/routing, full public Usage/List/large-error
coverage, synchronous SQL deadlines and host/AZ failover remain separate gates.
No15-second or tenant-density claim follows from this semantic suite.
