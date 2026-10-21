# go-foks main realtime fixtures

Generated from go-foks `35dc03766234572551733c0a8cb4ac9aab0f2f11` (unreleased
v0.1.10; module version `v0.1.10-0.20261012000410-35dc03766234`) by
`tools/foks-main-oracle`:

```sh
cd tools/foks-main-oracle
FOKS_MAIN_FIXTURE_OUT=../../crates/foks-snowpack/tests/fixtures/foks-main \
  go test -run TestRealtimeMainFixtures -count=1 .
```

Without `FOKS_MAIN_FIXTURE_OUT` the same test compares its output with these
files. Identifiers and keys match the v0.1.9 realtime fixtures
(`../../foks-v0.1.9/realtime`), so the Rust side can open the messages. Every
frame uses sequence number 7.

| File | Content |
| --- | --- |
| `seed.bin` | Realtime key seed (bytes 0..31) |
| `channel-no-push.snowp` | `RTChannelMetadata` with `noPush = true` (14 fields) |
| `channel-push.snowp` | The same channel with `noPush = false`, still written with all 14 fields |
| `request-0-no-push.frame` | `rtNewChannel` request as `RealTimeClient` frames it, carrying `channel-no-push` |
| `response-2-no-push.frame` | `RTChannelSet` result containing that channel |
| `send-res-fresh.snowp`, `send-res-replay.snowp` | `RTSendRes` with `wasReplay` false and true |
| `response-3-fresh.frame`, `response-3-replay.frame` | The `rtSend` results carrying them |
| `status-12007.frame` | `RT_MSG_REPLAY_MISMATCH` status (no payload) |
| `pegged-{reply,reactji,edit,delete}.snowp` | `RTMsg` with a sealed pegged Reply, Reactji, Edit, and Edit with an empty body (a deletion) |
| `pegged-mismatch.snowp` | Metadata type Reactji with an Edit body arm, which `openMessage` refuses |
| `pegged-*-body.snowp` | The plaintext `RTMsgBody` sealed in each message |
| `send-pegged-*.snowp` | The `RTSendArg` that delivers each pegged message, as `client/librt` `attemptSend` builds it (`expectedPrevSeq` 0) |
| `request-3-pegged-*.frame` | The `rtSend` request carrying it, as `RealTimeClient` frames it |

`SHA256SUMS` lists each file's SHA-256 in `sha256sum` format.
