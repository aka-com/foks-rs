# go-foks main KV replay-result fixtures

Generated from go-foks `35dc03766234572551733c0a8cb4ac9aab0f2f11` (unreleased
v0.1.10; module version `v0.1.10-0.20261012000410-35dc03766234`) by
`tools/foks-main-oracle`:

```sh
cd tools/foks-main-oracle
FOKS_MAIN_FIXTURE_OUT=../../crates/foks-snowpack/tests/fixtures/foks-main \
  go test -run TestKVReplayResultFixtures -count=1 .
```

Without `FOKS_MAIN_FIXTURE_OUT` the same test compares its output with these
files. Every frame uses sequence number 1.

| File | Content |
| --- | --- |
| `mkdir-response-fresh.frame` | `kvMkdir` result `KVMkdirRes{wasReplay: false}` from the generated server handler |
| `mkdir-response-replay.frame` | `KVMkdirRes{wasReplay: true}` |
| `put-small-response-fresh.frame` | `kvPutSmallFileOrSymlink` result `KVPutSmallFileOrSymlinkRes{wasReplay: false}` |
| `put-small-response-replay.frame` | `KVPutSmallFileOrSymlinkRes{wasReplay: true}` |
| `void-response.frame` | The void reply a v0.1.9 server sends for both methods: the DataWrap omits `Data` |
| `mkdir-race-response.frame` | `KV_RACE_ERROR` with upstream's message for a reused directory ID |
| `put-small-race-response.frame` | `KV_RACE_ERROR` with upstream's message for a reused small-file node ID |

`SHA256SUMS` lists each file's SHA-256 in `sha256sum` format.
