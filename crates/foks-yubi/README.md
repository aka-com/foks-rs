# foks-yubi

FOKS-only YubiKey provider boundary. The default feature set supplies a
deterministic in-memory provider for isolated client/server tests. The
`hardware` feature adds the macOS/Linux PC/SC PIV adapter used by `foks-rs` and
`foks-agent`; it is deliberately absent from server and desktop-process code.

The provider generates two P-256 keys in distinct empty retired-key slots,
returns a locator bound to the card name, serial, slots, both public keys, and
the derived PQ key ID, and revalidates that complete tuple on every reopen.
Crypto handles require a PIN when the slot policy does. A separate
administrative handle supports retry inspection, PIN/PUK changes, and
management-key replacement even when the PIN is blocked. PINs,
PUKs, management keys, and mock-card secrets are zeroized and redacted from
debug output.

Changing PIV retry counters necessarily resets the card PIN and PUK to their
factory values. Retry configuration is therefore accepted only as part of
initial preparation, after the provider proves every PIV key slot is empty and
before it generates either FOKS key. The provider verifies the supplied PIN and
factory management key, holds the process-wide card lock, and restores the
supplied PIN and PUK before key generation. A card removal or power loss in
that narrow window can leave one or both factory values active, but cannot
expose an enrolled PIV credential; the returned error identifies the recovery
state. No administrative API can change retry counts after enrollment.

Hardware operations are serialized process-wide because PC/SC APDUs are
stateful. A partially failed two-slot preparation can leave one slot
changed; the caller must report that condition and require explicit card
cleanup rather than retrying into a different slot layout. No API infers a
state path from the user's home directory or performs network I/O.

macOS provides PC/SC. Linux builds need pcsc-lite development files and runtime
access to a PC/SC daemon and the physical token. Normal library and test builds
do not enable the hardware feature.
