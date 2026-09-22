# foks-keystore

Small, FOKS-only secret-store boundary used by the standalone client surfaces.
It has no dependency outside the FOKS workspace.

`SecretStore` deliberately exposes only record operations. The in-memory
implementation is for tests. `EncryptedFileSecretStore` is the portable
headless adapter: it authenticates each record independently with
XChaCha20-Poly1305, rejects symlinks and unsafe names, uses private permissions,
and publishes writes by atomic rename.

`NativeCredentialStore` keeps the wrapping key and hard-state rollback
checkpoint outside the application directory. The standalone client combines
its logical records in one versioned generic-password item in the macOS
Keychain or one encrypted-session item in the Linux Secret Service default
collection. The raw 32-byte master-key file helpers remain an explicit
development/test fallback and do not provide an external rollback boundary.
Windows is intentionally outside the current platform scope.

Real native-backend coverage is opt-in because it writes temporary records to
the current login credential service. The test uses random namespaces and
removes every item before returning:

```console
cargo test --locked -p foks-keystore --test native_credential_store -- --ignored
```

On Linux this command must run inside a D-Bus session with an unlocked Secret
Service default collection. The standalone FOKS workflow provisions a private
GNOME Keyring session for this test; it never uses a developer's collection.

The crate needs stable Rust and Cargo only. Its cryptography and randomness are
Rust crates. macOS links Security.framework; Linux talks to a Secret Service
provider over the user D-Bus session. There is no Go toolchain, system SQLite
library, or desktop runtime dependency.
