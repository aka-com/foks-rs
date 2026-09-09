# foks-merkle-store

Storage-neutral, deterministic construction and proof generation for the
compressed authenticated tree used by FOKS v0.1.9. The implementation rebuilds
a canonical tree from immutable leaves for each prepared commit. That is an
intentional v1 tradeoff: updates are O(n), while persistence, proof behavior,
and future incremental implementations remain behind small node-store traits.

The crate has no SQL, sockets, signing keys, or application dependency.

## go-foks v0.1.9 epoch ceiling

The v0.1.9 protocol cannot mint or verify Merkle epoch 65,536. That epoch's
skip list is the first with 16 entries: go-codec emits it as `array16`, but
go-foks v0.1.9's signable validator rejects `array16` lengths that fit in a
fixarray. Because epoch 65,536 cannot be published, a compatible tree cannot
advance to later epochs either.

`back_pointer_hash` returns `EpochLimitExceeded` before encoding this list. The
server preserves that detail as a `MERKLE_VERIFY_ERROR`; it does not emit an
alternate hash that v0.1.9 peers would reject. Raising the ceiling requires a
coordinated upstream canonicality/wire fix.
