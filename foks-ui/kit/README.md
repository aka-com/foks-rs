# FOKS UI kit

This directory contains reusable presentation primitives for the FOKS desktop
frontend. Kit modules may depend on React and Lucide types, but they must not
depend on application state, commands, or bridge implementations.

The kit currently provides viewport-aware menu positioning, overlays, toast
state, virtual-list calculations, icon rendering, and shared light-theme
tokens. Application-specific components and theme overrides belong under
`foks-ui/src/`.
