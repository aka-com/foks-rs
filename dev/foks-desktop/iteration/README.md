# FOKS Desktop — redesign iteration

Five waves of design mocks for `crates/foks-desktop`, exploring how the
application could work like Keybase while being usable as a password manager
and as a drive, with a settings hierarchy a newcomer can follow. Everything
here is a proposal; nothing has shipped.

Start with `BRIEF.md` (object model, wire rules, fixture, file format). Every
mock is a self-contained HTML file: open it in a browser, use the review
controls under the window, or deep-link a state with `?state=`.

```text
BRIEF.md                 the shared brief every mock follows
wave1/                   Items views — Finder, Keychain Access, 1Password, Apple Passwords, drive
wave1b/                  Variants of the Apple Passwords and Drive mocks, plus onboarding in that shell
wave2/                   Settings, Teams, Servers & identity, onboarding, evolved pills
wave3/AUDIT-BRIEF.md     the persona audit protocol
wave3/audits/            the auditors' reports, verbatim
wave3/                   mocks drawn for the personas before their reports came back
wave4/                   remixes driven by the audit reports
wave5/                   the coherent final design, two to three screens per part
wave6/                   the chosen combination implemented on one shell, and its round-two audits
DESIGN-REVIEW.md         the self-review of wave 5, and wave 6 with its audit round
IMPLEMENTATION-PLAN.md   the phased plan for taking the GPUI app to the wave 6 design
readiness/               engineering readiness assessments underpinning that plan
SUMMARY.md               findings, recommendations, what to explore next
```

Each wave's section below is filled in as the wave lands.

## Wave 1 — Items

Five ways to draw the Items view. Each keeps the same fixture and the same wire
rules, so the differences are purely structural.

| File | Borrowed from | The argument |
| --- | --- | --- |
| `wave1/01-finder.html` | macOS Finder | The KV tree is already a filesystem. Stores are locations grouped under their server, folders fall out of paths, list and column views both work, Get Info is the inspector, Quick Look is the reveal, and the status bar is honest about counts and bytes. Every write names the version it guards. |
| `wave1/02-keychain.html` | Keychain Access | A store is a keychain. Two-list sidebar (Stores over Category), a sortable five-column table, and a bottom pane whose Access tab answers "who else can see this" from the roster without prose. |
| `wave1/03-onepassword.html` | 1Password 8 | Vaults are stores; categories are derived from paths on this Mac; a secret whose content is `key: value` lines renders as labelled fields (a stated client convention). The store chip says "only you" or "shared with N parties" so nobody looks for a per-item share button. |
| `wave1/04-apple-passwords.html` | Apple Passwords | The simplest shape: All / Personal / Attention / Shared Groups. Shared Groups *are* teams, with avatar stacks and a management sheet that explains Member visibility, Admin and Owner in one line each. |
| `wave1/05-drive.html` | Dropbox / Google Drive | FOKS as a drive where secrets are small files. My drive vs Shared, a navigating breadcrumb, list and grid, a details panel with a "Who can see this" roster, and an upload toast that says nothing is committed until the final frame. |

What every wave 1 mock agreed on, independently: the server must be visible in
the sidebar because a lapsed lease is a per-server fact; reveal must name the
version it reads; sharing is a place (a team store), not a button; team-store
writes are drawn disabled and labelled deferred; and the setup path (server →
identity → team) must be reachable from the first screen.

## Wave 1b — Variants of the Apple Passwords and Drive mocks

A follow-up set answering one piece of feedback on wave 1: the Drive mock
(`wave1/05`) is visually clean and its components are good, but the app
should semantically contain passwords, resources and files-in-a-password-
manager rather than files in a drive; the Apple Passwords mock (`wave1/04`)
is too busy but has the right semantic atoms. Every variant here is
self-contained and follows `BRIEF.md`.

The semantic atoms taken from 04: All / Personal / Shared Groups (teams drawn
as groups with avatar stacks and a people count) / Attention; items are
**Password**, **Resource**, **File** or **Link** (a client-side reading of
the four node types: a Secret with a `password:` line or under `/logins` is
a Password, any other Secret is a Resource; folders are chips, not places);
the detail is Login (or Value, or a file's locality sentence) + Details +
Sharing (only you, or the group roster with a Manage link); and the Manage
group sheet with People and Add People with a role picker.

| File | What it is |
| --- | --- |
| `wave1b/01-vault-list.html` | **The primary combination**: 05's visual system carrying 04's atoms. A kind filter replaces the breadcrumb; rows carry a path chip and a Readable-by chip; the details panel holds 04's insets. |
| `wave1b/02-drive-with-groups.html` | **The minimal change**: 05 verbatim, with only the words, kinds, avatar stacks, Login inset, Sharing roster and Manage sheet swapped in, so it can be compared with 05 directly. Keeps folders as navigation. |
| `wave1b/03-passwords-calm.html` | **04, de-busied**: every atom kept, ten named sources of busyness removed (one-line rows, monochrome glyphs, one value inset, an About row and a Sharing line that expand on click). |
| `wave1b/04-card-vault.html` | **The card vault**: 05's tile grid as the primary view, grouped by vault and group, with 04's insets in the details panel. |
| `wave1b/05-resources.html` | **Resources forward**: organised by what things are (Passwords, Resources, Files, Links) before vaults and groups; a resource type chip from the path; a Use-it inset on resources with the CLI line. |
| `wave1b/06-onboarding-in-shell.html` | `wave2/03`'s six-step first run (ending on a group) baked into the vault-list shell, with the resumable Get started checklist inside the sidebar. |
| `wave1b/07-first-run-in-shell.html` | `wave4/06`'s both-paths first run (invited with the Waiting screen, or on my own) baked into the same shell, ending in the shell with Engineering appearing under Shared groups. |

The vault-list shell (01) was chosen for the two onboarding variants because
it is the cleanest combination: 05's components, 04's objects, nothing added.

## Wave 2 — Settings, teams, servers, onboarding

Five ways to make the hierarchy under the Items view legible. Today Stores,
Parties, Servers and Settings are stacked operator forms with a JSON panel and a
row of context pills.

| File | Borrowed from | The argument |
| --- | --- | --- |
| `wave2/01-system-settings.html` | macOS System Settings | One window for everything that is not an item: You, Servers, Teams, Devices & Recovery, Security Keys, Agent, About. Scope is a place you navigated to (section → row → page with a breadcrumb), not a pill you set. Thirteen YubiKey buttons become Everyday / Recovery / Danger. |
| `wave2/02-teams-hub.html` | 1Password Business, Keybase teams | Team cards, then a team page with Members / Federation / Store / Danger tabs. States plainly that there is no join button: an Admin adds you by username, and shows you the sentence to send. Change role only demotes and says so. |
| `wave2/03-onboarding.html` | Setup Assistant, 1Password first run | A six-step first run that ends on a team: Preparing this Mac → Connect a server → Create your identity → Protect it → Join or create a team → Done, with the same steps as a resumable Get started checklist inside the app. |
| `wave2/04-servers-identity.html` | Apple ID / Mail accounts, Keybase devices | A server and your identity on it are one object: "where am I signed in, as whom, is it healthy". Status-striped cards; a server page with You / Host facts (as of last probe) / Teams / Danger; the lease-lapsed and rollback states drawn loudly. |
| `wave2/05-pills-evolved.html` | the shipping app | The lowest-cost path: keep the rail and the pages, replace the pills with a scope bar of two menus, give Settings and Parties tab bars, one primary action per card, a More menu for the rest, and collapse the JSON panel into an Inspect disclosure. |

What every wave 2 mock agreed on: identity belongs at the top of the settings
hierarchy; the server's health (probed, lease, history check) must be visible
wherever the server is named; Reset hard state must stand alone, red, and
explain its blast radius; and joining a team must be explained rather than
implied, because FOKS has no invite links.

## Wave 3 — Persona audits and inferred mocks

Five simulated users walked the application and the wave 1–2 mocks
(`wave3/AUDIT-BRIEF.md`; reports verbatim under `wave3/audits/`). Three ran on
Fable, two on Opus. In parallel, five mocks were drawn for the same personas
before their reports came back.

| Persona | Report | Mock drawn for them |
| --- | --- | --- |
| Priya, engineering lead | `audits/01-priya-engineering-lead.md` | `wave3/02-share-a-secret.html` — a read-role picker whose right-hand side previews who can read the item, plus a guided team setup stack |
| Marcus, household organiser | `audits/02-marcus-household.md` | `wave3/03-family.html` — the Apple Passwords shape with every FOKS word translated, and a Your Macs page |
| Jun, agent engineer | `audits/03-jun-agent-engineer.md` | `wave3/01-agent-access.html` — a party panel computing what a machine identity can read, with revoke and rotate |
| Ade, self-hosting admin | `audits/04-ade-self-hoster-admin.md` | `wave3/04-trust.html` — one Trust page per server: host identity, lease, history check, keys, federation |
| Sol, invited first-timer | `audits/05-sol-first-time-user.md` | `wave3/05-invited.html` — the invited path including a real Waiting screen and the Added moment |

### What the audits agreed on

1. **The three real goals are blocked by deferred team-store writes.** Priya,
   Marcus and Jun cannot put anything into a team from the desktop; the only
   notice is one grey sentence in the inspector, seen after the team exists.
   Every audit asked for the target flow to be drawn with a visible
   "deferred" control and a sentence naming what works today.
2. **Nobody can answer "who can read this".** No screen sets a read role, and
   no screen lists the parties a read role admits. Jun joined the Items
   inspector against the Parties JSON by hand. The share-a-secret and
   agent-access mocks were drawn for exactly this and the auditors, seeing
   them late, named them as the right shape.
3. **The join story is missing from the app.** Nothing says you need the
   server address from the person adding you, nothing says they add you by
   username, and there is no waiting state. Parties opens on Create named
   team with a pre-filled alias, so the obvious button creates a wrong team.
4. **Destructive actions are undifferentiated.** Reset hard state sits in a
   row of four identical blue buttons beside Add server; Revoke YubiKey sits
   between Recover subkey and Change PIN; Remove member matches List roster.
5. **Jargon blocks the non-technical path**: Parties, alias (three times in
   one form), probe, lease, store, symlink, ad-hoc, visibility band,
   journaled, tombstone, Merkle epoch, compare-and-swap.
6. **Locality is never stated.** Nothing says a file is not on this Mac until
   read; an 84 MB file has the same blue Reveal button as a password.
7. **The second Mac is unanswered.** Provision owner device "creates another
   local owner credential"; the only cross-machine path drawn is 17-word
   recovery, and the app returns that phrase into the raw JSON panel.
8. **The best pieces already exist** across the mocks: the Teams hub's "no
   join button" card and role radios; the onboarding flow's copyable "Add rae
   on foks.example.net as a Member"; System Settings' YubiKey regrouping;
   Servers & identity's server cards; Apple Passwords' "the value isn't kept
   on this Mac"; Drive's "uploads to shared drives are deferred" up front.

### Errata the audits surfaced (checked against source on this branch)

- **GAPS #17 is stale.** `InitializeState`, `AddProfile`, `RemoveProfile` and
  `ResetHardState` are in `foks-agent-proto`; first run is in-app, as the app
  and the onboarding mock draw it. GAPS.md should be updated.
- **Device pairing exists in the agent and not in the desktop.**
  `StartDevicePairing` … `ResumeDevicePairingAcceptance` are in the protocol
  and wrapped in `foks-desktop/src/lib.rs`, but `gui.rs` never calls them.
  The onboarding and Family mocks' "set it up from this Mac" is wire-backed;
  BRIEF.md and GAPS.md do not mention it.
- **Visibility ordering.** `foks_proto::Role` orders by kind, then by the
  signed visibility: `Member(-0x4000) < Member(0) < Admin < Owner`. Higher
  means more; 0 is the default; negative bands are more restricted. The Apple
  Passwords mock's "0 is the widest band" is wrong; the Drive mock's
  "visibility 0 and above" is right.
- **A team someone else adds you to does not appear on its own.**
  `foks-client-app::team::list_teams` reads only the aliases in the local
  vault, and `put_team` is called only by team creation and federation
  admission. The client-side membership walk
  (`foks-client::team::authenticated_user_team_memberships`) has no caller in
  the agent. The invited path's Waiting screen cannot end today without new
  agent work. This is the single most important gap for the "get onto a team"
  goal.
- **Probe acceptance is discarded.** `ProbeOutcome.acceptance`
  (Inserted | Advanced | Unchanged) exists below the agent and is dropped
  before `ProbeReport`, so no screen can say whether a probe pinned a new host
  or matched the old one (GAPS §26). The Trust mock draws the row as
  "Not reported" under a PROPOSED band.
- **Reset hard state also discards every resumable operation** on that
  profile, because the hard-state database holds the finalizable mutation
  records the Resume actions read. No screen or mock says so yet.
- **`locally_manageable` means "is a local user"**, not "is managed on
  another server". "Managed where it lives" is a misleading gloss for a
  federated party; removing an admitted team is an operation on this team.
- **Rollback and capability-denied are not machine-distinguishable** until
  the v2 typed errors land (plan §5 group E); mocks that draw them should say
  so.
- **The federation card in the app** says CLI/agent clients "can explicitly
  select admin or owner"; the agent refuses Admin and Owner as federation
  destination roles (GAPS #26).

## Wave 4 — Remixes from the audits

Eight mocks, each combining elements of earlier waves to answer the auditors'
asks (`WAVE4-BRIEF.md` records the corrected facts and the cross-cutting rules
they follow).

| File | What it is | Combines |
| --- | --- | --- |
| `wave4/01-home.html` | Home — the first screen every day: You, Attention, Get started, Recent, Your teams; first-launch and invited variants | Finder sidebar, Apple Passwords sections, the invited Waiting card, onboarding checklist |
| `wave4/02-items.html` | Items, unified — Keychain's two-list sidebar, Finder's list/column views, 1Password's detail card, a Readable-by chip, a locality line and Download for files, the New menu and upload toast | wave1/01, 02, 03; wave3/02 |
| `wave4/03-team.html` | The team page, complete — People & teams with a Reads column and a "what this party can read" panel, Connect an agent, Invite someone, Federation with an honest Take-it-back answer, Store, Danger | wave2/02; wave3/01, 04 |
| `wave4/04-create.html` | One New sheet for every kind (Login, Key, Note, Document, SSH key, Link, Folder) with Save in, Who can read (live preview), Who can change, the upload toast and the already-exists refusal; a plain-language variant | wave3/02, 03; wave1/05 |
| `wave4/05-settings.html` | Settings, unified — System Settings shape with a Trust group per server (out-of-band compare), Your Macs & recovery (pairing and phrase), Security keys, Agent, Danger groups with the reset blast radius | wave2/01, 04; wave3/03, 04 |
| `wave4/06-first-run.html` | First run, both paths — Who is setting you up → server → account → protect → wait to be added / create a team → you're in; both checklists | wave2/03; wave3/05, 04 |
| `wave4/07-attention.html` | Attention as an inbox — cards in severity order, each consequence → mechanism → action with its source response; filters; interrupted operations with Resume; the empty state says what appears here | flightdeck's queue, wave3/01 reminders, wave3/05 waiting |
| `wave4/08-ship-first.html` | Ship-first — the smallest patch to the app as built that removes the audits' top frustrations, with a change list mapping each change to a `gui.rs` function and a size | app-surface, wave2/05 |

Findings the wave 4 authors added while drawing:

- **No command writes a team item today either.** `foks-rs kv put` takes an
  account alias and fixes both roles at Owner; the only team-store write in
  the code is the root a team gets at creation. "Deferred — plan §5 group B"
  is therefore a protocol-and-CLI gap, not a desktop one.
- **An admitted team cannot be un-admitted from this Mac.** `RemoveTeamMember`
  names a username and an admitted team has none; the CLI has the same limit.
  The client library can already select a federated roster row by party id
  and remote host, so exposing it is agent and CLI work, not a protocol change.
- **Device pairing is a short-phrase flow** (`StartDevicePairing` →
  `AcceptDevicePairing` on the other Mac → `FinishDevicePairing` here,
  journaled) that the desktop wraps but never shows.

## Wave 5 — The coherent design

`wave5/DESIGN.md` records the decisions: four destinations (Home, Items,
Teams, Attention) and two utilities (Servers, Settings); a person's
vocabulary with the FOKS word in parentheses; five rules every screen obeys;
what was chosen from each earlier wave per part, and what was rejected. The
six screen files share `wave5/system.css` and `wave5/shared.js` (the one
deliberate exception to the self-contained rule) and link to each other
through the rail, so they can be walked as one application.

| File | Screens (`?state=`) |
| --- | --- |
| `wave5/01-start.html` | `who` first-run step 1 · `waiting` the invited path's Waiting screen · `home` the everyday Home |
| `wave5/02-items.html` | `login` Personal › github.com · `file` Engineering › bundle.tar with the Deferred band, locality and Readable-by · `new` the single New sheet |
| `wave5/03-teams.html` | `teams` · `people` Engineering with deploy-bot's panel · `federation`; plus `invite`, `add`, `demote`, `remove`, `admit`, `create`, `store`, `danger`, `roster` |
| `wave5/04-servers.html` | `list` · `server` foks.example.net with Trust · `rollback`; plus `lapsed`, `add` |
| `wave5/05-settings.html` | `macs` Your Macs & recovery · `keys` Security keys · `account`; plus `phrase`, `enrol` |
| `wave5/06-attention.html` | `all` the inbox · `resume` · `empty`; plus `rollback` |

The set was self-reviewed twice (`wave5/REVIEW-PASS-1.md`,
`wave5/REVIEW-PASS-2.md`) and the outcome is written up in
`DESIGN-REVIEW.md`.

## Wave 6 — The chosen combination, implemented

After wave 1b the product owner picked the pieces: the vault-list variant as
the base, the card-vault sidebar and item page header, the vault-list New
sheet, the card-vault error layouts, and the wave 1b first run, with a
**Set up again** row below Settings that returns to the first run.
`wave6/SPEC.md` records the choice and where each piece comes from.

```text
wave6/shell.css          tokens and components extracted from wave1b/01 and wave1b/04
wave6/shell.js           the corrected fixture, the kind rule, the readers computation
                         (readersOf excludes an inactive admission; peopleGroups counts
                         people and groups apart), and the sidebar and header builders
                         all five files use
wave6/01-vault.html      Items: 26 states — all · personal · password · show · resource ·
                         file · link · group · manage · new · new-group · new-resource ·
                         new-file · new-link · grid · lease · inactive · exists · conflict ·
                         attention · join · invite · party-remove · servers · settings ·
                         agent-lost
wave6/02-first-run.html  the first run, both paths, 17 states × 2 — boot · who · address ·
                         no-address · checked · compare · error · account · existing ·
                         protect · phrase · waiting · added · create-group · done ·
                         checklist-invited · checklist-own — landing in the vault
wave6/03-groups.html     Groups (wave 5's Teams, ported): 14 states — groups · people ·
                         party · federation · store · danger · invite · add · demote ·
                         remove · admit · create · lease · inactive
wave6/04-servers.html    Servers & trust (ported): 8 states — list · server · lapsed ·
                         rollback · reset · add · unprobed · check
wave6/05-settings.html   Settings (ported): 7 states — macs · phrase · keys · enrol ·
                         account · agent · about, plus ?state=macs&account=work
wave6/audits/            the round-two persona audits against 01 and 02, consolidated in
                         ISSUES.md, with a status section marking what the ports closed
```

Two Opus agents implemented the first two files and self-reviewed once each;
the product owner's reviewer then reviewed the whole (three small fixes: the
badge at zero, the lease notice's primary, the search placeholder under a
group header). All 57 states of that pair render with zero console errors at
1280×860. The five personas from wave 3 then walked their goals again against
it; their reports are under `wave6/audits/` and their remaining issues are
consolidated in `wave6/audits/ISSUES.md`.

The three wave 5 screens were then carried onto the same shell as
`03-groups.html`, `04-servers.html` and `05-settings.html`, and `01-vault`
and `02-first-run` took the round-two copy fixes — so all five screens of the
combination now live in wave 6, and 89 states in total (26 + 17 × 2 + 14 + 8
+ 7). `wave6/audits/ISSUES.md` § "Status after the ports" marks each item
closed, partly closed or open with the file and state that closes it.

All five would use it, with conditions: the vault itself is done, and every
persona reads a store, finds an item and understands who can read it. Of the
46 remaining issues in ten themes (6 blockers, 17 major, 23 minor), only one
blocker needs protocol work — team discovery, without which the Waiting
screen's Check now can never succeed — while four of the six were the wave 5
screens the combination had not carried. Those three screens have since been
ported, closing B3–B6 outright and B1 as far as it goes without group writes;
team discovery (B2) stays Proposed by decision, and un-admit stays a sentence.
`IMPLEMENTATION-PLAN.md` maps every theme to a build phase, and `readiness/`
holds the engineering assessments that plan rests on.

## Implementation

- `IMPLEMENTATION-PLAN-TAURI.md` — the plan being executed: a second Tauri app
  (`foks-tauri/`, `foks-ui/`, shared `ui/kit/`) replacing the GPUI desktop,
  Phases 0–8 with the review corrections folded in.
- `HANDOFF.md` — how to execute Phases 2–8: what exists, how the work has been
  run, per-phase notes, decisions already made, open questions.
- `IMPLEMENTATION-PLAN.md` — the earlier GPUI-targeted plan; still the source
  for the protocol list (§3 Phase 7, P1–P11) and the gap tables (§8).
- Status: **Phases 1–8 complete** (scaffold, kit, model, shell, live read and
  guarded-write command boundaries, exact-version Show/Copy/Download/Edit/
  Remove, native streamed file ingress, error workflows, explicit-role group
  item writes, Groups, roster and federation administration, resumable first
  run, render tests, Playwright gates, the Phase 7 protocol list, Servers,
  devices, Settings, Bazel graphs,
  signed/notarized ZIP policy, Debian and portable-Linux packaging, and the
  atomic Tauri cutover). The GPUI product executable is retired; only the free
  command library and non-shipping transcript backend remain.
