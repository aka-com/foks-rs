# foks-client-app

Application composition for standalone Rust FOKS clients. It owns explicit
profiles, per-profile state paths, encrypted account/team records, protocol
capability gates, scheduling integration, and the direct operations shared by
the CLI and local agent. It does not depend on AKA.

Each profile lives below a caller-supplied state directory and receives its own
hard-state SQLite database, soft KV cache, mutation journal, and encrypted
credential directory. Profiles can trust WebPKI or an explicitly supplied DER
root. Protocol policy defaults to the implemented v0.1.9 surface. A current
server can be probed without enabling mutations. Non-probe features require a
signed, target-bound, expiring compatibility artifact; drift artifacts and
lease expiry fail closed. Current profiles also pin the stable HTTPS lease URL
and signer. A lease grants only when its signed protocol digest exactly equals
the v0.1.9 metadata embedded in this client. Higher-generation drift,
unrecognized-capability, or metadata-mismatch artifacts immediately return the
profile to probe-only; older or conflicting same-generation artifacts are
rejected as replay.

Registry mutations reload and merge under a state-root cross-process lock, so
independent CLI and agent processes cannot publish stale snapshots over one
another. Profile operations use separate checked operation and scheduler locks;
periodic scheduling uses nonblocking acquisition and skips contended profiles.

The implemented application slice covers:

- probe/pin, software signup with optional standard or multi-use invites and
  an optional PPE passphrase, resume, and user refresh;
- authenticated passphrase set/change and public-challenge verification;
- personal KV list/read/write/mkdir/remove with streamed file I/O;
- durable refresh jobs and bounded retry state;
- owner software-device provisioning and resume;
- backup enrollment and owner recovery with pre-submit durable secrets; and
- named/ad-hoc team creation, resume, PTK-protected local records, team sync,
  and team-KV root creation.

Most provisioning and recovery inputs that contain long-lived secrets stay in
the direct application/CLI boundary. Software signup and the PPE passphrase
lifecycle deliberately cross the private local-agent protocol: their framed
buffers and secret strings are zeroized, and the agent generates or opens
long-term credential material inside the checked session so the desktop never
opens the credential store directly. Current-server profiles require an
explicit `passphrases` capability; the hosted canary must not grant it until a
scheduled canary actually exercises these mutations and reads.

The crate is split by ownership boundary: `checkpoint` owns native credentials
and rollback enrollment, `registry` owns profiles and compatibility policy,
`account`, `team`, and `kv` own their respective workflows and protected
records, and `runtime` owns process locks and scheduled execution. These are
internal modules; the crate's existing public API remains the frontend seam.

Tests use temporary state roots and loopback fixtures. No API infers an AKA or
user-data path.

## Build and platform boundary

The workspace builds these crates with stable Rust/Cargo (the current gate was
run with Rust 1.95). The FOKS client graph builds bundled SQLite and AWS-LC, so
a native C toolchain and CMake are build prerequisites even though neither is a
runtime shared-library dependency. Normal builds require no Go, Node, webview,
system SQLite, or AKA toolchain. Go is used only by the optional pinned-upstream
protocol/oracle audits.

The direct application, CLI, agent, and GPUI desktop are exercised on macOS and
Linux. Windows is not a release target. macOS uses Keychain and runtime Metal
shaders; Linux uses Secret Service plus Wayland/X11. Native desktop graphics
add their documented platform development packages, but remain outside every
protocol and storage crate.
