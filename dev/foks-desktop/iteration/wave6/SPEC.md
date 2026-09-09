# Wave 6 — the chosen combination, implemented

Wave 6 combines the desktop shell, vault, onboarding, and management screens
into a unified application surface across Groups, Servers, and Settings.

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
| `sidebar(active, opts)` | Renders the primary navigation sidebar across vault and onboarding modes, including navigation links to Groups, Servers, and Settings. |
| `pageHeader(location, opts)` / `headerParts` | The item page header: location title, subtitle, search, toolbar |
| `readersOf(item)` | Computes authorized readers for an item based on role permissions and active roster memberships, excluding inactive federated admissions. Centralizes reader calculations across all UI badges and lists. |
| `admissionActive(party, storeId)` | Returns whether a federated group's admission is active, granting read access to the specified store. |
| `peopleGroups(parties)` | Formats party counts into distinct user and federated group totals (e.g., '5 people · 1 group') for navigation badges and headers. |
| `admits(held, need)` / `parseRole` / `roleRank` / `fmtRole` | Role hierarchy utilities (Member with visibility level, Admin, Owner) used to evaluate read and write permissions. |
| `stack` / `stackOf` / `avatar` / `initials` | Roster avatars and the sidebar's avatar stacks |
| `kindOf` / `kico` / `kglyph` / `rtype` | Infers and formats item types on the client per BRIEF §2 specifications. |
| `setLease(state)` | Sets server lease status to lapsed for testing lease expiration states in the UI. |
| `openSheet` / `closeSheet` / `flash` | Modal sheet presentation controls and temporary toast notification display. |
| `review(states, current, extra)` / `getState` / `setUrl` | Manages the development toolbar and URL query-state routing for UI state inspection. |

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
1280 (`<scratch>/shot.mjs`). Implementations are validated against the visual and functional specifications in this document,
followed by full regression reviews across all defined user test flows.
