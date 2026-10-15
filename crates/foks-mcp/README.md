# FOKS MCP

Run the KV or team MCP server over stdio using an existing FOKS account:

```sh
cargo build --locked --release -p foks-cli -p foks-agent
target/release/foks-rs --state-dir /absolute/client/state mcp kv \
  --profile local --account-alias personal
```

Keep `foks-agent` beside `foks-rs` when installing the binaries. The CLI connects
to the authenticated resident agent and starts that sibling executable if needed.
Closing MCP leaves the resident agent running. The selected account must already
be usable by the agent; MCP never asks for passwords or hardware PINs on stdin.
Initialize, provision and unlock accounts through the ordinary client first.

A generic MCP client configuration is:

```json
{
  "mcpServers": {
    "foks-kv": {
      "command": "/absolute/bin/foks-rs",
      "args": ["--state-dir", "/absolute/client/state", "mcp", "kv", "--profile", "local", "--account-alias", "personal"]
    },
    "foks-team": {
      "command": "/absolute/bin/foks-rs",
      "args": ["--state-dir", "/absolute/client/state", "mcp", "team", "--profile", "local", "--account-alias", "personal"]
    }
  }
}
```

Add `--read-only` to the KV arguments to expose only list/get/stat/usage and reject
write calls. Ordinary KV mode permits writes using the selected account's existing
permissions. Team mode exposes list/list-memberships; it does not change teams.

## Tools and scope

| Server | Tools | Inputs |
| --- | --- | --- |
| KV | list, get, stat | path; optional team; get also accepts base64 |
| KV | usage | optional team |
| KV | put | path, content; optional team, base64, mkdir_p, overwrite |
| KV | mkdir | path; optional team, mkdir_p |
| KV | rm | path; optional team, recursive |
| KV | mv | src, dst; optional team |
| Team | list | team name or immutable ID |
| Team | list-memberships | no arguments |

All flags default to false. Paths refer to the encrypted FOKS namespace. Relative
paths start at its root; symlinks are resolved within that namespace. An existing
directory destination for mv receives the source name. Overwrites and removals
retain checked version preconditions. Standard padded base64 represents binary
files; text reads reject binary content instead of replacing bytes.

The process binds one profile/account and authenticated host. A tool can select an
accessible team, not another account or state directory. The current team resolver
uses verified local membership paths, including indirect membership. When remote
components cannot be enumerated, list-memberships returns `complete: false` in
structured content and describes that limit in text. Remote party names can fall
back to verified IDs. This does not claim full remote-directory discovery.

## Recovering a write

Every write returns structured content containing `submission_id`, `status`,
`partial` and optional `node_id`. Successful ordinary writes preserve Go's `ok`
text result; mkdir returns its directory ID. Recovery metadata is an additional
local API, not a change to the FOKS remote protocol.

Supply optional `submission_id` on a write to retain an explicit intent
across client restarts. Its canonical form is
`v1-<16 lowercase hex Unix seconds>-<32 lowercase random hex>` (128 random bits).
Legacy 32-hex IDs are rejected. Reuse it
only with identical semantic inputs, including the selected team. Changed inputs
are rejected. A JSON-RPC request ID is not a durable submission ID. Without the
optional ID, each explicit new write call creates a new intent.

Normal KV mode adds two recovery tools:

- `foks_status({"submission_id":"…"})` reads/reconciles a recorded intent.
- `foks_pending({})` lists pending intents, including handles whose original
  preparation or result reply was lost.

Prepared means remote delivery has not begun; the same input and ID may execute
once. Committed means the operation has sufficient acknowledgment or exact
namespace evidence. Rejected means it cannot execute under that ID. Submission
unknown means delivery may have occurred: inspect status instead of issuing the
write with a new ID. A partial result means ancillary namespace work committed
while final completion is unproven. Cancellation does not undo remote work.

An interrupted upload may leave an unreferenced encrypted object. A lost delete
acknowledgment can remain unknown: current absence alone cannot prove which
delete committed. A recovered write retains its node ID when authenticated evidence proves it;
unproven fields remain absent. None of these results automatically repeats a mutation.

## Bounds and compatibility

Limits are 4 MiB decoded file content, 8 MiB input/output frames, 1,000 catalog or
roster rows, 64 write-path components, four active calls and 16 queued calls per
process. Oversized results fail explicitly. Agent/profile limits also apply.
Each account admits at most 64 active intents and retains at most 65,536 ledger
rows, including submissions awaiting protected cleanup. Terminal evidence is
eligible for pruning 30 days after terminalization. A bounded cleanup/prune pass
runs before new admission; continued saturation returns `retention-full`.
Pending and uncertain submissions are never deleted by age or storage pressure.

New handles must be no more than 24 hours old and no more than five minutes in the
future. Existing ledger evidence takes precedence over age. Fresh unseen handles
return `not-recorded`; retired or stale unseen handles return `expired` and cannot
execute. A durable rejection watermark prevents clock correction from reopening
expired handles. Expiry describes replay eligibility, not proof of an earlier write.

Time uses a persisted anchor plus a monotonic clock that keeps counting during
sleep (`CLOCK_BOOTTIME` on Linux, `CLOCK_MONOTONIC_RAW` on macOS). The anchor
records the kernel boot identifier, so an agent restarted within the same boot
checks the elapsed time across the restart. A wall-clock jump over five minutes
within one process, or a restart more than 24 hours beyond the last trusted floor
that the boot clock does not account for, blocks new admission and pruning with
`clock-untrusted`. A trusted clock that is still behind the irreversible
watermark also refuses new writes until it reaches the watermark; this happens
only after an earlier clock ran more than 24 hours ahead, and no repair
shortens it. A handle issued at the current time is then older than the
watermark, so the write returns `expired`; only a handle issued up to five
minutes ahead, at or past the watermark, is refused with `clock-untrusted`.
Retained status and recovery still work. While admission is refused,
`foks_status` adds `"writes_paused": true` to its result, with
`"writes_resume_at"` (Unix seconds) in the watermark case, and explains what
lifts the pause; `clock-untrusted` refusals carry the same explanation, and an
`expired` write result carries the same fields and explanation in place of the
advice to issue a new handle.
The agent reads this clock status from local state only. First use establishes
the anchor.

Repair runs in the resident agent and is not an MCP tool: a person confirms the
time in the desktop app's account settings or with the CLI, which starts the
agent if needed. `retention clock-status` reports the account's admission;
`retention reanchor` previews the current time and then confirms it:

```sh
foks-rs --state-dir /absolute/client/state retention clock-status \
  --profile local --account-alias personal
foks-rs --state-dir /absolute/client/state retention reanchor \
  --profile local --account-alias personal
foks-rs --state-dir /absolute/client/state retention reanchor \
  --profile local --account-alias personal --unix-seconds <proposed_wall> \
  --confirm-digest <confirmation_digest>
```

The preview proposes the current time unless `--unix-seconds` names one; the
confirmation repeats the preview's `proposed_wall` and `confirmation_digest`. The
agent pairs the confirmed time with its own monotonic clock at confirmation, so
any difference from the computer's clock reduces the five-minute skew that later
writes may use. It refuses a preview more than five minutes from the computer's
clock and a confirmation more than one minute from it: confirm within a minute of
the preview, or preview again, and correct the system clock first if it is wrong.
Repair preserves the irreversible watermark and performs no admission or pruning.
The restriction to the CLI and the desktop applies to the MCP tool list; the
agent socket accepts the repair from any process of the same user, which can run
the CLI anyway.
`retention status` reports redacted resident maintenance counters. Maintenance runs
at agent startup and every minute on an independent single-flight lane, rotating
profiles and skipping busy profile locks without making network requests.

Protected cleanup uses complete typed ownership across generic, chat, SSO, team and
invitation records. Partial/stale inventories never authorize final-file deletion.
The private inventory is capped at 2,048 descriptors and 4 MiB per cached profile;
`inventory_saturated` reports deferral when it cannot fit. Temporary-file cleanup
remains available. Each scan examines/removes at most 256 directory entries and
shares a cooperative 50 ms maintenance deadline; individual I/O and fsync can exceed
that deadline. Unknown and terminal-owned material retain their owning contracts.

Rust hosts preserve uploads referenced by any retained directory version, including
incomplete linked uploads. Never-linked uploads become eligible after 24 hours and
are reclaimed in bounded, restartable batches. Server maintenance metrics report
failures, reclaimed bytes and deferred work. Pinned Go-host upload retention remains
that host's responsibility; these changes add no remote FOKS methods.

The current hard-state schema is 41 and the agent IPC protocol is 37. Opening a
supported schema-38, schema-39 or schema-40 database upgrades it to schema 41;
other older development schemas are rejected. Upgrade the CLI and resident agent
together, and restart the agent so their IPC versions match. Before serving, the
CLI exchanges a version-independent Hello; when a resident agent of another
version answers, it reports both versions and does not start a second agent.
Tool calls fail with `incompatible local agent` rather than an unknown outcome,
because that agent refuses each request before running it. This does not
convert legacy submission IDs or make pending operations eligible for replay.

The server advertises MCP 2025-11-25 and negotiates 2025-06-18, 2025-03-26 and
2024-11-05 through the official pinned Rust SDK. The ten upstream tool names and
their ordinary behaviors are compared with go-foks v0.1.9. Stat uses Go's KVStat
JSON shape; like Go, large-file stat reports size zero because the metadata lacks
the plaintext length. Get streams bounded chunks to EOF without scanning the
whole file to learn its size first.

Run `tools/foks-v019-oracle/run-mcp-compat.sh` from the repository to exercise the
independent Go SDK with Rust MCP against both Rust and pinned Go FOKS services.
The Go service target requires Docker. Ordinary Cargo tests require only local
test sockets.
