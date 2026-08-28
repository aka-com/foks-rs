# foks-desktop

`foks-desktop` is a GPUI application for status, profile/account selection,
personal KV, teams, and scheduled work. Agent calls run off the render thread.
The same crate retains `foks-desktop-backend` as a scriptable JSON shell, while
`DesktopModel` keeps screen-to-operation logic independently unit testable.

Keeping this crate separate prevents GPUI and platform graphics dependencies
from entering FOKS protocol, storage, or agent crates, and it has no AKA
dependency. Onboarding and secret-bearing mutations remain in the tightly
scoped direct client. The desktop never opens credentials or SQLite directly.

macOS and Linux use private Unix sockets. macOS builds enable GPUI runtime
Metal shaders; Linux enables Wayland and X11. Packaging, signing, updates, and
local redacted crash markers are specified in
`packaging/foks-desktop/RELEASE_POLICY.md`. Windows is not a target.
