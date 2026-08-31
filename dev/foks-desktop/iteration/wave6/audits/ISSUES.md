# Round-two audit findings against wave 6 — consolidated

Five personas re-enacted their goals against `wave6/01-vault.html` and
`wave6/02-first-run.html` (reports in this directory, one per persona, each
with a "then and now" against their round-one frustrations). This file
merges their remaining-issue tables into themes, deduplicated, with the
severity the personas gave, whether the fix needs protocol work, and the
implementation-plan phase it belongs to (`../../IMPLEMENTATION-PLAN.md`).

## Verdicts

| Persona | Would use it? | In one line |
| --- | --- | --- |
| Priya, engineering lead | Yes, the day a Who-can-read control ships | Roster, roles and readable-by are better than 1Password Teams; cannot create the one secret the exercise was about |
| Marcus, household | Yes for reading; not yet for the family | Finally says where the PDF lives and who can open it; cannot add to Household; sent twice to a page that does not exist |
| Jun, agent engineer | Yes as a personal vault; not for the job | Reads a store better than anything else here; no way to tell a program how to fetch a value, no way to take a bot's access away |
| Ade, self-hoster | Yes as a password manager; cannot run a server on it | Keeps the compare-and-pin screen verbatim; cannot add a second server; Federation page pointed at but absent; no fork state |
| Sol, first-timer | Yes | Every question from round one is answered on the screen where it arises; the only thing in the way is the honest "cannot see the group yet" band |

Counts across the five tables: 46 issues, of which 6 blockers, 17 major,
23 minor. Only one blocker and one major need protocol work; everything
else is desktop-side and most of it is "carry the wave 5 screen into the
wave 6 shell".

## Blockers

| # | Theme | Raised by | Needs protocol? | Plan phase |
| - | ----- | --------- | --------------- | ---------- |
| B1 | The New sheet has no Who-can-read / Who-can-change control, not even greyed | Priya 1, Jun 4 | Live: yes (group B). Drawn greyed with the reason: no | Phase 3 (greyed), Phase 7.6 (live) |
| B2 | A group someone else added you to cannot appear; the Waiting screen's Check now can never succeed | Sol 1 | Yes (team discovery) | Phase 7.2; Phase 5 for the interim copy |
| B3 | No Remove on a party row and no rekey / rotate-these sentence; a bot's access cannot be taken away from the roster | Jun 1 | No | Phase 4 |
| B4 | Nothing says how a program on another host obtains a value (no `foks` command, no "the bot needs its own account", no "this Mac's agent is not it") | Jun 2 | No | Phase 4 (party panel) |
| B5 | Servers & devices cannot add, check, compare or pin a second server; host ids are neither full nor copyable | Ade 1 | No | Phase 6 |
| B6 | "Set up another Mac", backup phrase and recovery are pointed at from first run and Done but do not exist in Settings or Servers & devices | Marcus 1, Marcus 2, Priya 7, Ade 7, Sol 7 | No (pairing operations exist) | Phase 6 |

## Themes

| # | Theme | Issues | Severity | Needs protocol? | Plan phase | Proposed action |
| - | ----- | ------ | -------- | --------------- | ---------- | --------------- |
| T1 | **Team writes and the read-role control** | Priya 1, 4; Jun 4; Marcus 3 | blocker, major | Live control: yes. Everything else: no | 3, 7.6 | Draw Who-can-read / Who-can-change on the New sheet greyed with "comes with group writes"; show the read role a created item gets; add "no command writes a group item today either" to the band; put the one-line "you can read what's in here; adding comes later" at the top of every group page |
| T2 | **Team discovery and the Waiting screen** | Sol 1, 2, 3, 4 | blocker, major, major, minor | Discovery: yes. Copy and placement: no | 5, 7.2 | Move the Proposed band below Check now and phrase it as what is coming; say whether reopening FOKS checks by itself; stop repeating the band once added; name the one thing that works meanwhile |
| T3 | **Machine identities and revocation** | Jun 1, 2, 5, 6, 7 | blocker ×2, major, minor ×2 | No | 4 | Carry wave 5's party panel: Remove on the row with the rekey sentence and the rotate-these list; "what this party can read" per party with a reason per line; Connect an agent with the command and the account the bot needs; party id shortened with Copy; generation with one line on what it means |
| T4 | **Federation surface** | Priya 5; Ade 2, 10; Jun 9 | major ×2, minor ×2 | Un-admit: yes. The page: no | 4, 7.4 | Carry wave 5's Federation tab: what was admitted, from which host, at which role, active or not, Re-run with its cost, and the honest take-it-back sentence; stop pointing at a page that is not there |
| T5 | **Servers, devices and settings are too thin** | Ade 1, 4, 5, 6, 8, 9; Marcus 1, 2; Priya 7; Sol 7 | blocker ×2, major ×4, minor ×4 | No | 6 | Carry wave 5's Servers page (server rows with Check, full copyable host id, the compare box, lease, Add a server, Forget, the typed-confirmation Reset with resumables), the rollback state with its gated tag, Your Macs & recovery (pairing and the phrase), Security keys (alias and server on the enrolled row, serial on the connected row, enrolment sheet); make "Set up again" say what it found |
| T6 | **Reader counting semantics** | Jun 3, 8; Ade 3 | major ×2, minor | No | 1 (pure function), 2 | An inactive admission is not a reader: show it struck through with "admission inactive, reads nothing here"; count people and groups separately ("5 people · 1 group"); `readers_of` excludes inactive admissions |
| T7 | **Invite message on the owner's side** | Priya 2; Marcus 4; Sol 8 | major ×2, minor | No | 4 | Bring wave 5's Invite someone: the written message (install link, server address, "username firstname.lastname", "tell me your username") with Copy, on Join or create a group and in the Manage sheet |
| T8 | **Vocabulary leaks and unproven claims** | Priya 6, 8; Marcus 5, 6, 7; Jun 10 | minor ×6 | No | 0 | Reword the sanctioned personal-store copy to group / people (BRIEF §2 allows a group-flavoured variant); explain or replace journaled, version guard, unlinks, alias, ad-hoc, epoch / chain / host id; drop the plan number from the band or move it after a dash; remove "Admin cannot change other admins" and "in one signed step" (two operations); Inspect shows roles in the wire shape |
| T9 | **First-run polish** | Sol 5, 6, 7; Marcus 8, 9; Ade 7 | minor ×5, major | No | 5 | "Nothing to compare? That's normal" under the compare field; fold the YubiKey warnings behind "What to know first"; the route that works (Recover) is the primary, pairing is plain until the desktop calls it; a third card "I already use FOKS on another Mac" on the first question; Download says where the copy lands |
| T10 | **Landing identity** | Sol 9 | minor | No | 5 | "Open your vault" lands in the vault the setup built (Personal and Engineering, as sol), not the fixture owner's |

## Index of every issue

| Persona · # | Severity | Theme |
| --- | --- | --- |
| Priya 1 | blocker | T1 |
| Priya 2 | major | T7 |
| Priya 3 | major | T3 (reads N of M per person) |
| Priya 4 | major | T1 |
| Priya 5 | major | T4 |
| Priya 6 | minor | T8 |
| Priya 7 | minor | T5 |
| Priya 8 | minor | T8 |
| Marcus 1 | blocker | T5 |
| Marcus 2 | blocker | T5 |
| Marcus 3 | major | T1 |
| Marcus 4 | major | T7 |
| Marcus 5 | major | T8 |
| Marcus 6 | minor | T8 |
| Marcus 7 | minor | T8 |
| Marcus 8 | minor | T9 |
| Marcus 9 | minor | T9 |
| Jun 1 | blocker | T3 |
| Jun 2 | blocker | T3 |
| Jun 3 | major | T6 |
| Jun 4 | major | T1 |
| Jun 5 | major | T3 |
| Jun 6 | minor | T3 |
| Jun 7 | minor | T3 |
| Jun 8 | minor | T6 |
| Jun 9 | minor | T4 |
| Jun 10 | minor | T8 |
| Ade 1 | blocker | T5 |
| Ade 2 | blocker | T4 |
| Ade 3 | major | T6 |
| Ade 4 | major | T5 |
| Ade 5 | major | T5 |
| Ade 6 | major | T5 (rollback and reset) |
| Ade 7 | major | T9 / T5 |
| Ade 8 | minor | T5 |
| Ade 9 | minor | T5 |
| Ade 10 | minor | T4 |
| Sol 1 | blocker | T2 |
| Sol 2 | major | T2 |
| Sol 3 | major | T2 |
| Sol 4 | minor | T2 |
| Sol 5 | minor | T9 |
| Sol 6 | minor | T9 |
| Sol 7 | minor | T9 / T5 |
| Sol 8 | minor | T7 |
| Sol 9 | minor | T10 |

## What the round says about the combination

The vault itself is done: every persona reads a store, finds an item, and
understands who can read it. What the combination lost by taking only the
Items and first-run screens from the design set is the rest of wave 5 —
the Teams page (party panel, Invite someone, Federation), the Servers page
(Trust, Reset) and Settings (Your Macs, Security keys) — and four of the
six blockers are exactly those absences. The recommended next step for
the mocks is to carry those wave 5 screens into the wave 6 shell
(`shell.css` / `shell.js`) as `03-groups.html`, `04-servers.html` and
`05-settings.html`; for the application, the phases already do this.

## Status after the ports

The three wave 5 screens were carried onto `shell.css` / `shell.js` as
`wave6/03-groups.html` (14 states), `wave6/04-servers.html` (8 states) and
`wave6/05-settings.html` (7 states, plus `?state=macs&account=work`), and
`01-vault.html` (23 → 26 states: `new-group`, `invite`, `party-remove`) and
`02-first-run.html` (17 states × 2 paths) took the round-two copy fixes.
Every claim below was checked by opening the named file at the named state,
not from the commit messages.

### Blockers

| # | Status | Closed by |
| - | ------ | --------- |
| B1 | Partly | `01-vault.html?state=new-group` draws **Who can read** and **Who can change** greyed with a `comes with group writes` chip and a computed "would be readable by **N of M**" naming the readers; the live control still needs group writes (Phase 7.6) |
| B2 | Open | Only the copy moved (see T2). Discovery itself stays Proposed — decided, not overlooked |
| B3 | Closed | `03-groups.html?state=remove` (rekey sentence + the rotate-these list from `readersOf`) and `?state=party`; `01-vault.html?state=party-remove` carries the short version |
| B4 | Closed | `03-groups.html?state=party`: **Connect an agent** with `foks kv get … --team`, "its own account on <server> and its own device credentials there", and "this Mac's agent socket is not how an agent reads values" |
| B5 | Closed | `04-servers.html` — `list` (cards with Check), `add`, `unprobed`, `check`, `server` (full 66-hex id behind Show full · Copy, the local out-of-band compare), `lapsed`, `rollback`, `reset` |
| B6 | Closed | `05-settings.html?state=macs` (Your Macs & recovery, both routes, Recover, Forget a Mac under a Proposed band) and `?state=phrase` (17 words, shown once, no copy) |

### Themes

| # | Status | Where it landed | What is still open |
| - | ------ | --------------- | ------------------ |
| T1 | Partly | `01-vault.html?state=new-group` (greyed pair + computed preview, the amber band now saying no command writes a group item either); the one-line "You can read what's in here" band renders once at the top of every group page (`?state=group`) | The live read/write-role control — group writes, Phase 7.6 |
| T2 | Partly | `02-first-run.html?state=waiting`: the Proposed band sits below **Check now** and reads as what is coming (`PROPOSED_DISCOVER`), and the card answers when to check — "Reopening FOKS checks again; nothing runs while it is closed … never on its own in the background"; `?state=added` does not repeat the band, and Waiting names the one thing that works (the sentence to send, with Copy) | Team discovery itself — an agent operation over the existing client call. **Proposed by decision**, Phase 7.2 |
| T3 | Closed | `03-groups.html?state=party` (what this party can read, a reason per line, party id with Copy, generation), `?state=remove`, `?state=demote`; `01-vault.html` Manage carries the id, the generation and Remove | — |
| T4 | Partly | `03-groups.html?state=federation` (what was admitted, from which host, at which role, active or not, Re-run with its cost and the journaled operation id) and `?state=admit`; nothing points at a page that is not there any more | Un-admit: `RemoveTeamMember` names a username and an admitted group has none, so the honest sentence stands in for the control (Phase 7.4) |
| T5 | Closed | `04-servers.html` (8 states) and `05-settings.html` (7 states): Trust, the compare, Add, Forget, the typed-confirmation Reset naming the resumable it discards, Your Macs, Security keys with alias and server on the enrolled row and the serial only on the connected row, the enrolment sheet, and **Set up again** saying each step reports what it finds | One deliberate deviation: the history-check failure (`?state=rollback`) is drawn as fact with **no** gated tag — `readiness/PROTOCOL-READINESS.md` §1 finds typed errors do exist, so the tag this table asked for would be untrue |
| T6 | Closed | `shell.js`: `admissionActive()`, `readersOf()` excluding an inactive admission, `peopleGroups()`; `03-groups.html?state=people` shows homelab struck through with "admission inactive · reads nothing here" and every **Reads N of 4** computed | — |
| T7 | Closed | `03-groups.html?state=invite` (the four-step message, install link, server address, username shape, "tell me your username", with Copy) and `01-vault.html?state=invite`; `?state=join` carries **Copy the sentence** | — |
| T8 | Closed | Personal sharing copy says group / people, never team or parties (`PERSONAL_FIXED`); journaled, unlinks, version guard, ad-hoc and alias are glossed where they appear; the plan number lives in a `title` attribute, not visible copy; "Admin cannot change other admins" is gone from every wave 6 file; a promotion is "remove and re-add, two operations, and it rekeys"; Inspect emits `read_role` in the wire shape via `wireRole()` | — |
| T9 | Closed | `02-first-run.html`: `?state=checked` "Nothing to compare? That's normal — carry on", `?state=protect` folds the YubiKey warnings behind **What to know first**, `?state=existing` makes **Recover** the primary and leaves pairing plain, `?state=who` adds the third card "I already use FOKS on another Mac", and Download says it saves to your Downloads folder | — |
| T10 | Closed | `02-first-run.html?state=added` / `?state=done`: **Open your vault** lands in the vault the setup built — sol's Personal and the group — not the fixture owner's | — |

Four of the six blockers (B3–B6) are closed by the ports, B1 is closed as far
as it can be without group writes, and B2 is open by decision. Two items need
protocol work and nothing else: **team discovery** (T2 / B2, Phase 7.2) and
**un-admit** (T4, Phase 7.4); a third, the live read/write-role control (T1 /
B1, Phase 7.6), waits on group writes.
