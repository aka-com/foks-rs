# Wave 4 Brief: Iteration Requirements Based on Persona Audits

Read `BRIEF.md` first (rules, fixture, file format). This brief adds what the
wave 3 audits and the source checks changed. Every wave 4 mock must honour it.

## 1. Facts corrected since BRIEF.md

| Topic | What is true on this branch | What to draw |
| --- | --- | --- |
| First run | `InitializeState`, `AddProfile`, `RemoveProfile`, `ResetHardState` are agent operations. GAPS #17 is stale. | First run is fully in-app. No CLI step. |
| Device pairing | `StartDevicePairing`, `RepublishDevicePairing`, `FinishDevicePairing`, `AcceptDevicePairing`, and `ResumeDevicePairingAcceptance` are implemented in the agent but not currently invoked by the desktop application. | Render the "Set up another Mac from this one" flow because it is supported by protocol operations. If depicting shipped behavior, indicate that the desktop UI does not yet expose this flow. |
| Adding a member | `AddTeamMember { username, role: Member \| Admin \| Owner, visibility }`. The desktop defaults to Member, although the protocol supports any role. `DemoteTeamMember` only lowers roles; in-place promotion is unsupported. | Allow the add member sheet to select Member, Admin, or Owner. The change role action only supports demotion; document that promotion requires removing and re-adding the member (which triggers a rekey) or is unavailable. |
| Visibility band | `Member(-0x4000) < Member(0) < Admin < Owner`. Higher visibility numbers grant broader access; 0 is the default; negative bands restrict access further. An item requiring Member at band N is readable by Members at band N or above, and by Admins and Owners. | Do not describe 0 as the widest visibility band. State that "0 is the default; lower numbers have more restricted access". |
| Team discovery | `ListTeams` reads only aliases stored in the local vault; `put_team` is invoked only during team creation and federation admission. No agent operation currently discovers teams to which external administrators have added the user. | The "Check now" action on the Waiting screen requires new agent support. Place this action under a PROPOSED banner: "Proposed — requires protocol additions: an operation to discover server memberships and store the team locally". |
| Probe acceptance | `ProbeOutcome.acceptance` (`Inserted`, `Advanced`, or `Unchanged`) is an internal protocol field omitted from `ProbeReport`. | Pin status ("New pin / same as before / advanced") following a probe is PROPOSED. Clearly indicate that out-of-band comparison is evaluated locally against stored pins. |
| Reset hard state | Resetting hard state also discards all resumable (journaled) operations for that profile because the hard-state database stores the journal records needed for resumption. | The reset confirmation dialog must state: "Discards all uncompleted operations that could otherwise be resumed on this server." |
| `locally_manageable` | Indicates that the party is a local user rather than an external user managed on another server. | For federated teams, display: "Admitted team; manage from this team's Federation page." For external users, display: "Non-local user; cannot be modified from this server." |
| Federation roles | The agent refuses Admin and Owner as federation destination roles. | Admit sheet offers Member (with visibility) only, and says why. |
| Rollback vs lease | Not machine-distinguishable until v2 typed errors (plan §5 group E). | Any rollback banner carries a small "gated: plan §5 group E" tag. |
| Team-store writes | Writing to team stores is deferred (plan §5, group B). Account-store writes function normally with Owner permissions. | Render the target flow with a single amber banner reading "Deferred — plan §5 group B" noting currently supported operations. Keep the remainder of the workflow interactive rather than disabling it entirely. |
| Backup phrase | The current production application outputs the 17-word recovery phrase directly to the JSON response panel. | Present a dedicated confirmation modal requiring the user to confirm manual recording via a checkbox, with clipboard copying disabled. |

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
