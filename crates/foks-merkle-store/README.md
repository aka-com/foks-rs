# foks-merkle-store

Storage-neutral, deterministic construction and proof generation for the
compressed authenticated tree used by FOKS v0.1.9. The implementation rebuilds
a canonical tree from immutable leaves for each prepared commit. That is an
intentional v1 tradeoff: updates are O(n), while persistence, proof behavior,
and future incremental implementations remain behind small node-store traits.

The crate has no SQL, sockets, signing keys, or application dependency.
