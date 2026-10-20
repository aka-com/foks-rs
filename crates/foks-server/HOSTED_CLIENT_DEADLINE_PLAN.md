# Caller deadline continuation

Prepared at `fbcf6640`; Markdown stays unstaged. This follows native D1 and precedes
complete agent/private-host propagation. No FOKS network field changes.

## D2: explicit caller budget in the Rust transport

The existing one-shot `call_prepared` shares a budget across TCP/TLS, vhost selection
and its call. However, a checked-out connection's later `call` starts a fresh budget,
and a multi-RPC operation obtains a fresh budget for each connection. Preserve the
existing per-RPC behavior when no operation context is supplied; allow callers to
bind an absolute `std::time::Instant` once for their complete operation.

Add an optional operation deadline to `FoksClient`, propagated through clones,
request-control copies, isolated clients and pooled connection handles. Public setter
or derived-client API accepts an absolute instant. New operation controls clamp the
configured per-RPC timeout to that instant. `PooledConnection::set_timeout` cannot
extend an already-bound operation deadline. Pool storage retains only sockets and
sequence state; a later independent borrower receives its own context, never a
previous caller's deadline. Explicitly reject expired context before DNS, socket
checkout, TLS credential/config preparation where practical, or transmission.

One synchronous operation may bind a clone while sharing the socket pool; it must
not mutate the reusable base client's context. No automatic retry of an ambiguous
mutation. Deadline expiry does not establish cancellation of remote effects or local
blocking DNS. Existing cancellation retains its distinct error.

Review/test matrix: expired context rejects before attempting a local listener or
DNS; cloned/isolated client cannot extend the original instant; timeout updates do
not extend it; a pooled handle keeps it across repeated calls; independent later
borrower can proceed with its own budget. Use pre-expired instants and deterministic
barriers for delayed dispatch, not time-race sleeps. Existing TLS/pool/recovery tests
must pass. This transport primitive alone is not end-to-end qualification.

## Remaining D propagation

After U/UC stabilizes agent IPC36, carry the original caller budget through local
IPC framing, profile queues, blocking dispatch, every Rust RPC and final response
flush. Account for streaming registration/readiness and caller retries without
refreshing a budget. Existing poll and bulk operations need explicit classes;
ordinary requests use15s, protocol polls preserve their wire waits, whole-file jobs
retain a separately bounded completion policy. Keep execution permits until actual
work terminates even after the response deadline. Audit the current cancellation
grace separately so it cannot silently extend the public response bound.

The private native-to-cell envelope must carry authenticated UTC expiry plus a
qualified clock-error margin, anchor once to a local monotonic instant, and clamp
queue, SQL admission, object I/O, sync/outcome lookup and transmission. A timing-out
await does not prove physical PUT or SQLite termination. Provider SQLite cancellation
and indexed statement work remain required gates. Local combined recovery tests must
show durable acknowledgments survive and a new authenticated write/read succeeds
within15s; retryable failure is insufficient.

Direct plan review: keep socket-pool state independent of caller deadlines, distinguish
operation deadline from per-RPC timeout, propagate through every clone helper, do not
shorten existing protocol polls implicitly. Independent recovery-agent review passed with two additions: binding an already-bound client clamps to the earlier deadline (no clearing/extending); test a pre-expired checked-out connection rejects before frame transmission. Independent operations begin from the unbound base client.


### D3 inspection notes (implementation plan still requires review)

The agent-client already uses one local socket deadline, but the deadline starts
inside `call_platform` after encoding. Streaming captures an outer instant before
preparation, then supplies a relative remainder to another platform function which
anchors again. The agent starts its ordinary operation budget after complete frame
read/decoding. Its shared-to-exclusive retry consumes that operation budget, but
response transmission gets a fresh timeout. Reconciliation, compatibility renewal,
streaming and chat/pairing have separate branches requiring explicit coverage.
The blocking supervisor also has a one-second cancellation grace: keep actual work
and ownership supervised without making the caller wait beyond its response budget.

`ProfileSession::open_with_control` and `open_with_control_and_client` set per-RPC
relative timeouts. They must accept/retain the same absolute operation instant
before opening profile state and deriving credentials; federation copies that
context through `with_operation_controls_from`. Scheduled background jobs need
separate bounded job contexts, rather than inheriting an expired foreground context.

A reviewed local IPC extension must transmit an expiry without resetting it at the
process boundary. Same-host monotonic clock coordinates are preferable to wall time;
Rust `Instant` itself is opaque and must not be serialized. Qualify the chosen OS
clock across the supported Unix client/agent processes, validate canonical integer
encoding/overflow, clamp caller claims to configured operation-class caps, and reject
missing/unsupported context with an IPC version change. Cross-host metadata envelopes
still require their separately authenticated, clock-error-qualified expiry contract.
These inspection notes do not select a production clock API or enable that extension.

## D3 proposed execution plan (awaiting review; no implementation)

This is a separate change after U/UC IPC36 is committed. The public target is a
bounded foreground attempt, including the local queue and response transmission.
A deadline error never proves that an acknowledged or ambiguous mutation did not
commit. Durable upload/adapter handles remain the means to reconcile that outcome.
The host/AZ successful-recovery target still needs the separate combined-fault
qualification; this client change does not establish it.

### Clock coordinate and IPC37

Do not serialize Rust `Instant`, an elapsed duration that the receiver restarts, or
an unauthenticated wall-clock expiry. Introduce one small Unix local deadline
module shared by agent-client and agent, with an injectable clock for tests. The
wire representation contains an explicit clock-domain version and a canonical
unsigned decimal nanosecond expiry string (no sign, whitespace, leading zeroes,
fraction, or JSON floating-point value; checked u64 arithmetic). `Request` gains a
mandatory deadline envelope and IPC increments36→37. Every constructor must supply
one; no serde default or missing-context fallback. Raw tests and streaming start
frames use the same envelope. The agent derives the budget class from `Operation`,
not from a caller-selected claim.

Proposed supported clocks match the current Rust timers: Linux `CLOCK_MONOTONIC`,
Darwin `CLOCK_UPTIME_RAW`. Rust documents these respective implementations, while
also warning they can change; qualification therefore pins our platform API and
tests the conversion rather than relying on `Instant` layout. Linux monotonic time
omits suspend, and Linux time namespaces may offset it. Treat suspend/VM pause as
outside the foreground responsiveness claim; do not claim15s of wall-clock delivery
while the machine cannot execute. Sources: [Rust Instant documentation](https://doc.rust-lang.org/stable/std/time/struct.Instant.html),
[Linux clock_gettime](https://man7.org/linux/man-pages/man3/clock_gettime.3.html),
[Linux time namespaces](https://man7.org/linux/man-pages/man7/time_namespaces.7.html).

Reject cross-domain comparison. Linux callers and agent must identify the same
boot and time namespace (bounded boot-ID value plus `/proc/self/ns/time` identity);
Darwin needs a bounded per-boot identity and the fixed uptime-clock tag. Exact
Darwin boot-identity API and Linux namespace-access behavior are an implementation
qualification gate before coding this module. Missing/unsupported identity fails
closed with an actionable local error, never substitutes wall time. Credentials
and the private Unix socket remain the authentication boundary; these coordinates
are local control data and are never sent in FOKS network requests or persisted as
retry deadlines. A platform without this qualification is unsupported explicitly.

Clock sampling produces `{wire_expiry, local_instant}` at the caller once. Capture
the original local Instant deadline before the OS-clock read, bracket the read with
a later Instant sample, and compute remaining time from that original deadline
after sampling. Set wire_expiry to OS_sample plus remaining_after_sample, rejecting
expiry or overflow. Descheduling during sampling therefore consumes budget; never
use OS_now plus the full cap after capturing a caller instant. On receipt, capture
an Instant before reading the same OS clock and add the checked wire remainder to
that earlier instant once, so receiver conversion overhead also consumes budget;
clamp to `min(caller_expiry, receipt_time + configured_class_cap)`. Existing client
contexts can only be shortened. Check the original OS expiry again before final
publication/flush, so a future Rust clock implementation change cannot silently
make a later reanchoring extend the coordinate. Use `checked_add`/checked subtraction
and reject overflow or expired input before state admission.

### Budget classes and ownership

| Work | Attempt cap and boundary |
| --- | --- |
| Ordinary foreground RPC, including reconciliation and compatibility renewal |15s maximum, caller may choose less; starts before first encode/connect/queue and covers final IPC flush. |
| Chat long poll | Existing60s cap; clamp protocol wait to remaining time with response room. No implicit enlargement of an explicit shorter caller deadline. |
| Device pairing relay flow | Existing300s cap and45s completion reservation, all queue/relay/final mutation work consumes the same context. |
| Whole-file stream or native download job | Explicit300s job cap, ordinary per-RPC transport caps still apply. Issuance→registration→ready→source→commit uses one outer context. Download chunk requests each have min(15s, remaining job budget). |
| Scheduled maintenance/reconciliation | Fresh independently bounded job context at scheduler admission (existing job cap, with its value made explicit at each caller); never inherit a foreground client's expired context. |
| Human dialogs/unlock/PIN interaction | Separate user-interaction phase; capture network/job context when confirmed work starts. Preserve D1 unlocked-generation gates across the entire command and dialog. |

These are policy proposals requiring review, especially300s whole-file completion.
A large or slow job may end ambiguously and resume by durable handle. The plan does
not promise all1GiB files finish within300s. No ordinary request can opt itself into
a polling/bulk cap by passing an arbitrary envelope field. Remove the desktop's
blanket60s ordinary timeout (`src-tauri/src/agent.rs`) only as part of this explicit
classification; longer waits become named operation classes.

A timed-out synchronous read, DNS call, fsync or SQLite call may continue. Keep its
worker permit, profile permit, protected-state ownership and memory budgets until
the physical work returns. Response expiry immediately cancels cooperative work
and closes/stops writing the socket. Move the current one-second cancellation
supervision grace off the response critical path into an owned supervisor/reaper;
it must retain/drain the JoinHandle and zeroize/discard a late response. This does
not release a permit early or pretend the blocking operation was terminated.

### Propagation map and implementation ownership

1. Local clock/envelope in agent-proto or a small shared module used by
   agent-proto/client; prefer keeping clock syscalls outside pure JSON types.
   The existing crates already separate pure framing from Unix transport. Add
   strict parse/validation tests and a real child-process same-clock probe on both
   supported OS CI runners before adopting the representation. Bump IPC once.
2. `foks-agent-client/src/lib.rs`: capture before encoding; replace relative
   `call_platform`/`upload_platform` arguments with the same owned deadline
   context. Add a derived client binding an earlier deadline without modifying
   the reusable base. Streaming preregistration must use that derived client:
   it currently calls `call_cancellable` with a newly started budget. Source reads,
   ready response, every frame, commit and final read all check the original cap.
3. `foks-agent/src/main.rs`: parse/validate envelope immediately after bounded
   framing and before profile/worker admission. The pre-envelope frame-read idle
   timeout remains separately bounded, but caller encoding/frame transmission has
   already spent its original budget. Use `timeout_at`, not a new timeout after
   each await. Pass the context through shared→exclusive retry, semaphore wait,
   `supervise_blocking`, dispatch, ready-handshake and `write_response`. Reserve
   response time within the original budget; never add a fresh write timeout.
   Explicitly audit `ReconcileProfile`, `RefreshLease`, chat poll, pairing,
   streaming upload, initialization/status and all early error replies.
4. `connectivity.rs`, `chat_poll.rs`, `profile_work/*`, background scheduler and
   retention workers: coalesced work has an independent owned job deadline;
   each subscriber stops waiting at its own deadline. One short waiter cannot
   extend a shared job, and its expiry cannot cancel another live subscriber's
   useful work. Worker creation is bounded before allocating blocking jobs.
   Reuse existing coordinator ordering and retain ownership on cancellation.
5. `foks-client-app/src/registry.rs`: add deadline-aware ProfileSession constructors
   and preflight expiry before registry/profile preparation and credential
   acquisition. Bind D2 `FoksClient` absolute deadline before network setup.
   Related/federated profile constructors copy the same controls; base transport
   pools remain context-free. All synchronous checked-session lock waits need a
   deadline-aware acquisition or an explicit supervised noninterruptible boundary;
   returning from a timeout does not allow a background operation to continue
   unchecked into a new remote mutation. Recheck immediately after each wait and
   before source/remote side effects.
6. Direct CLI commands and desktop native wrappers: bind a single request/job
   context at their execution boundary, including multi-call inspect→write and
   upload issuance→stream. Do not reset it at helper boundaries. Local filesystem
   setup remains cooperatively checked and is not claimed preemptible. Preserve
   native publication gates and renderer epochs independently of this deadline.
   Audit all raw AgentClient callers, agent test fixtures, backend transcripts,
   MCP calls and compatibility startup/probe flows after IPC bump.

D3 should be one coherent protocol+consumer commit if these surfaces cannot be
independently compatible. A short preparatory pure deadline module commit is
possible only if it is unused and does not silently change live constructors.
No upload schema, FOKS remote wire, crypto, or hosted server changes belong here.

### Tests and review gates

Use injected clocks and deterministic barriers instead of sleeps to exercise
expiry before framing, in profile queue, after semaphore acquisition, at blocking
worker entry, after shared→exclusive retry, after first RPC, before source/ready,
between stream frames, and before final response flush. Assert no deadline refresh
across each boundary; no network/source consumption after a pre-effect expiry;
ambiguous errors after a mutation may have started; exact retained capacity until
a deliberately blocked worker is released; discarded/zeroized late plaintext.

Strict wire cases include missing envelope, unsupported clock/domain, malformed
integer, overflow, domain mismatch, expired context, excessive future expiry and
operation-class clamp. A two-process Unix-socket probe compares clocks in bounded
brackets; a Linux time-namespace mismatch fixture must reject, with a documented
skip only where namespace creation is unavailable. Tests do not infer macOS
qualification from Linux or vice versa. Wall-clock jumps must have no effect.

Add real agent tests for each special branch and client stream preregistration,
plus a delayed final socket reader that proves no fresh transmit timeout. Preserve
long-poll/KEX tests and durable upload source-once/receipt replay tests. Run native
serial tests, IPC/CLI/backend transcripts, agent/app tests, relevant D2 transport
regressions, clippy and UI separately under the existing disk/Cargo-slot discipline.
Before tests: direct author review, then independent/root review; behavioral fixes
receive narrow rereview before reruns. Only measured end-to-end results qualify
responsiveness, and the combined recovery matrix remains a separate required gate.


### D3 platform prerequisite evidence (2026-10-15)

A standalone C probe ran 100 separate exec-child samples inside parent clock
brackets on each OS. macOS used direct `sysctlbyname(kern.bootsessionuuid)` and
`CLOCK_UPTIME_RAW`; all shared the boot domain, maximum bracket4.34ms. Ordinary
app execution succeeded; the tool sandbox denies this sysctl, so the eventual
Rust adapter must return an explicit unavailable-domain error in that environment.
No wall-clock fallback is qualified.

A disposable512MiB Alpine3.23.3 aarch64 QEMU VM (Linux6.18.7) used
`CLOCK_MONOTONIC`, bounded `/proc/sys/kernel/random/boot_id`, and stat identity of
`/proc/self/ns/time`. All100 brackets passed (maximum0.423ms). Another100 passed
inside `unshare --time --fork --monotonic120`: boot ID stayed equal and namespace
inode changed from4026531834 to4026532220. This establishes observable differing
domains; actual IPC mismatch rejection remains a Rust module regression gate.

Source: `/private/tmp/foks-deadline-clock-probe.c`; macOS log
`/private/tmp/foks-deadline-clock-darwin.log`; Linux logs
`/private/tmp/foks-deadline-clock-linux-b.log` and
`/private/tmp/foks-deadline-clock-linux-namespace-b.log`. The initial Linux compile
failed GCC's misleading-indentation warning; splitting the independent fclose onto
its own line fixed it. Failed logs remain preserved. VM configuration and start/stop
logs use `/private/tmp/foks-deadline-linux*`; the disposable VM was stopped/deleted
after qualification to recover build space. These are API/domain feasibility
results, not end-to-end deadline, suspend, overload, or host/AZ recovery proof.
Safe Rust API selection and two-process production-module tests remain required.


Safe Rust API candidate: reuse existing nix0.30 `clock_gettime` and platform
clock IDs (the server already uses this safe wrapper). For Darwin boot identity,
consider target-only pinned sysctl0.7.1 `Ctl::new(...).value_string()`; validate the
returned UUID's exact bounded shape and fail on unavailable/unsupported values.
[The crate's API documentation](https://docs.rs/sysctl/0.7.1/sysctl/trait.Sysctl.html)
provides the safe named-control read. Inspect its macOS implementation and test the
actual Rust module across processes before adopting it; no subprocess command
lookup or wall-clock fallback is proposed for request execution. Current
agent/proto/client forbid unsafe code, which this design preserves. IPC pure types
and OS clock acquisition should stay separate, potentially through a small shared
local-deadline crate. No dependency or IPC change has been made for D3 yet.

### D3a preparatory module contract (root plan review)

Implement an unused `foks-local-deadline` workspace crate first. Its default
feature exposes only strict serde control types; optional `clock` enables the
safe OS adapter and local `Instant` binding. The future agent-proto dependency
uses pure types without enabling clock acquisition. Agent/client will enable it
in D3b; this preparatory commit changes no IPC version or live behavior.

Envelope fields are `version=1`, a tagged clock domain, and canonical decimal
`expires_ns`. Linux domain is boot UUID plus decimal device/inode of the calling
thread's time namespace; Darwin domain is boot UUID plus fixed uptime-raw tag.
UUIDs have exact36-character ASCII hexadecimal/hyphen shape and are normalized
to lower case by the OS adapter; wire input requires canonical lower case.
Every integer has checked u64 parsing and strict string encoding. Unknown fields,
numeric JSON integers, leading zeroes and unsupported versions/tags fail decoding.
No mutable public envelope fields; constructors enforce these invariants.

Clock acquisition uses bounded reads, checks namespace identity before and after
the Linux clock sample, and never caches the domain across namespace changes.
Use `/proc/thread-self/ns/time` so a process with a thread that entered another
namespace cannot report the main thread's domain. Darwin reads the fixed
`kern.bootsessionuuid` through the safe sysctl0.7.1 API. Its source was inspected
from the pinned crate: read-only `Ctl::new` resolves a kernel OID and `value_string`
reads its value; the fixed kernel UUID is validated before use. The dependency
allocates the kernel-reported value length, so this is not an untrusted IPC-string
allocation boundary. No user-selected sysctl name or command subprocess exists.
Nix0.30 clock_gettime returns a checked timespec; Darwin uses the libc constant
through safe ClockId::from_raw. Unsupported OS returns a typed error.

`Deadline::capture(original_instant)` samples once and constructs the wire
coordinate using remaining time after sampling. `Deadline::receive(envelope,cap)`
captures its Instant before sampling, validates domain, then clamps to the cap.
The retained wire coordinate is also clamped; nested `shorten` can never extend
either coordinate. `check()` validates both original local Instant and current
OS-domain expiry. Expose immutable envelope and local Instant getters. Preserve
typed expired/range/domain/unavailable errors. Injectable sampling stays private
to unit tests so production callers cannot accidentally select a synthetic clock.

Tests cover exact decoding, overflow, unsupported domain/version, sampling delay
consuming budget, receiver delay consuming budget, clamp/shorten nonextension,
domain change and expiry. A small qualification binary performs no daemon work:
print one bounded sample, validate an input envelope, or run a parent/exec-child
bracket test. Run the actual Rust crate on macOS and disposable Linux, including
time-namespace mismatch, before adopting it in live IPC. Author review and an
independent agent review precede test execution. Keep Markdown unstaged.

D3a qualification passed on the frozen seven-file snapshot (root manifest/lock
plus five crate files) recorded in `/private/tmp/foks-d3a-sha256.json`; exact source
is retained under `/private/tmp/foks-d3a-frozen`. Seven clock/type tests and three
pure-feature tests passed on macOS, along with all-target Clippy. Actual macOS
parent/exec-child qualification passed100 brackets (maximum6.11ms), and a separate
process accepted the bound envelope. Linux Alpine3.23.3/Rust1.91.1 passed the same
seven tests and100 brackets (maximum0.516ms); a changed time namespace rejected
the original envelope with DomainMismatch, and100 brackets inside the changed
namespace passed (maximum0.186ms). All29 dependency package identities/checksums
in the reduced Linux workspace match the frozen root lock; all five Rust crate
files match exactly. AppleDouble tar metadata was excluded from source comparison.
The disposable VM was stopped/deleted. Logs use
`/private/tmp/foks-deadline-rust-{macos,linux}-*`; no live IPC migration is claimed.

Independent plan review passed with the explicit requirement that both receive
and shorten propagate only cap remaining after sampling; local Instant anchors
before sampling. Root author review of the unused implementation passed: checked
integer/UUID fields, namespace revalidation, expiry/range failures, pure feature
boundary and both-clock publication checks inspected. Added tests for Linux
namespace integer encoding, actual earlier shortening, zero cap and sample-cost
expiry during check. Code review by the second agent is pending; no tests run yet.

### D3b consumer execution map (proposed interface freeze)

Baseline is U commit `6c0f8674` plus the unused D3a crate. This phase changes
local IPC36 to37 in one compatible consumer commit. It does not change remote
FOKS encoding or the upload/receipt schema. The following signatures are the
proposed boundary between the two implementers; no consumer code has changed.

**Public controls and ownership.** `foks-agent-proto` depends on the pure default
local-deadline crate and reexports `DeadlineEnvelope`. Add the mandatory
`deadline: DeadlineEnvelope` field to `Request`, and change construction to
`Request::new(id, operation, deadline)`. Missing/invalid context has no default.
Old version36 frames still receive version mismatch, not silent interpretation.
A pure `Operation::deadline_class() -> DeadlineClass` returns `Ordinary`,
`ChatPoll`, `Pairing`, or `Bulk`; caps are15s,60s,300s,300s respectively. The class
is computed from the actual operation and is not supplied by JSON. Initially
`PutKvStream`, `ResumeKvUpload`, and adapter file-upload execution are Bulk;
metadata/issuance/preparation/status/cancel and individual download-chunk calls
remain Ordinary. The desktop download job can bind300s across its sequence of
ordinary calls. The existing six `is_device_pairing_wait` variants are Pairing;
only `ChatAction::PollInbox` is ChatPoll. Exhaustive classification tests protect
these lists. Any additional bulk route needs an explicit reviewed entry.

`foks-agent-client` enables `clock` and adds
`AgentClient::with_operation_deadline(&self, original: Instant) -> Result<Self>`.
The returned client stores an owned `Deadline`; an already bound client uses
`shorten(original)` so rebinding cannot extend either coordinate. Its reusable
base stays unbound. Clones share `Arc<AtomicU64>` request IDs. Existing `call`,
`call_cancellable`, `put_kv_stream`, and cancellable stream signatures remain.
`set_timeout(Duration)` becomes an explicit per-call shortening ceiling; the
unset default selects the operation's class cap. A caller's short custom timeout
must never be raised to60s/300s by the current `max` branches. Each public entry
captures its original Instant before encoding, clock reads, or connection work,
then either captures a new Deadline or shortens the client's retained one.

Private `call_platform`/`upload_platform` take `&Deadline`, not a relative budget.
Streaming preregistration calls a private operation helper with the SAME retained
Deadline as header/ready/source/frames/commit/final response. It must not reenter
a public call that captures a new cap. Before and after every synchronous source
read and before every socket write, validate cancellation and the owned deadline.
Transport wait helpers can use its Instant after an immediate domain check;
recheck after waiting and before the next effect. Encoding is inside the budget;
an expired encoded request is erased without connecting. Mutation ambiguity
starts when request bytes may have reached the agent, preserving current binding
and response error rules. No source read is authorized by a deadline retry.

The agent validates `Deadline::receive(&request.deadline, class_cap)` immediately
after bounded decoding, before bootstrap checks, branch dispatch or admission.
The configured ordinary request limit may shorten the15s ordinary cap; it must
not accidentally shorten all named long-job classes to the ordinary default or
raise their fixed caps. A pre-envelope frame/invalid-frame response has only a
separate bounded connection budget: a malformed envelope cannot authorize work.
Once a valid envelope exists, EVERY response path, including early errors and
ready replies, uses that same context. At expiry close the socket instead of
starting a fresh response timeout; a best-effort deadline response is possible
only before expiry within the reserved margin.

**Internal agent signatures.** Add a private cloneable `RequestControl` holding
`Deadline` and `CancellationToken`, with `check()`, `deadline() -> &Deadline`,
`remaining() -> Result<Duration>`, and a shortened admission deadline that leaves
the existing reply margin. Its constructor accepts an owned Deadline, never a
Duration. The request loop, `supervise_blocking`, streaming worker, dispatch,
`read_cache::open_profile_session`, data/chat adapters and profile-work TLS take
this control (or borrow it) in place of their timeout/cancellation pair. Network
per-RPC limits are configured separately and never constructed from elapsed
request duration. `profile_work::with_control` installs the existing control
without `Instant::now()+timeout`; nested controls may only shorten it. Add
`Coordinator::acquire_until(..., &RequestControl)` and its blocking counterpart;
check before admission, after admission, after the worker semaphore and at actual
blocking-worker entry. Existing relative helpers may remain test/job-boundary
conveniences only after an audit proves no foreground path uses them.

| Path | Required propagation and ownership |
| --- | --- |
| Ordinary dispatch and cached-read retry | Preserve the same control through profile FIFO, semaphore, shared-to-exclusive retry, stale-auth retry, registry/credential setup and final response. Retry only acquisition or existing allowed reads; never rerun a mutation callback. |
| Streaming upload | Same deadline from preregistration through profile admission, worker admission, durable ready barrier, bounded channel sends, every frame, worker join and final flush. Bound `sender.send` as well as reads. The ready-error `worker.await` branch also uses the absolute deadline. |
| ReconcileProfile | Subscriber uses its caller deadline through renewal plus identity phases; renewal cannot consume a new full cap before identity receives another. Validate profile name and expiry before joining any flight. |
| RefreshLease | Same subscriber rule; HTTP send/body and local apply are bounded by the flight's independent job deadline. Caller expiry does not cancel work still useful to other subscribers. |
| Chat poll | Same caller deadline through polling semaphore, profile/context setup, network poll, reacquisition/validation, and response. Clamp server poll duration to remaining budget; remove fresh60s deadline inside handler. |
| Pairing variants | Preserve caller context through both relay polls and completion; derive relay budget from remaining absolute interval with existing completion margin. Explicit shorter caller budget wins. |
| InitializeState, AgentStatus, Ping and bootstrap rejection | Mandatory envelope even for bootstrap/status. Local setup/encoding remains cooperative; no early-return branch starts a fresh budget. |
| Scheduled jobs | Mint an independent named job deadline at job admission, before waiting for profile/worker capacity. Scheduled profile work uses the configured scheduler budget; hosted refresh uses its existing explicit job cap. No foreground context is inherited. |
| Retention | Preserve the reviewed50ms physical-turn deadline, including coordinator wait, independently of foreground operations. Its8-removal budget and retained worker permit are unchanged. |

Connectivity coalescing needs two distinct controls: a subscriber deadline and a
single independently captured job deadline created when the bounded flight entry
is inserted. A later joiner never refreshes that job. `coalesce` should accept
`subscriber: &RequestControl` and a closure receiving the new job control; it
waits with `timeout_at(subscriber.deadline().instant())`. The flight retains its
entry and physical worker ownership until actual completion, even when all
observers time out. Renewal and identity must have explicit fixed job caps; the
foreground reconcile pipeline separately enforces its one caller deadline.
Foreground subscriber cancellation must not become the flight cancellation token.

`supervise_blocking` must return/close at the original deadline without awaiting
the extra1s grace on the response path. Transfer a timed-out JoinHandle to a
bounded owned reaper whose cardinality is covered by the original physical-worker
permits. The worker retains semaphore, profile/database/protected-state ownership
until its physical work returns. The reaper awaits it, zeroizes/discards any late
Response plaintext, and releases the returned profile permit; it cannot launch
unbounded replacement work. Apply the same ownership rule to streaming early
returns and chat-poll cancellation. Grace remains a supervision/shutdown policy,
not additional caller response time. No new remote mutation may start after a
cooperative expiry check, but an already running syscall/RPC may finish and its
checkpoint obligation must still be completed.

**ProfileSession and checked admission.** Add
`ProfileSession::open_with_deadline(registry, name, deadline: Deadline,
cancellation: CancellationToken) -> Result<Self>` and
`open_with_deadline_and_client(..., base: &FoksClient) -> Result<Self>`. Check
before profile lookup/directory preparation and after any blocking preparation;
clone the base, set its per-RPC timeout to15s, bind D2
`with_operation_deadline(deadline.instant())`, and store the full Deadline on the
session for domain/pre-effect checks. A narrower configured RPC timeout remains
narrower. `related_profile` clones the full context and D2 transport controls;
base pooled clients stay context-free. Existing no-deadline session construction
is retained for offline setup/tests only; audit every live agent/direct-CLI
remote entry and migrate it. Existing relative controlled constructors can be
convenience boundaries that capture once, but no propagated request may call one.

Add `ProfileSession::check_operation()` and checks around checked-session
admission, nested/paired admission, optional pre-callback maintenance and the
callback entry. Bound file-lock acquisition using existing try-lock primitives
plus the session deadline, preserving canonical ordering and the newly reviewed
fresh-empty admission protocol. In particular, do not repeatedly rerun a
partially admitted callback or reset a freshness observation to make contention
succeed. The profile, database and authority lock helpers need a context-aware
wait path. Raw filesystem/keychain calls that cannot be interrupted remain owned
physical work, with immediate checks on return. Paired sessions must share the
same or a shorter context; no effect begins while either side is expired.

Once a callback has changed hard state, its external checkpoint publication and
lock release MUST still run after expiry/cancellation. They are mandatory cleanup,
not a newly authorized mutation. Never insert a deadline return before this
publication and strand valid local evidence. Pre-source checkpoint barriers must
check before the source is read, after the publication completes. Local receipt
returns also honor the caller deadline; time expiry does not erase the receipt.

**File split.** This agent owns agent-proto (`message.rs`, framing tests/Cargo),
agent-client (`lib.rs`, socket transport/tests/Cargo), agent (`main.rs`,
`profile_work.rs`, `read_cache.rs`, `data.rs`, `chat.rs`, `chat_poll.rs`,
`connectivity.rs`, retention and associated tests/Cargo), client-app (`registry`,
`checkpoint`, `runtime`, affected session-constructor call sites/tests/Cargo).
Root owns CLI, native desktop wrappers/transport/vault commands, desktop backend,
MCP and other external consumers/fixtures. Root owns workspace dependency/lock
updates after coordinating exact feature selections. Any necessary core client
change beyond existing D2 transport controls requires a separate reviewed hunk;
no server or DB-schema changes belong in this phase.

Root's consumer contract is one `Instant` captured at the confirmed execution
boundary, then one derived `AgentClient::with_operation_deadline(original)` reused
for all IPC calls. Direct CLI remote work instead captures one Deadline and passes
it to the new ProfileSession constructor. Human file/save/unlock dialogs remain
separate from the execution budget; D1 unlocked-generation checks still span them.
Native download chunks, inspect/write, issuance/registration/stream, MCP subcalls,
and retry/recovery helpers must preserve the original execution Instant. Native
60s blanket timeout is replaced by the explicit operation/job class. Preserve
renderer epochs and native publication gates independently of deadline expiry.

**Review/test freeze.** Before consumer edits, root reviews these signatures and
special branches; independent review follows. Tests require two code reviews,
then the shared Cargo slot. Use deterministic queued/admission/source/publication
barriers and private fake clocks where possible, including no refresh across
preregistration, shared retry and coalesced joins; expired callbacks never run;
mandatory post-effect checkpoint still publishes; permits remain held through a
blocked worker and late plaintext is erased. Add a delayed final reader, bounded
stream-channel backpressure and ready-error worker branch. Run real IPC37 framing
and ordinary/poll/pairing/stream/coalesced/background paths, source-once upload
regression, app admission/private authority regressions, native serial/UI and
CLI/backend/MCP fixtures. Platform clock qualification is a prerequisite, not a
substitute for these end-to-end tests. No hard preemption of synchronous
filesystem work or successful15s host/AZ recovery is claimed by this commit.

D3b audit clarifications before approval:

- `deadline_class` must exhaustively match every `Operation` and relevant nested
  action enum, with no wildcard default hiding future polling/bulk additions.
  SSO Poll is currently a250ms isolated wait (`client-app/src/sso.rs`) and remains
  Ordinary; enumerate the other SSO actions explicitly. The agent `sso.rs` adapter
  is in this agent's ownership map too. `ExecuteDataWrite` has no file bytes and
  remains Ordinary unless an actual inspected route resumes large-file work;
  the existing adapter stream is already `PutKvStream` and is Bulk. Do not grant
  an entire generic write enum300s merely because some other route uploads files.
- A15s ordinary per-RPC base must not erase existing protocol-aware overrides:
  `client/src/realtime.rs` sets `REALTIME_POLL_BUDGET` on an isolated checked-out
  connection; KEX sets its per-poll slice timeout; SSO adds its bounded poll
  allowance. D2 already clamps these to the retained operation Instant. Preserve
  those overrides and test the long-poll/pairing flows; the15s default applies to
  ordinary RPCs and file chunks, not an implicit incompatible shortening of an
  already specified protocol wait.
- Existing relative session-constructor callers are limited in production to
  `read_cache::open_profile_session` and the two chat-poll setup/rescope calls.
  Both migrate to the absolute constructors. Scheduled profile work reaches the
  read-cache helper and therefore supplies a fresh job control at the scheduler
  boundary; retention currently calls bare `ProfileSession::open` and will bind
  its existing50ms turn deadline. Keep old `open_with_control[_and_client]` as
  bounded compatibility entry points: capture once BEFORE preparation from their
  explicitly supplied Duration, retain that absolute deadline, and preserve the
  supplied per-RPC cap. Do not silently replace a scheduled/poll duration with
  an ordinary15s cap. No live propagated path calls these relative boundaries.
- Profile registration bypasses ordinary session creation today:
  `ClientCredentials::check_and_add_profile[_for_host]_with_control` ultimately
  constructs a client around registry.rs841, then opens an unbound session to
  finish checked enrollment. Add corresponding `_with_deadline` variants carrying
  the same context through staging reservation, probe, registry recheck,
  publication and final checked admission. Keep mandatory completion/rollback
  cleanup after effects even if the deadline expires. Its existing publication
  lock ordering and crash recovery must not be weakened.
- Reset preview/reset currently open bare sessions in main.rs3706/3733. These
  local operations still require the request deadline around blocking preparation
  and before deletion/ticket consumption. Registry writes, initialization and
  credential-only operations do not all create a ProfileSession; dispatch must
  explicitly check the RequestControl around their blocking preparation and
  immediately before a new effect. A session-only change is insufficient.
- Add `Response::zeroize_plaintext()` (or an equivalent private owned response
  guard) for recursive JSON success values and textual error fields, and ensure
  late responses and encoded buffers are erased on deadline/abandonment. Existing
  `Operation::zeroize_plaintext` covers request secrets, not returned JSON. The
  reaper must not merely drop an ordinary serde_json::Value containing plaintext.
- Root's consumer API can use `with_operation_deadline(Instant)` without exposing
  ClockSample or mutable envelope fields. Any local clock/domain failure maps to
  an explicit actionable client error before sending; preserve it as distinct
  from an ambiguous deadline after request bytes may have been delivered.

### D3b root consumer propagation contract

Root direct plan review passed for the consumer map, with a bounded version peek
before full37 request decoding so36 frames lacking the new field still report
VersionMismatch. The actual same-host context must retain both Instant and wire
coordinate. D3b adds `Deadline::intersect(&self,&Self)` with equal-domain checking,
minimum Instant and minimum wire expiry. `AgentClient::with_local_deadline` and
`FoksClient::with_local_deadline` take an owned Deadline; copying only its Instant
is insufficient. Existing core transport `OperationControl::remaining` runs inside
DNS/connect/socket poll and pooled-call boundaries, so it can check the retained
domain without modifying KEX/SSO/realtime protocol logic. Empty pooled sockets
store no previous borrower's controls. Keep failures before dispatch distinct
from ambiguity after request bytes may have reached the peer.

Root owns the following external-consumer changes in the same D3b commit:

- Extend `AgentTransport` with required `call_with_deadline(operation,&Deadline,
  cancelled)` and an explicit stream equivalent. Existing call conveniences stay,
  but there is no default implementation that silently ignores a supplied
  deadline. Concrete AgentClient, ObservedTransport, LabelledTransport and
  IntervalTransport forward the same control. Tests' fake implementations may
  explicitly emulate it; production implementers must choose behavior visibly.
  A bound transport wrapper owns one Deadline and forwards all existing helper
  calls through these required methods, preserving cancellation and post-response
  zeroization when expiry wins. Streaming defaults may reject unsupported work.
- Capture ordinary15s or named bulk300s at native confirmed execution entry,
  before spawn_blocking and startup gating. StartupGate waits use the original
  absolute bound. Maintenance reservations remain fail-fast for ordinary use;
  explicit maintenance draining consumes its own original bounded context.
  AgentHandle direct calls, labelled transports and unreserved startup probes
  all preserve their respective context. Remove the blanket60s client timeout.
  A timed-out join retains the physical worker/owned state; discard and erase
  its eventual plaintext response instead of awaiting additional response time.
- Bind one transport across read/chunks, inspect/write and upload
  issuance/registration/stream. Save/open/unlock/PIN dialogs are a separate human
  phase; start the network/job budget after confirmation, while unlocked-generation
  checks still span the dialog. Check the retained deadline alongside interval
  validation before returning plaintext, clipboard publication and file rename;
  synchronize temporary files outside the existing publication gate. Preserve
  original entry identity/version and renderer epoch checks.
- Direct CLI network workflows open deadline-bound ProfileSessions before
  registry/credential preparation, including related/federated profiles and
  staged registration. Interactive enrollment phases obtain a fresh explicit
  execution context after user input; never carry an expired context through
  a human prompt or refresh it during an automatic retry. CLI agent commands
  reuse a bound AgentClient for each complete command. The standalone desktop
  backend likewise binds the whole command rather than each helper RPC.
- Each MCP invocation gets a derived scoped AgentBackend/client before scope
  resolution and subsequent reads/writes/chunks. Preserve the reusable backend
  for future invocations. Initial account binding's existing5s contention loop
  binds its original deadline once rather than resetting relative timeouts.
  Existing cancellation still closes IPC and preserves mutation ambiguity.

Tests cover context forwarding through every concrete wrapper, expired startup
and worker queues, original bound surviving multiple helper calls, late plaintext
erasure, and no post-expiry clipboard/file publication. Include native transport
and vault barriers, CLI staged enrollment/upload integration, MCP multi-call
execution and5s binding contention. This plan does not claim arbitrary local
filesystem calls or lock syscalls are preemptible. Two reviews precede tests.

Root review gates for D3b (2026-10-15): bounded header/version dispatch must precede
full Request deserialization so valid old IPC36 requests lacking the new field
still produce `VersionMismatch`. Tests cover both an old valid frame and malformed
new37 envelopes; neither may create work. Root will separately freeze the external
`AgentTransport` context-forwarding API across observed/labelled/startup wrappers.

The remaining narrow core proposal is to carry an optional full local Deadline
through `FoksClient`, `PooledConnection` and `transport::OperationControl`, and
check it in `OperationControl::remaining`. That helper already gates DNS/connect,
pooled RPC admission and each controlled TCP read/write poll, so protocol-specific
KEX/realtime/SSO timeout overrides retain domain checks without changing their
remote formats. Add `FoksClient::with_local_deadline(&self, Deadline)->Result<Self>`;
clone/isolated/related controls preserve it and idle pooled sockets must not retain
it after return. A proposed `Deadline::intersect(&self,&Self)->Result<Self>` checks
matching domains and takes the earlier Instant AND wire expiry, then validates
without recapturing a fresh cap. This API addition needs narrow approval before
code. `with_operation_controls_from` currently returns Self; there is only one
production caller (related_profile). Prefer making it fallible and adapting that
caller/tests rather than silently dropping a mismatched inherited domain. Existing
D2 Instant-only callers retain their behavior. Typed domain/unavailable errors must
survive transport I/O wrapping; they cannot be mislabeled as a retryable timeout.

Approved D3b interface refinement: add
`AgentClient::with_local_deadline(&self, deadline: Deadline) -> Result<Self>` as
well as the convenience Instant binding. The full-context method intersects any
existing binding using the new `Deadline::intersect` and never recaptures from an
Instant. Root's required transport methods are
`call_with_deadline(operation, &Deadline, cancelled)` and
`stream_with_deadline(header, reader, &Deadline, cancelled)`; concrete observed,
labelled and interval/startup wrappers forward the same context. There is no
trait default that ignores a deadline. The owned core transport Deadline and
`intersect` are approved D3b changes, excluded from the frozen D3a preparatory
snapshot. Typed unavailable/domain mismatch errors stay distinct before dispatch;
once mutation bytes may have been delivered they remain inside the existing
ambiguous-outcome wrapper.

Additional non-FOKS network boundary found during map review: SSO constructs
`foks_oidc::ProviderHttp` directly. Its `get` and `SyncHttpClient::call` perform
provider discovery/JWKS/token HTTP with a fresh10s timeout per request; they are
outside the FoksClient control above. Proposed narrow optional
`ProviderHttp::with_local_deadline(Deadline)->Result<Self>` binds the same full
context, checks before and after each request and bounded body read, and applies
`min(PROVIDER_DEADLINE_MS, remaining)` to the individual HTTP request. Default
unbound server use stays unchanged. Agent SSO and root's direct CLI SSO constructors
must bind it. A blocking resolver/HTTP operation remains physically owned until it
returns; expiry forbids a subsequent provider or FOKS request. This additional
`foks-oidc` surface awaits root/independent plan approval before editing.

Root approved the optional ProviderHttp deadline binding as part of D3b. Tests
must show a pre-expired context sends no HTTP request and a first response that
consumes the budget prevents a second discovery/JWKS request; no automatic retry
may reset it. Exercise both GET and SyncHttpClient request/body paths. This agent
owns the narrow `foks-oidc` change and its tests; the default server behavior is
unchanged. Independent whole-plan review remains the final gate before consumer
implementation starts.

Native consumer context refinement (root author review and recovery independent
review PASS): AppState carries an optional owned Deadline only on a cloned
per-command view. begin_operation captures before admission/preparation and
intersects an existing context; profile/store scopes preserve it. Transport and
call helpers fail closed without that context. Human dialogs delimit explicit
execution stages: capture the newly confirmed stage from unbound state, retain
unlocked generation and receipt identity, and never refresh an ambiguous retry.
Local-only commands need no clock. Required deadline transport capabilities may
have explicit Unsupported defaults; no default may silently ignore context.
Concrete production wrappers forward it. A worker result guard owns its physical
permit through actual completion and late plaintext erasure, including receiver
cancellation; this removes the need for an unbounded reaper task queue. Final
publication checks occur inside the generation gate. Already-entered blocking
filesystem publication is not claimed preemptible. Two code reviews remain
required before consumer tests.

D3a committed e33ff9fd after source review, lock-delta review and macOS/Linux
qualification. Only seven implementation/build files were staged; ongoing D3b
intersect/consumer changes and all Markdown remain outside that commit.

### D3b bot-token publication correction

The deadline audit found that Export currently removes the only encrypted token
before the IPC reply is delivered. A late-response discard therefore destroys a
credential without publishing it. The local IPC37 contract becomes two-phase;
no FOKS wire or cryptographic format changes are involved.

- Add `BotAction::CompleteExport { operation_id }` and corresponding local app
  action. Add serde-default `export_offered: bool` to encrypted PendingBot.
- Export requires verified Finalized/noncancelled enrollment, persists
  `export_offered = true` before returning a clone of the retained exact token,
  and never consumes it. Repeated Export returns that same token until completed.
- CompleteExport branches immediately after stored identity/journal validation,
  before any credential/network reconciliation. It requires Finalized and
  noncancelled; a present token additionally requires persisted offered=true.
  It erases the token durably, then returns redacted status. Already absent token
  with Finalized/noncancelled is an idempotent completion, including pre-upgrade
  successful exports. Prepared, unknown, rejected, and cancelled states refuse
  with no enrollment, reconciliation, or token generation.
- Frontends retain exact operation ID and scope. Reserve a private new destination
  before Export. The opaque reservation opens the parent directory first and
  creates a random private hidden temp relative to that descriptor with openat.
  prepare_export writes and fsyncs the temp outside the generation gate;
  publish_export uses no-clobber linkat under that gate after deadline validation;
  finish_export removes the temporary link and fsyncs the retained directory
  outside the gate. Drop only removes the temporary name. The destination stays
  absent until publication and existing destinations stay unchanged. Only after
  those durability steps succeed may CompleteExport be sent, using the same
  original full deadline. A deadline or sync failure leaves the original token
  recoverable; a lost CompleteExport reply is retryable by exact ID. A failure
  after durable publication must explain that the saved file remains usable,
  while local completion may still need retry; it must not delete that file.
- Root owns the native consumer. This agent owns proto bot.rs, client-app
  bot_token.rs, agent bot_token.rs, secret_file.rs, and CLI bot_token.rs. Shared
  helper `reserve_export` returns an opaque reservation; `prepare_export`,
  `publish_export`, and `finish_export` accept it. Callers infer its type.
  Unsupported platforms refuse rather than weaken the fd-relative guarantee. No plaintext is
  printed. The original response token and copied string are zeroized.
- Tests cover replay of identical offered token, offer persistence across reopen,
  premature completion without effects, cancelled/rejected refusal, legacy
  finalized tokenNone, duplicate completion, and publication failure preventing
  CompleteExport. Shared helper tests cover private creation and durable output;
  native/CLI transcript tests assert CompleteExport follows completed publication
  and retains original scope/ID/deadline.

Author plan review: the offer flag is encrypted with existing PendingBot and
checked under existing profile exclusion. Every erase follows durable local
publication, while an ambiguous reply cannot trigger fresh enrollment. Legacy
absent tokens are only accepted behind exact journal kind/host/user/device and
Finalized/noncancelled checks. This retains more local secret material until
explicit completion; abandoned offers remain recoverable and existing pending
capacity bounds still apply. Filesystem sync is cooperative and can overrun a
request budget; completion is not sent after the original deadline expires.

CLI propagation implementation follows the reviewed command-stage plan: run
captures before profile/state preparation, helper calls require that scope,
startup uses an intersection with five seconds, and ordinary commands use15s
while file/state bulk and pairing stages use300s. Retention uses its explicit
1–60s limit. Pure help/parser exits happen before clock sampling. State export
pauses its scope before human confirmation and starts a fresh stage only after
that confirmation; --yes preserves the original coordinate. MCP startup ends
its stage before the long-lived stdio loop, whose requests have separate budgets.

### D3b native/desktop/MCP consumer review record

Root author review and recovery-agent independent review passed the 45-path consumer
snapshot in `/private/tmp/foks-d3b-consumer-manifest.txt`; the implementation is not yet
tested. The common native worker retains its physical admission until late results
are erased, including when its awaiting command has timed out or disappeared.
Owned command contexts survive nested native calls and profile/store views. Dialog
waits remain outside the explicitly started post-selection transfer phase. Final
plaintext, clipboard, file and browser-publication checks retain the original context.
Process creation is checked within the unlock publication gate; waiting for a launcher
occurs outside that gate. Already-entered synchronous OS/filesystem operations remain
non-preemptible, retain physical admission, and may finish after caller timeout.

Independent review found and corrected preflight ownership gaps, final plaintext
checks, SSO URL erasure, and one-time bot-token export loss. Export now uses a durable
retained offer plus local CompleteExport acknowledgment after private temporary-file
sync, no-clobber publication and parent-directory sync. Native generation/deadline
checks guard only final publication; preparation and synchronization remain outside
the application lock gate. A saved file survives an interrupted acknowledgment, and
errors do not claim the agent still retains a token when acknowledgment may have
committed. The bot lifecycle and CLI implementation have their own supplemental
review gate before the combined IPC37 validation.

Tests authored for review include original-envelope forwarding across helper calls,
unsupported adapter rejection, late worker-result erasure/admission lifetime, MCP
timeout/cancellation while backend work is paused, download expiry after a read,
between chunks and before publication, and bot export locking before publication.
They use explicit checkpoints and bounded channels; expiry tests release a paused
worker only after the actual absolute deadline. Cargo.lock adds only twelve direct
dependency edges to existing locked versions; root and independent review passed.

### D3b qualification in progress, 2026-10-16

Validation uses an isolated worktree based on a01bf92c and an explicit frozen
source manifest. IPC/client/clock tests passed (62/20/8), core transport10 and
OIDC6 passed. The full client-app suite passed serially245 with2ignored. Its first
parallel run had236pass/9fail/2ignored: all failures involved cooperative cleanup
progress under filesystem load, and all passed serially without production edits.
The50ms maintenance turn includes database-open cost, so an overloaded turn can
perform no erasure. Preserve that as a capacity/progress qualification concern;
serial passing is not evidence of unlimited cleanup throughput.

One older data_writes integration assertion expected immediate spool compaction
after a successful upload, despite the committed UC deferred-cleanup contract.
Its isolated failure reproduced. The reviewed test correction first asserts the
retained dependency and charged debt, drives bounded maintenance until both clear,
then retains the original receipt identity/replay checks. Root independent review
passed before rerun. The test joins the frozen manifest; no cleanup production
behavior was changed to make this test pass. Agent/CLI/desktop/MCP and final lint
gates remain pending; this is not a completed D3b qualification record.

Native consumer final review amendment: preserve an already observed terminal security/compatibility or clock error through final deadline expiry, while erasing late diagnostic text. Connection loss alone does not override expiry. Root direct and recovery independent reviews passed before tests; deterministic completed-result tests cover classification, erasure and physical permit release.

D3b native validation: two malformed archive matches and three concatenated imports were discovered by compilation, corrected, directly and independently reviewed, then rerun. Removed nine now-unused imports. The vault read success gate no longer replaces a preserved terminal error with a later timeout; successful plaintext still passes original generation/deadline validation. All262 native tests passed serially; logs foks-d3b-native-tests-run1/2/3 preserve both initial failures and the passing run.

D3b committed as `74be8a2b` after direct and independent reviews and exact96-file source hash verification against the isolated tested worktree. Final gates:897 passing tests (557 core/app/agent/CLI/OIDC/clock,50 desktop library,9 backend binary,5 backend transcripts,14 MCP,262 serial native desktop), three existing ignored tests; strict all-target Clippy across11 affected packages and whole-workspace formatting passed. Logs and source hashes: `/private/tmp/foks-d3b-final-qualification.json`, `/private/tmp/foks-d3b-union-sha256.json`. All Markdown remains unstaged. Parallel cleanup throughput is unqualified; successful host/AZ15s recovery and authenticated cross-host deadlines are not claimed.


### Hosted interruption audit for the next qualification (source evidence only)

The locally pinned celld0.6.2 source contains a process CPU watchdog in
`crates/celld/js.rs:2623–2764`. It measures the executing thread's CPU budget,
reschedules when CPU remains, and calls V8 `terminate_execution` on exhaustion.
This is not an absolute wall-clock deadline: scheduling pauses, blocked I/O and
other waiting do not consume the same CPU allowance. Do not substitute its CPU
limit for the frontend's original15-second budget or the authenticated cross-host
remaining time. Source inspection alone also does not prove that V8 termination
interrupts an already-running native SQLite statement.

The inspected storage module's explicit SQLite progress-handler interruption
entry is `set_interrupt_fault_for_test` around storage.rs3354, compiled under
`celld_internal_tests`. It is not a demonstrated production JavaScript capability.
Before deadline qualification, audit the full dispatch/SQL call path and run
bounded disposable probes for JavaScript/Wasm CPU work, native SQL work, blocked
sync/storage and queued work. Record physical completion/rollback separately from
HTTP timeout and isolate termination; keep permits and unresolved work owned until
actual completion. Terminating one request must not silently certify outstanding
SQL or PUT attempts as stopped. Outer timeout tests alone do not close this gate.

This note records an implementation lead and a qualification gap, not a new
platform requirement or an assertion that no other cancellation mechanism exists.
Any necessary celld adaptation must get its own concrete plan and two source
reviews, and does not require a FOKS public wire-format change.


#### Deployment CPU-limit applicability refinement

A second source pass over the same pinned0.6.2 tree narrows the watchdog lead.
`WorkerConfig.resource_limits` (`js.rs:1926–1928`) explicitly documents that
Deployment Workers leave it empty; its constructor initializes it toNone
(`js.rs:2069`). The only inspected `WorkerConfig::with_resource_limits` call is
in the dynamic Worker Loader construction (`js.rs:7686`). Tracked cell contexts
copy that runtime configuration (`js.rs:11231–11240`), while `CpuWatchdog::start`
returnsNone when no limit is supplied. The bundled dynamic-workers documentation
also places `cpuMs` in WorkerCode/getEntrypoint limits and limits
getDurableObjectClass options to props.

The existing local proof deploys a normal Worker plus a Durable Object binding.
It does not supply this dynamic-worker limit. Therefore the inspected CPU watchdog
is not currently demonstrated to protect our deployed SQL/Wasm invocation at all,
even as a CPU-only backstop. Do not add an unsupported deployment `cpuMs` field
and assume it works. A dynamic-worker probe would qualify that different execution
path only; it cannot stand in for the actual tenant cell path. A production
wall-clock deadline still needs a separately reviewed physical execution and
ownership design, along with probes of the actual deployed route. This is a
source finding, not a runtime assertion of unlimited execution or an exhaustive
claim that celld has no other termination mechanism.

Direct and independent (recovery agent) source reviews completed against the
pinned tree, including the operation-timer exclusions below. No code or
configuration was changed for these findings.


The separate `CELLD_OPERATION_DEADLINE_MS` default15,000
(`crates/celld/actor.rs:41–51`) must not be confused with execution interruption.
Its logic-side `arm_operation_deadlines` (`crates/logic/lib.rs:2798–2848`)
selects activation, owner/lease, publication and durability effects. It explicitly
excludes Restore (total duration scales with replica size; task cannot be canceled)
and StopRuntime (a pending release may still own runtime/database; expiring it
would allow unsafe concurrent restart). That source distinction is appropriate
for ownership safety, but means the setting alone cannot establish our15-second
end-to-end successful recovery target. Qualification must measure restoration
history size and physical runtime retirement separately, and retain admission
while those operations still own resources. No change to the setting is proposed
by this source audit.


### D4a proposed deployed-SQL deadline diagnostic

Plan only, pending root/direct and independent review before implementation.
This small diagnostic tests the actual normal deployed Durable Object path;
it does not introduce a replacement FOKS adapter or a dynamic Worker substitute.
Proposed implementation paths are `tools/foks-celld-proof/deadline-worker.mjs`,
`deadline-worker.test.mjs`, and `run-deadline-local.py`. Keep it independent of
Cargo builds and production configuration. A separate later slice covers the
full original client deadline, blocked durability, recovery and capacity.

The worker has one private, bounded diagnostic surface, protected by a fresh
capability before any actor routing. Fixed operations initialize/read a small
marker table and run a fixed recursive SQLite aggregate inside transactionSync.
The caller selects only an enumerated work size, never SQL, a database name,
loop expression or an arbitrary iteration limit. Sizes are bounded at1000,
100000,1000000 and8000000 recursive steps, producing one exact-decimal aggregate
cell and no large rows or output. Emit a transaction marker before and after
the aggregate; await storage.sync before returning success. A separately armed
short JS timer records whether it could run before the synchronous aggregate
returned; its clock/timing is not used as the external wall-clock oracle.
No production data or tenant schema is used. The actor persists only fixed small
probe counters and results; maximum response/body sizes are explicit.

The controller uses the pinned0.6.2 binary, a new private output/project/watch
root, a disposable Moto bucket, loopback listeners and an environment containing
only dummy AWS credentials. Reuse the established local deployment recipe and
hash the deployed worker/configuration/binary. Supply a short positive actor
operation deadline (250ms) explicitly, recording that it controls activation
and durability effects rather than assuming it preempts SQL. A prewarm failure
is an inconclusive deployment result, not evidence of execution interruption.
An optional default15000ms control run uses a separate process/output and the
same bounded workloads. No unbounded busy loop or arbitrary workload flag.

Measure request start, headers/body completion and subsequent marker observation
with controller monotonic time. Record HTTP status and bounded diagnostic text,
not just success/failure. During the largest work case submit a bounded ordinary
read to the same actor and another actor, recording both queue latencies. Do not
claim the read definitely overlapped SQL unless an observed worker start marker
establishes that ordering; otherwise report overlap unknown. Preserve all raw
observations and process cleanup status. A controller20s total per-case ceiling
closes client connections and terminates/reaps the disposable process if needed;
it does not label a client timeout as canceled SQL or a terminated remote PUT.
The entire diagnostic has a bounded overall process deadline as well.

A success response must match the closed-form sum and the post-sync marker read.
For an interrupted/error response, inspect the surviving or restarted actor and
report whether the transaction marker committed, rolled back or remains unknown.
Do not infer rollback from HTTP errors. If a request exceeds250ms successfully,
that is evidence against treating the actor timer as a250ms total request deadline,
not a harness failure to conceal or a15s production qualification failure by
itself. If no workload exceeds the configured timer, record the probe as
inconclusive for that claim. JavaScript timer ordering is recorded separately
from external elapsed time and SQL physical termination.

Unit checks use a mocked storage boundary only to verify authorization-before-
effects, fixed workload selection, transaction/sync ordering, bounded outputs and
exact aggregation expectation. They are not the SQLite timing oracle. Root/direct
and independent source reviews precede those tests and the actual local run.
After the diagnostic, select any necessary execution-control redesign under a
new reviewed plan; do not automatically lower production limits or add celld
patches based on a source-only assumption.


D4a direct and independent plan reviews passed. Refinements before implementation:
each marker is bound to the closed case ID and one monotonic attempt, so old
successful work cannot satisfy the current observation; timers are cleared in
finally. Controller diagnostic clients are drained/canceled on cleanup without
claiming cancellation of unknown physical SQL or remote work. No Cargo or
production path changes are needed.


D4a three-file source authored in metadata-deadline-probe-validation at345247ce.
Direct author review corrected the controller timeout design before tests: socket
inactivity timeouts alone can be extended by trickled bytes, so each bounded
client also owns a timer that shuts down its socket at the original case deadline.
This stops the diagnostic client only. Worker run success requires exact aggregate,
atomic stage1 marker and sync; errors remain ambiguous until a later marker read.
Timer cleanup is explicit and tested across success/SQL/sync paths. Incoming
empty POST streams are checked before actor routing with one bounded read and a
1s timer, then canceled without awaiting a hostile cancel promise. Main/other
read timing is recorded without claiming native SQL overlap. Fixed marker IDs
cannot be silently replayed. Source manifest/hashes are frozen under
/private/tmp/foks-deadline-probe-*. Independent source review is pending; no test,
deployment or Python execution has run.

D4a independent source review passed. One evidence-wording refinement, directly
reviewed before tests: total HTTP elapsed includes routing and sync, so a success
over250ms does not independently measure SQL duration or disprove SQL CPU control.
The report explicitly leaves SQL-only duration unmeasured.


D4a first local run `/private/tmp/foks-deadline-probe-short-a` stopped at prewarm
with HTTP400 `invalid diagnostic operation`; no probe SQL ran. Both disposable
processes were reaped. Actual celld presents an empty Durable Object POST as a
body stream, so the class boundary's null-only check was stricter than the
already-reviewed frontend check. The narrow correction reuses the same bounded
empty-stream check at the class boundary and adds both-boundary stream tests;
nonempty input still rejects before SQL. Original failure evidence is retained.
Direct review passed; independent review is required before rerunning tests and
using a new deployment directory.


D4a narrow correction received direct and independent source approval; five Node
unit tests and Python syntax validation passed. Fresh actual celld deployments
short-b (250ms) and default-a (15000ms) each completed all four bounded SQL cases
with HTTP200, exact stage1 markers and successful storage.sync responses. The
largest request took0.9269s and0.9311s respectively. Concurrent same/other actor
reads took0.9170/0.9096s and0.9227/0.9152s. These are external total request times;
native SQL overlap and SQL-only duration remain unmeasured. A short actor timer
is not a total request deadline. The cross-actor delay warrants isolate scheduling
and interference qualification; causality is not proven by this observation.
The10ms JS timer never ran before synchronous SQL returned. Both process sets
were reaped; celld required SIGKILL after the3s graceful cleanup budget in the
control run. Cleanup termination is not a production failover measurement.
Evidence: /private/tmp/foks-deadline-probe-results.json plus pinned source hashes
and both qualification reports. Initial short-a prewarm failure remains retained.
No production deadline,15s host/AZ recovery, or remote PUT termination is qualified.
