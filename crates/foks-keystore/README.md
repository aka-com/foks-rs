# foks-keystore

Small, FOKS-only secret-store boundary used by the standalone client surfaces.
It has no dependency on an `aka-*` crate.

`SecretStore` deliberately exposes only record operations. The in-memory
implementation is for tests. `EncryptedFileSecretStore` is the portable
headless adapter: it authenticates each record independently with
XChaCha20-Poly1305, rejects symlinks and unsafe names, uses private permissions,
and publishes writes by atomic rename.

`NativeCredentialStore` keeps the wrapping key and hard-state rollback
checkpoint outside the application directory: generic-password records in the
macOS Keychain and encrypted-session items in the Linux Secret Service default
collection. The raw 32-byte master-key file helpers remain an explicit
development/test fallback and do not provide an external rollback boundary.
Windows is intentionally outside the current platform scope.

The crate needs stable Rust and Cargo only. Its cryptography and randomness are
Rust crates. macOS links Security.framework; Linux talks to a Secret Service
provider over the user D-Bus session. There is no Go toolchain, system SQLite
library, or AKA runtime dependency.
