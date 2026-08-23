# foks-crypto

`foks-crypto` implements cryptographic conventions used by the FOKS v0.1.9
public-host and authenticated-device paths:

- SHA-512/256 prefixed hashes;
- 8-byte big-endian Snowpack type IDs;
- Ed25519 verification using public keys embedded in EntityIDs; and
- typed `Future(T)` blob signature verification;
- device Ed25519, X25519, and ML-KEM-768 derivation from the master seed;
- NaCl-compatible X25519 precomputation and hybrid SHA3 key swizzling; and
- XSalsa20-Poly1305 PUK unboxing with receiver, host, generation, role, and
  public-PUK binding checks.

Higher-level host-chain policy and SQLite pinning live in `foks-verify` and
`foks-client-db` respectively.
