# foks-rpc

Strict client-side framing for the FOKS v0.1.9 Snowpack RPC protocol.

The RPC envelope is ordinary MessagePack and uses named maps. Authenticated
FOKS objects inside it are canonical Snowpack. This crate keeps those layers
separate and returns exact result bytes for protocol verification. Request
encoders cover public probe, registration certificate retrieval, authenticated
user-chain load, and owner-role PUK retrieval, with official Go differential
fixtures for each frame.
