# foks-proto

`foks-proto` applies the exact FOKS v0.1.9 Snowpack schemas used by the native
client. It covers host discovery and delegation, Merkle transparency, user and
team chains, software and Yubi-backed mutations, PUK/PTK distribution, and KV
requests and encrypted objects. It also models the v0.1.9 passphrase stretch,
PPE parcel, login challenge, passphrase-annex wire values, realtime chat,
invitation certificates and requests, OIDC sessions and bindings, username
changes, and locally negotiated chat-extension capability records.

Authenticated inputs are decoded by `foks-snowpack`, schema checked here, and
re-encoded canonically without any JSON or AKA application-type translation.
Unknown versions, union tags, entity types, field counts, and fixed-size blobs
fail closed.
