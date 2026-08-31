# FOKS Desktop — design reference

The retained HTML artifact is a self-contained, interactive transcription of
the desktop surface. Open it directly in a browser; it has no build step or
external application assets.

```text
GAPS.md                  where the mocks outrun what FOKS actually supports today
app-surface.html         interactive transcription of the current GPUI application
```

## Product contract

The design follows FOKS's native hierarchy rather than introducing another
domain model:

```text
profile (a probed and pinned FOKS server)
  └── account (a local alias holding one device credential)
        ├── personal store (that account's KV tree)
        └── teams
              ├── team store (the team's KV tree)
              └── parties with Member { visibility } / Admin / Owner roles
```

There is no local store, generic container object, independent store identifier, store
kind, or server-level content role. Store identity is derived from the owning profile and
account or team. Profile-wide trust failures block every store on that profile;
store-specific failures remain local to their store.

The transcription's top-level nouns are Items, Notifications, Get started,
Stores, Parties, Servers, and Settings. Activity is absent because FOKS exposes
no activity or audit log. Team creation and resume are represented; rename and
closure remain deferred product decisions.
The first release is live-only: in-memory results may survive navigation during the
running session, but unreachable data is shown as unavailable rather than presented as an
offline replica.

The desktop and agent use local protocol v2. It adds a bootstrap-only mode driven by the
desktop, paged `ListKv` and `ListTeamKv` operations for one unified catalog, structured
errors, visible native read/write roles, version-bound large-file chunks, and mandatory
compare-and-swap preconditions for edits. The desktop launches a detached agent that
survives window closure. Up to four reads run concurrently; mutations are single-flight.
The UI shows no activity history, scheduled-job inventory, per-item sync
state, or telemetry/facts that the selected response did not carry.

## Reading order

1. `app-surface.html` — the retained interactive transcription.
2. `GAPS.md` — the capability audit and stage gates.

## Status

Proposal. Nothing here has shipped.
