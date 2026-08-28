# foks-desktop

`foks-desktop` is a GPUI application for status, profile/account selection and
software-account onboarding, personal KV, teams, and scheduled work. Agent
calls run off the render thread. The account form accepts optional standard or
multi-use signup invites and an optional confirmed passphrase. The account
security form exposes passphrase set, change, and public-challenge verify; all
secret fields are masked, non-copying, bounded to 1,024 bytes, zeroized when
replaced or dropped, and consumed before an agent request.
The same crate retains `foks-desktop-backend` as a scriptable JSON shell, while
`DesktopModel` keeps screen-to-operation logic independently unit testable.

Keeping this crate separate prevents GPUI and platform graphics dependencies
from entering FOKS protocol, storage, or agent crates, and it has no AKA
dependency. Onboarding and passphrase operations cross only the private typed
agent boundary; the desktop never opens credentials or SQLite directly.

macOS and Linux use private Unix sockets. macOS builds enable GPUI runtime
Metal shaders; Linux enables Wayland and X11. Packaging, signing, updates, and
local redacted crash markers are specified in
`packaging/foks-desktop/RELEASE_POLICY.md`. Windows is not a target.
