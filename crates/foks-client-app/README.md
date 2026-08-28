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
lease expiry fail closed.

The implemented application slice covers:

- probe/pin, software signup, resume, and user refresh;
- personal KV list/read/write/mkdir/remove with streamed file I/O;
- durable refresh jobs and bounded retry state;
- owner software-device provisioning and resume;
- backup enrollment and owner recovery with pre-submit durable secrets; and
- named/ad-hoc team creation, resume, PTK-protected local records, team sync,
  and team-KV root creation.

Mutation and recovery inputs that contain secrets stay in the direct
application/CLI boundary. The local agent exposes only read, sync, and due-job
operations.

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
