# Wave 6 — the chosen combination, implemented

The product owner picked the pieces from the wave 1b variants. This set
implements them as one application mock: a shared shell, the vault, and the
first run, linked to each other so they can be walked as one product. After
the round-two audits, wave 5's Groups, Servers and Settings screens were
carried onto the same shell, so all five screens of the product are here.

```text
shell.css         tokens and components, extracted from wave1b/01 and wave1b/04
shell.js          the fixture (BRIEF §4, with the WAVE4-BRIEF §1 corrections),
                  helpers, and the sidebar / header builders
01-vault.html     Items: All items, vaults, groups, the New sheet, errors,
                  Manage, Alerts, and the footer panes — the Servers and
                  Settings panes are now redirects into 04 and 05
02-first-run.html the first run (both paths) and the resumable checklist,
                  landing in 01-vault.html
03-groups.html    Groups: the group list, People with roster and federation,
                  the party panel with Connect an agent, Settings, and the
                  invite / add / demote / remove / admit /
                  create sheets — wave5/03-teams.html on this shell
04-servers.html   Servers & trust: server cards, the server page (You · Host
                  facts as of the last check · Trust · Groups · Danger), the
                  out-of-band compare, the lapsed and history-check pages,
                  Add a server, and Reset behind typed confirmation —
                  wave5/04-servers.html on this shell
05-settings.html  Settings: Your Macs & recovery, the 17-word phrase sheet,
                  Security keys and the enrolment sheet, Account, Agent and
                  About with Set up again — wave5/05-settings.html on this
                  shell
```

## shell.js — what the five files share

| Function | What it is for |
| --- | --- |
| `sidebar(active, opts)` | The left rail in both modes (vault and the first run's step list), including the footer rows that open `03-groups.html`, `04-servers.html` and `05-settings.html` |
| `pageHeader(location, opts)` / `headerParts` | The item page header: location title, subtitle, search, toolbar |
| `readersOf(item)` | Who can read an item: read role × roster, excluding any admission that reports inactive. Every "Readable by N of M" and every readable-by list is this function, never prose |
| `admissionActive(party, storeId)` | Whether an admitted group's admission reports active — the one thing that makes it a reader |
| `peopleGroups(parties)` | "5 people · 1 group": people and admitted groups counted apart, used by both the sidebar rows and the group page header |
| `admits(held, need)` / `parseRole` / `roleRank` / `fmtRole` | The role ordering (Member{visibility} < Admin < Owner) the reader computation rests on |
| `stack` / `stackOf` / `avatar` / `initials` | Roster avatars and the sidebar's avatar stacks |
| `kindOf` / `kico` / `kglyph` / `rtype` | The client-side kind reading (BRIEF §2) |
| `setLease(state)` | Puts a server's check-in into `lapsed` so the lease states can be seen |
| `openSheet` / `closeSheet` / `flash` | Sheets and the transient confirmation line |
| `review(states, current, extra)` / `getState` / `setUrl` | The review strip outside the window and `?state=` deep links |

## What comes from where

| Element | Source | Notes |
| --- | --- | --- |
| Overall shell, list rows, details panel, Manage sheet, footer panes | `wave1b/01-vault-list.html` | The base. Rows: kind icon, title, path chip, "vault · server" subtitle, Readable-by chip, version, hover actions. Details: Login / Value / locality / Link inset, Info, Sharing, footer Edit · Copy path · Remove. |
| Left sidebar | `wave1b/04-card-vault.html` | "All items" on top with no section label; VAULTS show their server; active GROUPS show their people/group count without a server. A blocked, unavailable, lapsed, or inactive row is dimmed, carries an amber dot, and reads "Connection error". Footer rows are Join or create a group · Servers & devices · Alerts with badge · Settings, plus **Set up again** below Settings. |
| Item page header | `wave1b/04-card-vault.html` | "All items" has no subtitle. Store titles use the sidebar description exactly: an account shows its server, an active group shows its people/group count, and a blocked, unavailable, lapsed, or inactive store reads "Connection error". Search (⌘K) is on the right; the toolbar beneath contains New, the kind control, Sort, the list/grid toggle, and the details toggle. |
| New sheet (password, resource, file, link) | `wave1b/01-vault-list.html` | Save in (vaults and groups; groups "shared with N people"), the kind's fields, one amber Deferred band when a group is chosen naming what works today. |
| Error dialogs and error message layouts | `wave1b/04-card-vault.html` | Store access uses one takeover in vault and group-management pages. Blocked, unavailable and lapsed servers show a critical notice with one working link to the server; lapsed access also shows "Waiting for renewal". Inactive groups show a warning notice with "Finish setup". Takeovers omit search, empty toolbars and eyebrows. All items keeps available content and summarizes every unavailable server or inactive group in a band. |
| Grid view | `wave1b/04-card-vault.html` | Secondary view behind the toggle. |
| First run | `wave1b/07-first-run-in-shell.html` | Both paths, the Waiting screen, the Added moment landing in the vault, both checklists. Rebuilt on this shell so the sidebar and header match 01-vault exactly. |

## Rules

Everything in `../BRIEF.md` §2 and `../WAVE4-BRIEF.md` §1 applies. In
particular: kinds are a client-side reading (a Secret with a `password:`
line or under `/logins` is a Password, any other Secret is a Resource; File
and Link are node types; folders are chips); Readable-by is computed from
read role × roster; values are masked until Show, which says what it reads
and from where; every write names its version guard; team-store writes are
deferred and drawn under one amber band; team discovery and probe acceptance
are Proposed bands; visibility is "0 is the default; lower bands see less";
one primary per surface; destructive actions red, alone, at the bottom.

## States

01-vault (26): all · personal · password · show · resource · file · link ·
group · manage · new · new-group · new-resource · new-file · new-link · grid ·
lease · inactive · exists · conflict · alerts · join · invite ·
party-remove · servers · settings · agent-lost

02-first-run (17 on each of two paths): boot · who · address · no-address ·
checked · compare · error · account · existing · protect · phrase · waiting ·
added · create-group · done · checklist-invited · checklist-own (with
`&path=invited|own`)

03-groups (14): groups · people · party · federation · store · danger ·
invite · add · demote · remove · admit · create · lease · inactive

04-servers (8): list · server · lapsed · rollback · reset · add · unprobed ·
check

05-settings (7): macs · phrase · keys · enrol · account · agent · about, plus
`?state=macs&account=work`

Eighty-nine states in all.

## Verification

Every state renders at 1280×860 with zero console errors and scrollWidth
1280 (`<scratch>/shot.mjs`). Each implementer self-reviews once against
this file and the rules before reporting; the product owner's reviewer then
reviews the whole; then the five persona audits are re-enacted against it.
