# foks-proto

`foks-proto` applies the exact FOKS v0.1.9 Snowpack schemas needed for public
host discovery and authenticated-user synchronization. It covers probe
responses, signed future blobs, host-chain links and changes, Ed25519/ECDSA
signature containers, public service zones, v1 Merkle roots and back-pointers,
user group-change links, both compressed Merkle terminal forms, HEPKs, hybrid
PUK parcels, and shared-key cleartexts.

Authenticated inputs are decoded by `foks-snowpack`, schema checked here, and
re-encoded canonically without any JSON or AKA application-type translation.
Unknown versions, union tags, entity types, field counts, and fixed-size blobs
fail closed.
