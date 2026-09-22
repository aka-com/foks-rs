# foks-crypto

`foks-crypto` implements cryptographic conventions used by the native FOKS
v0.1.9 client:

- SHA-512/256 prefixed hashes;
- 8-byte big-endian Snowpack type IDs;
- Ed25519 verification using public keys embedded in EntityIDs; and
- typed `Future(T)` blob signature verification;
- device Ed25519, X25519, and ML-KEM-768 derivation from the master seed;
- NaCl-compatible X25519 precomputation and hybrid SHA3 key derivation; and
- XSalsa20-Poly1305 PUK/PTK unboxing with receiver, host, generation, role, and
  public-key binding checks;
- exact account, device, PUK, and ad-hoc-team mutation construction;
- v0.1.9 Argon2id stretching, PPE enrollment/reboxing, and challenge login;
- authenticated KV name, directory-entry, and content encryption;
- context-bound Basic and extension chat encryption fixtures;
- invitation certificate/request signing and verification;
- OIDC binding signatures and server token-envelope helpers; and
- Go-compatible bot-token key derivation and authentication.

Higher-level host-chain policy and SQLite pinning live in `foks-verify` and
`foks-client-db` respectively.
