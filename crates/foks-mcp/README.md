# FOKS MCP

Run the KV or team MCP server over stdio using an existing Fennec account:

```sh
cargo build --locked --release -p foks-cli -p foks-agent
target/release/foks-rs --state-dir /absolute/client/state mcp kv \
  --profile local --account personal
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
      "args": ["--state-dir", "/absolute/client/state", "mcp", "kv", "--profile", "local", "--account", "personal"]
    },
    "foks-team": {
      "command": "/absolute/bin/foks-rs",
      "args": ["--state-dir", "/absolute/client/state", "mcp", "team", "--profile", "local", "--account", "personal"]
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

Supply optional `fennec_submission_id` on a write to retain an explicit intent
across client restarts. It must be 32 lowercase hexadecimal characters. Reuse it
only with identical semantic inputs, including the selected team. Changed inputs
are rejected. A JSON-RPC request ID is not a durable submission ID. Without the
optional ID, each explicit new write call creates a new intent.

Normal KV mode adds two recovery tools:

- `fennec_status({"fennec_submission_id":"…"})` reads/reconciles a recorded intent.
- `fennec_pending({})` lists pending intents, including handles whose original
  preparation or result reply was lost.

Prepared means remote delivery has not begun; the same input and ID may execute
once. Committed means the operation has sufficient acknowledgement or exact
namespace evidence. Rejected means it cannot execute under that ID. Submission
unknown means delivery may have occurred: inspect status instead of issuing the
write with a new ID. A partial result means ancillary namespace work committed
while final completion is unproven. Cancellation does not undo remote work.

An interrupted upload may leave an unreferenced encrypted object. A lost delete
acknowledgement can remain unknown: current absence alone cannot prove which
delete committed. A recovered mkdir can report committed without reproducing its
original directory ID. None of these results automatically repeats a mutation.

## Bounds and compatibility

Limits are 4 MiB decoded file content, 8 MiB input/output frames, 1,000 catalog or
roster rows, 64 write-path components, four active calls and 16 queued calls per
process. Oversized results fail explicitly. Agent/profile limits also apply.
Each account retains at most 64 pending intents and 4,096 submission IDs. Terminal
IDs remain replay tombstones; reaching the retention limit rejects new writes
instead of silently forgetting old IDs.

The server advertises MCP 2025-11-25 and negotiates 2025-06-18, 2025-03-26 and
2024-11-05 through the official pinned Rust SDK. The ten upstream tool names and
their ordinary behaviors are compared with go-foks v0.1.9. Stat uses Go's KVStat
JSON shape; like Go, large-file stat reports size zero because the metadata lacks
the plaintext length. Get streams bounded chunks to EOF without scanning the
whole file to learn its size first.

Run `tools/foks-v019-oracle/run-mcp-compat.sh` from the repository to exercise the
independent Go SDK with Rust MCP against both Rust and pinned Go FOKS services.
The Go service target requires Docker. Ordinary Cargo tests require only local
test sockets. See [the execution record](../../MCP_EXECUTION.md) for phase reviews
and the exact scope of compatibility evidence.
