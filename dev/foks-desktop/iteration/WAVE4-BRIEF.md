# Wave 4 brief — remixes driven by the audits

Read `BRIEF.md` first (rules, fixture, file format). This brief adds what the
wave 3 audits and the source checks changed. Every wave 4 mock must honour it.

## 1. Facts corrected since BRIEF.md

| Topic | What is true on this branch | What to draw |
| --- | --- | --- |
| First run | `InitializeState`, `AddProfile`, `RemoveProfile`, `ResetHardState` are agent operations. GAPS #17 is stale. | First run is fully in-app. No CLI step. |
| Device pairing | `StartDevicePairing`, `RepublishDevicePairing`, `FinishDevicePairing`, `AcceptDevicePairing`, `ResumeDevicePairingAcceptance` exist in the agent; the desktop never calls them. | "Set up another Mac from this one" is wire-backed. Draw it. Say the desktop does not expose it yet only if the mock is about what ships. |
| Adding a member | `AddTeamMember { username, role: Member \| Admin \| Owner, visibility }`. The desktop *chooses* to default to Member; the protocol allows any role. `DemoteTeamMember` only lowers; there is no promote-in-place. | Let the add sheet pick Member / Admin / Owner. Change role only demotes; say a promotion is done by removing and re-adding (rekeys) or is not offered. |
| Visibility band | `Member(-0x4000) < Member(0) < Admin < Owner`. Higher visibility is more; 0 is the default; negative bands are more restricted. An item whose read role is Member · N is readable by Members at N or above, and by Admins and Owners. | Never say "0 is the widest". Say "0 is the default; lower numbers see less". |
| Team discovery | `ListTeams` reads only aliases in the local vault; `put_team` is called only by team creation and federation admission. There is no agent operation that discovers a team someone else added you to. | The Waiting screen's "Check now" needs new agent work. Draw it under a PROPOSED band: "Proposed — needs protocol work: an operation that walks your membership chain on the server and stores the team locally". |
| Probe acceptance | `ProbeOutcome.acceptance` (Inserted / Advanced / Unchanged) exists below the agent and is dropped before `ProbeReport`. | "New pin / same as before / advanced" after a probe is PROPOSED (GAPS §26 calls it two lines). Out-of-band comparison is a local comparison only; say so. |
| Reset hard state | Also discards every resumable (journaled) operation on that profile, because the hard-state database holds the records Resume reads. | The reset copy must say: "…and every half-finished operation you could still have resumed on this server." |
| `locally_manageable` | Means "this party is a local user". It does not mean "managed on another server". | For a federated team party: "an admitted team; change it from this team's Federation page". For a non-local user: "not a user of this server; cannot be changed from here". |
| Federation roles | The agent refuses Admin and Owner as federation destination roles. | Admit sheet offers Member (with visibility) only, and says why. |
| Rollback vs lease | Not machine-distinguishable until v2 typed errors (plan §5 group E). | Any rollback banner carries a small "gated: plan §5 group E" tag. |
| Team-store writes | Deferred (plan §5 group B). Account-store writes work, Owner/Owner. | Draw the target flow live under ONE amber "Deferred — plan §5 group B" band that also names what works today. Do not disable the whole flow. |
| Backup phrase | The shipped app returns the 17 words into the JSON panel. | A write-it-down sheet with a confirmation checkbox and no copy button. |

## 2. What the auditors asked for, verbatim where it matters

- **Priya**: a "Who can read" and "Who can change" control on New secret,
  greyed with "coming later" and the command that does it today; Teams in the
  sidebar and Admin pickable when adding; a "Send to a new hire" button that
  produces install link + server address + "create your account with username
  firstname.lastname" + "then tell Priya your username"; Invite marked
  optional with where to get one.
- **Marcus**: drag a file into the family folder, and if not yet, one line on
  the folder itself saying so and where to put things meanwhile; one box
  "Group name", one button, and "Groups" not "Parties"; a "Set up my other
  Mac" button in the app; one line under every document: "Stored in Household
  on foks.example.net — not on this Mac. Download to open."
- **Jun**: a "What can deploy-bot read?" view on the party row and inside the
  Remove sheet, working for parties you cannot manage; read-role choice on
  New item (disabled + `deferred` if not yet); a "Connect an agent" panel with
  the exact `foks` command, the account alias the bot needs on its own host,
  and one line saying this Mac's agent socket is not it.
- **Ade**: after a probe, one line saying new / same / advanced (PROPOSED) and
  a paste field for the fingerprint the other admin gave; every destructive
  button somewhere it cannot be hit by accident, with blast radius spelled
  out (reset kills resumables); one Federation page per team listing what was
  admitted, from which host, at which role, active or not, and how to take it
  back or the sentence that you cannot from here.
- **Sol**: first screen asks "Did someone invite you, or are you setting this
  up yourself?" and if invited says you need the server address from them,
  they add you by the username you pick next, you cannot add yourself; a
  Waiting page ("Waiting for sam.ortiz to add sol on foks.acme-corp.com",
  Copy, Check now, "Engineering will show up here as a group above
  Personal") which also replaces the empty Notifications; Create team, Reset
  hard state and raw JSON off the day-one path; danger not the same blue as
  Add server.

## 3. Cross-cutting rules for wave 4

1. **Vocabulary.** Inside the window use: Items, Personal, Teams (never
   Parties as a destination), people and teams (parties only in the roster
   table header, once, with "a party is a person or a team"), Servers,
   Settings, "your account on <server>" (alias only in Settings), Show /
   Hide (reveal only in a tooltip), "check the server" (probe in parentheses),
   "server check-in" (lease in parentheses), "Someone else changed this
   first" (conflict). Technical words survive in a collapsed Details or an
   Inspect disclosure.
2. **Locality line.** Every File shows "Stored in <store> on <server> — not
   on this Mac. Download reads version N." Every Secret's Show says it reads
   version N from the server.
3. **Readable-by everywhere.** Every team item carries a "Readable by N ▾"
   chip computed from read role × roster; every party row carries a "Reads
   N of M" cell; both expand to the list.
4. **Danger is red, alone, at the bottom, and explains blast radius.**
   Nothing destructive shares a row or a colour with a routine action.
5. **The join story is on the first screen** and in the empty states: no
   invite links, an Admin adds you by username, here is the sentence to send,
   here is where the team will appear.
6. **Inspect, not JSON.** Raw responses live only behind "Inspect response".
7. **One deferred band per flow**, amber, naming the plan group and what
   works today. PROPOSED bands only for the two items in §1 marked PROPOSED.
8. **Reuse the established chrome**: the light stage, 1180×760 window,
   traffic lights, the review-controls strip outside the window, `?state=`
   deep links, zero console errors, scrollWidth 1280.
