# foks-proto

`foks-proto` applies the exact FOKS v0.1.9 Snowpack schemas used by the native
client. It covers host discovery and delegation, Merkle transparency, user and
team chains, software and Yubi-backed mutations, PUK/PTK distribution, and KV
requests and encrypted objects. It also models the v0.1.9 passphrase stretch,
PPE parcel, login challenge, and passphrase-annex wire values.

Authenticated inputs are decoded by `foks-snowpack`, schema checked here, and
re-encoded canonically without any JSON or AKA application-type translation.
Unknown versions, union tags, entity types, field counts, and fixed-size blobs
fail closed.
