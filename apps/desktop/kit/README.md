# FOKS UI kit

This directory contains reusable presentation primitives for the FOKS desktop
frontend. Kit modules may depend on React and Lucide types, but they must not
depend on application state, commands, or bridge implementations.

The kit currently provides viewport-aware menu positioning, overlays, toast
state, virtual-list calculations, icon rendering, shared light and dark appearance
tokens, and the bundled interface face (`fonts/InterVariable.woff2`, Inter v4.1,
SIL Open Font License 1.1 in `fonts/Inter-OFL.txt`). Application-specific
components and theme overrides belong under `apps/desktop/src/`.
