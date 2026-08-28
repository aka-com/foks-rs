# foks-agent

Bounded resident process for standalone FOKS clients. It listens on a private
Unix-domain socket below an explicit `--state-dir`, runs blocking client work
through a bounded worker semaphore, limits active connections and requests per
connection, and applies read/write/dispatch deadlines. Dispatch cancellation is
propagated into the synchronous FOKS network client; a timed-out operation
closes its local connection because its durable outcome can be ambiguous, and
any surviving blocking work retains its worker permit until it exits.

The resident scheduler polls durable `FoksScheduler` state and runs due user
refreshes only for profiles whose protocol policy permits synchronization.
Scheduler sweeps are single-flight, and a profile already in use by another
frontend or scheduler is skipped rather than tying up a worker.
The agent separately polls each current profile's stable HTTPS compatibility
lease. Fetches are single-flight, limited to four concurrent requests and 64
KiB per response, use WebPKI with bounded redirects and deadlines, then require
the profile-pinned Ed25519 signer, exact target and protocol digest, and a
strictly newer generation before an atomic registry update. Fetch failures
retain the existing short lease; expiry still revokes, while an authenticated
drift artifact revokes on the next successful poll.
The agent does not accept secret-bearing signup, mutation, provisioning, or
recovery requests.

```text
foks-agent --state-dir /private/client
```

macOS and Linux are supported. Windows is currently refused because an
authenticated named-pipe implementation and ACL validation have not landed.
Use `foks-rs` directly on Windows until that boundary exists.
