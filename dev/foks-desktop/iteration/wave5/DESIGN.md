# Wave 5 — FOKS Desktop, one coherent design

This is the synthesis of the twenty-three mocks in waves 1–4 and the five
persona audits: the best element for each part of the application, made to
agree with every other part. It is drawn as six files, two or three screens
each, on one shared design system:

```text
system.css      tokens, chrome, components — every wave 5 screen uses only these
shared.js       the corrected fixture (FX), helpers, and the chrome builders
01-start.html   First run and Home        — who is setting you up · waiting · Home
02-items.html   Items                     — catalog + login · file + readable-by · New sheet
03-teams.html   Teams                     — teams list · people & teams + party panel · sheets
04-servers.html Servers & trust           — server cards · server page with Trust · rollback + reset
05-settings.html Settings                 — Your Macs & recovery · Security keys · Account & agent
06-attention.html Attention               — inbox · resume in progress · empty
```

The shared files are the one deliberate exception to the "single
self-contained file" rule in `../BRIEF.md`: coherence across six files is the
point of this wave, and a shared stylesheet is how real applications get it.
Everything still opens from `file://` with no build step.

## 1. Decisions

### Information architecture

```text
rail, top      Home · Items · Teams · Attention (badge = items that need you)
rail, foot     Servers · Settings
Items sidebar  Stores grouped under their server; Category list beneath
Get started    a card on Home and a badge on the Home rail entry until done —
               not a rail destination of its own
```

Seven equal rail entries become four destinations and two utilities. Stores
and Parties stop being destinations: a store is a place inside Items, a party
is a row inside a team. Notifications becomes Attention because it holds
things that need you, not messages.

### Vocabulary (from the Family and Ship-first mocks, kept everywhere)

| Screen word | FOKS word, shown in parentheses or in Details |
| --- | --- |
| team, people & teams | team, party (a party may be a team) |
| your account on `server` | account alias (`personal`) shown as secondary text |
| server | profile |
| check the server / last check | probe / ProbeReport |
| server check-in | compatibility lease |
| history check | rollback checkpoint |
| Show (reads version N from the server) | reveal / ReadKv |
| not on this Mac — Download reads version N in chunks | chunked read |
| Someone else changed this first | compare-and-swap conflict |
| Deferred — plan §5 group B | team-store writes and role choice |
| Proposed — needs protocol work | anything the wire cannot do today |

Member · N, Admin and Owner keep their names. Visibility is explained once,
wherever it appears: "0 is the default; lower numbers see less."

### The rules every screen obeys

1. One primary action per surface. Destructive actions are red, alone, at the
   bottom, behind a sheet that states the blast radius; Reset also requires
   typing the server name and says it discards every resumable operation.
2. Every write names its version guard. Every server fact says when it was
   learned. Values are masked until Show, which says what it reads and from
   where. Files say where they live.
3. "Who can read this" is computed, never prose: read role × roster, shown as
   a chip that expands to names.
4. The safety states outrank the page: lease lapsed and history-check failure
   are banners across the top, and the page beneath refuses rather than
   greys.
5. Nothing appears that the wire cannot carry. Deferred and Proposed bands
   are the only fiction, and they say so.

## 2. What was chosen, per part, and from where

### Start (first run and Home)

- **Who is setting you up?** as step 1, with the invited card's three facts
  (`wave4/06`, from Sol's audit).
- The plain probe sentence + collapsed Details + out-of-band compare
  (`wave3/05`, `wave3/04`).
- **Waiting for sam.ortiz to add sol** with Copy, Check now under the
  Proposed band for team discovery, and "meanwhile, your own logins"
  (`wave3/05`, `wave4/06`).
- **Home** tiles: You · Attention · Get started · Recent in this session ·
  Your teams with the "Not seeing a team?" explainer (`wave4/01`).

### Items

- Keychain's two-list sidebar (stores under servers; categories derived
  from paths) (`wave1/02`).
- Finder's list/column toggle and path-derived folders (`wave1/01`).
- 1Password's detail card: labelled fields convention, Show, Copy
  (`wave1/03`).
- Apple Passwords' locality sentence for files, Drive's Download and chunk
  toast (`wave1/04`, `wave1/05`).
- The **Readable by N ▾** chip and the New sheet's who-can-read preview
  (`wave3/02`, `wave4/04`), the single New sheet for every kind (`wave4/04`).
- The Deferred band at the top of a team store's rows, once (`wave4/08`).

### Teams

- Team cards with your role and party avatars, the "no join button" explainer
  with your usernames (`wave2/02`).
- People & teams table with Kind, Role here, Role at source when different,
  Reads N of M, and the party panel's "What X can read" and "Connect an
  agent" (`wave3/01`, `wave4/03`).
- Add member with Member (band stepper) / Admin / Owner; Change role =
  strict demotion with the remove-and-re-add sentence; Remove with the
  rotate-these checklist (`wave2/02`, `wave3/01`).
- Federation tab with both host ids, the pin comparison, Re-run, and the
  honest Take-it-back sentence (`wave3/04`, `wave4/03`).
- Invite someone → the copyable message (`wave4/03`, from Priya's audit).

### Servers & trust

- Status-striped server cards (`wave2/04`).
- Server page: You on this server · Host facts, as of last check · Trust
  (host id, pinned on first use, out-of-band local compare, server check-in,
  history check, "Last check against the pin: Not reported" under a Proposed
  band) · Teams through this account · Danger (`wave2/04`, `wave3/04`,
  `wave4/05`).
- Rollback state: banner, inert page, typed-confirmation reset with the
  full blast radius, gated tag for plan §5 group E (`wave3/04`, `wave4/07`).

### Settings

- System Settings shape: coloured sections, inset rows, chevrons
  (`wave2/01`).
- Your Macs & recovery: this Mac, other Macs, backup phrase with the
  write-it-down sheet (checkbox, no copy), Add another Mac with pairing
  (wire-backed, "not in the desktop yet") and the phrase route (`wave3/03`,
  `wave4/05`).
- Security keys: Everyday / Recovery / Danger regrouping, enrolment sheet
  that states the three hard facts first (`wave2/01`, `wave3/04`).
- Account & agent: identity per server with passphrase actions; the agent's
  status and socket with "this is not how an agent reads values" (`wave2/04`,
  `wave3/01`).

### Attention

- The inbox: severity by stripe, consequence → mechanism → action, a source
  line naming the response the fact came from, filters, interrupted
  operations with Resume and its "verifying what was already written"
  progress, reminders with per-item ticks, the waiting card, and an empty
  state that lists what appears here (`wave4/07`, `wave3/01`, `wave3/05`).

## 3. What was rejected, and why

- **The dark operator rail** (app as built, `wave2/05`, `wave4/08`): it reads
  as an admin console; every consumer reference uses a light sidebar.
  Ship-first keeps it only because it is a patch, not a design.
- **Stores and Parties as rail entries**: two auditors opened Parties and
  found a Create-team form. A store is where an item lives; a party is who is
  in a team.
- **Categories as a top-level filter with counts in the rail** (`wave1/03`):
  kept, but inside the Items sidebar, below stores, so a newcomer sees places
  before kinds.
- **A per-item share affordance** in any form: FOKS has none. Sharing is a
  place. The New sheet's Save-in picker is the share control.
- **Guided setup as a permanent left rail** (flightdeck, `mocks/`): it is a
  first-run mode and a Home card, not the application's shape.
- **The JSON response panel**: an "Inspect response" disclosure survives on
  server and team pages for operators, collapsed.

## 4. Corrected facts every screen carries

From `../WAVE4-BRIEF.md`: `Member(-0x4000) < Member(0) < Admin < Owner`;
first run is in-app; device pairing exists in the agent but not the desktop;
a team someone else adds you to cannot appear on its own today (Proposed
band); probe acceptance is dropped before ProbeReport (Proposed band); Reset
hard state discards every resumable operation on that server; `AddTeamMember`
takes any role; `RemoveTeamMember` takes a username, so an admitted team
cannot be un-admitted from this Mac; no command writes a team item today;
rollback and capability-denied are not machine-distinguishable until plan §5
group E.
