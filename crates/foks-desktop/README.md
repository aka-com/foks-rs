# foks-desktop

`foks-desktop` is a GPUI application for status, profile/account selection and
software-account onboarding, personal KV, teams, federation, and scheduled work. Agent
calls run off the render thread. The account form accepts optional standard or
multi-use signup invites and an optional confirmed passphrase. The scriptable
backend accepts invites only through a private `--invite-file`, never process
arguments. The account
security form exposes passphrase set, change, and public-challenge verify; all
secret fields are masked, non-copying, and refuse system input-service text
queries (IME reconversion, dictation, press-and-hold). Secret input is bounded
(1,024 bytes for passphrases); an edit that would exceed a field's bound is
rejected whole and marked with a warning border rather than silently
truncated, and pasted trailing newlines are stripped rather than substituted.
Fields are zeroized when replaced or dropped, cleared only once a request is
dispatched, and preserved across validation errors. The agent client waits
longer than the daemon's dispatch deadline so slow operations surface the
daemon's structured deadline verdict, which the UI annotates with resume
guidance.
The same crate retains `foks-desktop-backend` as a scriptable JSON shell, while
`DesktopModel` keeps screen-to-operation logic independently unit testable.

The Teams screen can admit a remote profile's active team into a selected
local named team, list protected remote bindings, and surface scheduled
reconciliation failures. The graphical path deliberately defaults to the
member role; explicit admin/owner selection remains available in the CLI and
typed agent protocol.

Keeping this crate separate prevents GPUI and platform graphics dependencies
from entering FOKS protocol, storage, or agent crates, and it has no AKA
dependency. Onboarding and passphrase operations cross only the private typed
agent boundary; the desktop never opens credentials or SQLite directly.

macOS and Linux use private Unix sockets. macOS builds enable GPUI runtime
Metal shaders; Linux enables Wayland and X11. Packaging, signing, updates, and
local redacted crash markers are specified in
`packaging/foks-desktop/RELEASE_POLICY.md`. Windows is not a target.
