# Gaps between the mocks and what FOKS supports

The mocks were drawn from the product model, not from a capability audit. This file
records the audit: every affordance, field and claim in the design set checked against
`foks-agent-proto`, `foks-agent`, `foks-client-app` and `foks-client`, with each gap
either corrected in the mock, gated behind a stage in `ENGINEERING-PLAN.md` §4, or
removed.

**Design requirement:** UI affordances must align with protocol capabilities.
Unsupported wire operations must not be simulated with synthetic client state.

## Classification

| | Meaning |
| --- | --- |
| **A** | Supported today — an agent `Operation` exists and the daemon implements it |
| **B** | Exists below, not exposed — `foks-client-app`/`foks-client` can do it, no agent op |
| **C** | Proposed in an earlier design, but no FOKS code implements it |
| **D** | Not supported anywhere — the mock invented it |
| **E** | Wrong — the mock contradicts actual FOKS semantics |

---

## Round 1 findings

### 1. The catalog cannot list team items — **B**

`ListKv { profile, alias }` lists an **account's** KV store. There is no operation for a
team's KV. `SyncTeam` returns `TeamSyncReport { alias, team_id_hex, team_chain_sequence,
directories, entries }` — counts, not rows. Below the agent,
`foks-client::sync_team_kv` does produce the projection, and
`foks-client-app::team::sync_team` builds it (`team.rs:157`) and then discards everything
but the counts.

Consequence in v1: the unified catalog can show personal-store items only.

**V2 decision:** add paged, metadata-only `ListTeamKv` with the same `KvPage` response as
the paged `ListKv`. The target mock now lists account and team stores together. This gap is
closed in the v2 contract and remains implementation work rather than a UI exception.

### 2. There is no activity or audit log — **D**

No `ListActivity`, `History`, `Events` or equivalent operation exists anywhere in
`foks-agent-proto::Operation`. Nothing in `foks-client-app` reads one. The Activity screen
in all three mocks — day-grouped entries with per-entry chain sequences, Merkle epochs and
"signed by" attribution — has no backing at all.

**Mock change:** the Activity screen and every placeholder explaining its absence are cut.
No replacement history is implied by this plan.

### 3. Roles are Member / Admin / Owner, not Owner / Manager / Member / Reader — **E**

`foks_proto::role::RoleType` is `None | Member | Admin | Owner`;
`foks_agent_proto::TeamRole` is `Member | Admin | Owner`. The strings "Manager" and
"Reader" appear **nowhere** in the FOKS crates; the initial mock imported roles FOKS
does not have.

Worse, a FOKS `Member` carries a **`visibility: i16`** (`Role::member(visibility)`;
`AddTeamMember { …, role, visibility }`). It is an ordered Member-role band used by
disclosure, PTK distribution, and KV authorization; the mocks dropped it entirely.

**Mock change:** roles become Member / Admin / Owner everywhere; `visibility` is shown on
member rows and in the add/demote controls. The Navigator's invented Reader example is
re-cut — see finding 4.

### 4. "Reader" read-only refusal has no FOKS analogue — **E**

The read-only demonstrations are built on a Reader role that does not exist. The
underlying *idea* — a surface that cannot decrypt — is real in FOKS, but it arrives
differently: `Error::CapabilityDenied(Capability)` from the canary/lease gate, or a
`RollbackDetected` pause, or simply an unsynced store. It is not a role.

**Mock change:** the invented Reader example is cut. A real capability-denied profile may
still render its actual refusal, but it is not presented as a role or a read-only item.

### 5. Per-item sync states are borrowed fiction — **D**

`KvEntrySummary` is `{ path, node_type, version, size }` — nothing else. The initial
"Not on this device", "Waiting to upload", "Conflict" and "Can't decrypt" states were
not derived from any FOKS response.

**Mock change:** per-item state pills are removed. Items show path, kind, version, size.
The only honest per-item exception today is "this store has not synced", which is a
property of the store, not the item.

### 6. There is no way to list scheduled jobs — **B**

`RunDueJobs { profile }` is the only job operation, and it *runs* due work, returning
`JobRunReport { runs: Vec<JobRun> }`. There is no passive read.
`foks-client-app::runtime` has no public list/inspect either — `FoksScheduler::register`
and `unregister` are internal.

Consequence: a Sync screen showing schedules, next-run times, paused jobs and failure
counts cannot be built. You can only run jobs and read what happened.

**Mock change:** scheduled work is absent from the desktop. The agent may continue its
background maintenance, but the UI neither lists jobs nor provides a run/report panel.

### 7. `TeamSummary` carries none of the store-page facts — **B/D**

`TeamSummary { alias, account_alias, team_id_hex, kind, name, active }`. No chain
sequence, no member count, no created date, no last-sync time.

Consequence: the Stores table's `ITEMS` / `MEMBERS` / `LAST SYNCED` columns and the facts
strip's `CREATED` / `HOLDS` / `LAST SYNCED` cannot be filled from a list call. Chain
sequence and counts require a `SyncTeam` per team (a network round trip each); created
date exists nowhere.

**Mock change:** the Stores table drops `ITEMS`, `MEMBERS` and `LAST SYNCED`, keeping
`STORE` / `LIVES ON` / `KIND` / `STATE`. The team detail uses only the six fields in
`TeamSummary`; it does not add a client-memory "last synced" fact.

### 8. `ProbeReport` carries none of the server-page trust facts, and probing is a network write — **B/D**

`ProbeReport { lookup_name, canonical_name, host_id_hex, host_chain_sequence,
merkle_epoch }`. No canary state, no TLS CA fingerprint, no service list, no pinned-at
timestamp. And `Probe` *performs* a probe-and-pin (`session.probe_and_pin()`), so it is
not a passive read of local state — there is no agent operation that returns the
already-pinned host without touching the network.

Canary state is the exception and is genuinely available: `ListProfiles` returns
`Vec<Profile>`, whose `protocol: ProtocolPolicy` carries `last_artifact` with
`expires_at` (`foks-compat-artifact`), so lease freshness is computable locally.

**Mock change:** the Servers facts strip keeps `PROBE HOST`, `HOST ID`, `CHAIN`,
`MERKLE EPOCH` (all marked "as of last probe") and `CANARY`. TLS CA, services and
pinned-at are removed. The page gains an explicit "Probe now" rather than implying the
facts are live.

### 9. `ListAccounts` returns bare alias strings — **B**

The account alias registry yields `Vec<String>`, nothing more (`account.rs`). Username, state,
chain sequence, device count and last-sync all require `SyncAccount` (network) per
account, or do not exist.

**Mock change:** ListAccounts views show aliases only. SyncAccount results are not folded
back into the account-list row as if ListAccounts had returned them.

### 10. The agent reports no health telemetry — **D**

`Operation::Ping` returns exactly `{"ready": true}`
(`foks-agent/src/main.rs:578`). No version, worker occupancy, queue depth or uptime is
exposed anywhere.

**Mock change:** worker counts, queue depth, uptime, and version claims are removed. Local
protocol v2 exposes only the lifecycle state required for `Bootstrap` / `Ready`; this is a
state-machine contract, not health telemetry.

---

## Consolidated round 1 table

| # | Gap | Class | Disposition |
| --- | --- | --- | --- |
| 1 | Team KV not listable | B | Closed in v2 contract with paged `ListTeamKv`; implementation pending |
| 2 | No activity/audit log | D | Screen cut |
| 3 | Roles Member/Admin/Owner + visibility | E | Corrected everywhere |
| 4 | Reader role does not exist | E | Re-cut as capability-denied |
| 5 | Per-item sync states | D | Removed |
| 6 | No job listing | B | Scheduled-work UI cut |
| 7 | TeamSummary lacks store facts | B/D | Columns and facts-strip fields removed |
| 8 | ProbeReport lacks trust facts; probe is a network op | B/D | Fields removed; probe made explicit |
| 9 | ListAccounts returns aliases only | B | Account-list views keep aliases only |
| 10 | No agent health telemetry | D | Metrics cut; v2 exposes lifecycle state only |

Ten findings in round 1, so a second round follows. Rounds continue until one yields three
or fewer.

---

## Round 2 findings

Round 1 checked whether operations exist. Round 2 checks what the operations that *do*
exist actually return, and finds the payloads are materially different from what the
mocks render.

### 11. `ListKv` is a network operation, not a local catalog read — **E**

`list_kv` → `authenticated_tree` (`foks-client-app/src/kv.rs:202`) calls
`pinned_host()`, then `authenticate_and_pin(&host, &loaded.credential)` — a login round
trip — then `sync_user_kv`. There is no agent operation that lists a store from local
cache.

Consequence: the catalog is a live sync per store per view, requiring every server
reachable and every account unlocked. Offline, Items is empty — not "showing what it last
knew". The mocks imply a local catalog you browse.

**Mock change:** Items gains an explicit per-store load state and an honest offline
state: "This store could not be reached, so nothing from it is listed." No "last known"
or cached-items language anywhere.

### 12. `TeamMemberSummary` is a different model from the roster shown — **E**

```rust
pub struct TeamMemberSummary {
    pub username: Option<String>,        // may be absent
    pub party_id_hex: String,
    pub scoped_host_id_hex: Option<String>,
    pub party_kind: String,              // "user" | "named-team" | "ad-hoc-team" | "unknown"
    pub source_role: TeamMemberRole,     // TWO roles, not one
    pub destination_role: TeamMemberRole,
    pub generation: u64,
    pub locally_manageable: bool,
}
```

Four things the mocks get wrong:

- **A member may be a team, not a person** (`party_kind`, from
  `foks_proto::ENTITY_NAMED_TEAM` / `ENTITY_AD_HOC_TEAM`). That is how federated admission
  works. A roster of people avatars misrepresents the model.
- **`username` is optional.** A party may have no resolvable username; the handle that
  always exists is `party_id_hex`.
- **There are two roles.** `source_role` and `destination_role` differ for federated
  members. A single role dropdown cannot express this.
- **`locally_manageable` describes target eligibility only** — it is literally whether
  the party is a local user. The verifier separately requires the caller to hold enough
  current authority, and the summary does not expose that conclusion to the desktop.

There is no email and no device count anywhere in the model.

**Mock change:** the roster becomes a table of `party_id_hex` (mono, always present) with
username when known, a `party_kind` tag, source and destination roles as separate columns,
and generation. A false `locally_manageable` suppresses controls; a true value only makes
the edit attemptable and the server/verifier may still refuse it. Avatars and emails are
removed; "Members" is renamed to reflect that parties are not all people.

### 13. `TeamMemberRole::Member` carries its visibility — **E**

`enum TeamMemberRole { Member { visibility: i16 }, Admin, Owner }`. Visibility is part of
the Member variant, not a parallel field. A role control must offer a visibility value
when and only when Member is chosen.

**Mock change:** role selectors show a visibility input that appears only for Member.

### 14. `FederatedMembershipSummary` has no refresh time or staleness — **D**

```rust
pub struct FederatedMembershipSummary {
    pub local_team_alias, remote_profile, remote_team_alias: String,
    pub remote_host_id_hex, remote_team_id_hex: String,
    pub destination: FederationDestinationRole,
    pub operation_id_hex: Option<String>,
    pub active: bool,
}
```

No last-refresh timestamp, no stale flag. `active: bool` is the only state.

**Mock change:** "Shared with" shows remote team, remote host id, destination role and
active/inactive only. The "refreshed 23 h ago" and stale pill are removed.

### 15. `ListYubiAccounts` merges completed and interrupted enrollments — **E**

`yubi_aliases()` (`foks-client-app/src/yubi.rs`) collects keys under **both**
`yubi-account.` and `pending-yubi.`, then sorts and **dedups**. The returned
`Vec<String>` therefore cannot tell you which aliases are complete and which are
half-enrolled — the distinction is deliberately flattened. Card serial, slots, model and
firmware are not in it either; `ListYubiCards` enumerates *connected* cards, which is a
different set, and `YubiPinStatus` touches the card.

Consequence: the "interrupted enrollment" state that drives a queue item and a device
badge **is not passively derivable**. It is discoverable only by attempting a resume.

**Mock change:** the device list shows aliases only, with card details appearing solely
for cards currently connected (from `ListYubiCards`). The half-enrolled notification is
removed from the derived queue; "Resume enrollment" becomes an action a person can always
attempt on an alias, not a state the app claims to have detected.

---

## Consolidated round 2 table

| # | Gap | Class | Disposition |
| --- | --- | --- | --- |
| 11 | `ListKv` is a network sync, not a local read | E | Per-store load and offline states added |
| 12 | `TeamMemberSummary` model differs on four axes | E | Roster rebuilt on the real model |
| 13 | Visibility belongs to the Member variant | E | Role control reshaped |
| 14 | Federation summary has no refresh/staleness | D | Fields removed |
| 15 | Yubi alias list flattens pending vs complete | E | Derived state removed; resume made an attemptable action |

Five findings in round 2, so a third round follows.

## Round 3 findings

Rounds 1 and 2 checked operations and payloads. Round 3 checks preconditions and error
signalling — what the client can actually *know* about why something failed, and what it
can do before it is set up.

### 16. Only three of the five error codes are semantically distinguishable — **E**

Every dispatch failure, whatever its cause, becomes
`ErrorCode::OperationFailed` with `bounded_error(error.to_string())`
(`foks-agent/src/main.rs:564`). There is no mapping from `foks_client_app::Error`
variants to codes. So `RollbackDetected`, `CapabilityDenied`,
`CheckpointResetRequired`, a wrong passphrase and an unreachable host are all one code
and one prose string.

Only `DeadlineExceeded`, `Busy` and `VersionMismatch` are machine-distinguishable, and
they are raised by the agent's own supervision rather than by the operation.

Without structured error variants from the agent, the UI cannot branch on specific
failure causes (such as rollback or capability denial) and must handle errors generically,
with the exception of supervisor deadlines.

**Mock change:** the rollback and capability-denied states stay in the mocks but are
explicitly marked as requiring §5 group E. Team listing is separately supplied by the v2
`ListTeamKv` contract.
All other errors display the agent's error message directly without client-side prose parsing.

### 17. `Probe` presupposes an initialised credential store — **C/E**

`Operation::Probe` opens `ClientCredentials::open(state_dir)` and runs inside
`with_checked_session`, which takes the rollback-checkpoint lock
(`foks-agent/src/main.rs:582`). It cannot run on a fresh state root, and the agent itself
will not start without an unlockable master key.

Consequence: the "Add server" wizard cannot be a new install's first action. First-run
order is forced and its first step is CLI-only: `foks-rs --state-dir <D> init`, then
`profile add <name> <target>`, then `profile probe <name>`.

**Mock change:** the Add-server wizard is reachable only once a store exists. Get started
states the real order and the real commands, and says plainly that the first step has no
in-app equivalent yet.

### 18. No sync timestamp is exposed anywhere — **D**

`SyncReport { username, user_chain_sequence, directories, entries }` and
`TeamSyncReport { alias, team_id_hex, team_chain_sequence, directories, entries }` carry
no time, and no operation returns when a store was last synced.

Consequence: the client can only report syncs **it performed in this session**. On launch,
"last synced" is unknown for everything.

**Mock change:** every "synced N ago" becomes "not synced this session" until a sync runs,
then shows the in-session time. No relative times are shown for work the app did not do.

---

## Consolidated round 3 table

| # | Gap | Class | Disposition |
| --- | --- | --- | --- |
| 16 | Only 3 of 5 error codes distinguishable | E | Failure-kind branching gated to §5 group E |
| 17 | Probe presupposes an initialised store | C/E | Wizard gated; Get started states the real order |
| 18 | No sync timestamp exposed | D | "not synced this session" until the app syncs |

Three findings in round 3. The audit did **not** end here — a deeper sweep run in parallel
returned findings that reframe the whole set. See round 4.

---

## Round 4 findings — Domain model corrections

Review of the native FOKS domain model identified several structural discrepancies
between mock assumptions and protocol capabilities.

### 19. The generic container model was not a FOKS model — **E, and it invalidates a premise**

The initial plan assumed a generic container with its own kind, identifier, local form,
display-name lifecycle, four-role roster, and per-container trust boundary. None of those
objects or roles exists in the FOKS crates; neither the proposed generic store kind nor
the proposed server-level content role has an implementation there.

The plan has been corrected to use the native hierarchy: profile → account → personal
store, plus teams with their own stores and party rosters. A unified Items catalog remains
a useful client view, but it is an aggregation over those authenticated FOKS objects, not a
new object in the protocol or persistence model.

### 20. The lease-lapse copy is inverted, and it is a safety claim — **E**

The mocks say a lapsed compatibility lease means the client "keeps reading what it already
has and stops making changes". The opposite is true: `permits_at`
(`foks-client-app/src/registry.rs:36`) allows only `Capability::Probe` through, and
`list_kv` enforces `Capability::Kv` immediately, disallowing read operations once a lease expires.
User copy must accurately reflect that reading is disabled when a lease lapses.

### 21. The rollback/fork copy is inverted the same way — **E**

"Items already on this Mac keep working from its own copy" is false. The checkpoint gate
runs in `with_checked_session` **before any operation dispatches**
(`checkpoint.rs:166`), so a rollback-flagged profile refuses everything, not just syncing.
The scope is the **profile**, never one store. And the remedy is not "wait until the
histories line up" — it is `reset_hard_state` (`checkpoint.rs:474`), which is destructive
and which the error string itself names.

### 22. The security-key enrollment story is fiction in four ways — **E**

- **`prepare()` generates both keys atomically** (`foks-yubi/src/hardware.rs:74`, `piv::generate`
  at `:111` and `:119`); the pending record is written only after it returns
  (`yubi.rs:485`). There is no checkpoint between the signing key and the second key, so
  "wrote the signing key but stopped before the post-quantum key" cannot happen as drawn.
- Worse, a card genuinely interrupted mid-`prepare` is **not resumable**:
  `ensure_slot_empty` (`hardware.rs:106`) refuses the retry. It needs a PIV reset. The
  flagship resume story is the one case the code cannot do.
- **Both slots are `AlgorithmId::EccP256`** (`hardware.rs:111`, `:119`). The "PQ slot"
  holds a P-256 key. The ML-KEM-768 label, the 41.4-second timing and the trace lines are
  invented.
- **Enrolling onto an in-use card fails.** `prepare` authenticates with
  `MgmKey::default()`; the source comment is explicit: *"Provisioning intentionally
  requires a factory/default management key. Existing managed cards must be reset or
  enrolled through a future import flow."*

The PUK is client-supplied rather than generated by the device (`YubiRetryConfiguration.puk`).
Displaying a generated unlock code does not reflect device behavior. Additionally, all
resume operations require the card PIN.

### 23. Store identity must follow native party identity — **E**

`TeamSummary.team_id_hex` is a 33-byte `EntityId` whose first byte is the entity-type tag,
so an abbreviated team ID cannot stand in for an independent store identifier. A personal
KV store has **no separate id at all** — its party is the account uid. The client therefore
keys account stores by profile and account alias, and team stores by profile, team alias,
and authenticated team ID.

### 24. `ListKv` cannot serve a catalog at its current scale — **E**

`MAXIMUM_KV_DIRENTS = 100_000` (`foks-proto/src/kv.rs:13`) against a **1 MiB** frame cap,
over a **15-second** deadline, while `sync_user_kv` eagerly downloads **every chunk of
every large file** (`foks-client/src/kv/sync.rs:564-612`) and fails the whole call if any
file is incomplete. A metadata-only, paged `ListKv` is a prerequisite, not an
optimisation. This also means finding 5's per-item states were doubly wrong: sync is
all-or-nothing, so after a successful call everything is present.

### 25. `DeadlineExceeded` closes that call and leaves completion ambiguous — **E (architecture)**

`main.rs:538` closes the connection after a deadline. The cancellation design in
`ENGINEERING-PLAN.md` §3.4 assumed dropping a `Task` was sufficient. `AgentClient` opens a
fresh connection for every call, so the next call already reconnects; the real requirement
is to discard the timed-out continuation and resolve a mutation by refresh or resume rather
than blind retry.

### 26. Several smaller inversions — **E**

- **`CreateAccount` already works** (`main.rs:600`). Get started claims signup is CLI-only
  — wrong in the app's favour. Only keystore init and profile-add are CLI-only.
- **Removing a member is an atomic rekey** (`foks-client/src/team/rotation/mod.rs:518`),
  not something that "takes effect at the next key rotation".
- **`remove_kv` unlinks with a version guard**; prior versions are not readable afterward.
- **The agent rejects Admin/Owner for federation roles** (`main.rs:1317`): federated teams
  hold member roles only.
- **Sync intervals** are 15 min (user) and 17 min (team), not 5; the lease poll is a tokio
  interval in the agent process (300 s), not a scheduled job at all.
- **`ProbeOutcome.acceptance`** (`Inserted | Advanced | Unchanged`) exists and is dropped
  by `ProbeReport` — surfacing it is a two-line change and is the only thing that would
  make "the host key still matches" a fact.
- **`JobRun.deferred`** is an action-required state (hardware needed), not an error, and
  no mock renders it.

---

## Consolidated round 4 table

| # | Gap | Class | Disposition |
| --- | --- | --- | --- |
| 19 | Generic container premise is not native FOKS | E | Plan recast around account and team stores |
| 20 | Lease lapse stops reads first | E | Copy inverted back; safety-critical |
| 21 | Rollback blocks the whole profile | E | Copy and scope corrected; remedy is destructive |
| 22 | Enrollment resume / ML-KEM / in-use card | E | Flow rebuilt on real PIV behaviour |
| 23 | Store identity follows account/team party identity | E | Independent store IDs removed |
| 24 | `ListKv` cannot scale to a catalog | E | Paged metadata `ListKv` becomes a prerequisite |
| 25 | Deadline closes one call and completion is ambiguous | E | Fresh connection next call; refresh/resume mutations |
| 26 | Six smaller inversions | E | Corrected individually |

Eight findings in round 4, so the audit continues past the stopping rule. **Twenty-six
findings total.** The corrections are large enough that the mocks are being rebuilt around
them rather than patched.

---

## Where this leaves the design set

Twenty-six findings across four rounds. The mocks remain the target picture, but each
screen now shows only what the stack can serve, with the rest visibly gated. The
significant losses, all deliberate:

- **Activity is gone.** No audit log exists.
- **The target catalog covers account and team stores** through v2 paged metadata calls.
  It remains live-only and empty when its sources cannot be reached.
- **Rosters are parties, not people** — a member may be a team, usernames are optional,
  and there are two roles.
- **Scheduled work has no desktop surface.** It remains agent-owned background behavior.
- **Almost nothing is timestamped**, because nothing persists a time the client can read.
- **V1 failure kinds are opaque.** Local protocol v2 must land structured errors before
  the app branches on rollback, capability, or checkpoint-reset conditions.
- **A lapsed lease or a rollback flag stops everything on that server, reads included** —
  the opposite of what the mocks first said.
- **The product model is FOKS-native.** FOKS has profiles → accounts → personal stores,
  plus teams with their own stores and an Owner/Admin/Member party roster. The catalog
  aggregates those stores without inventing another authenticated object.

The resulting architecture retains the unified catalog, store details, attention queue,
and resumable operations. Implementing structured error reporting (group E) is prioritized
to enable accurate error feedback across screens.
