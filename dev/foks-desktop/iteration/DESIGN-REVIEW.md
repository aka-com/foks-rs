# Final design review — FOKS Desktop, wave 5

This is the sign-off review of the coherent design in `wave5/`. It records
what the design is, how it was checked, what the two review passes found and
what was done about it, and what remains open. Read `wave5/DESIGN.md` first
for the decisions themselves.

## 1. What is being reviewed

Six files, sixteen primary screens plus their sheets and states, on one shared
stylesheet and fixture:

| Part | File | Primary screens |
| --- | --- | --- |
| Start | `wave5/01-start.html` | Who is setting you up · Waiting to be added · Home |
| Items | `wave5/02-items.html` | Login detail · Team file with locality and Readable-by · New sheet (+ lease refusal) |
| Teams | `wave5/03-teams.html` | Teams list · People & teams with a party panel · Federation (+ six sheets) |
| Servers | `wave5/04-servers.html` | Server list · Server page with Trust · Rollback (+ reset sheet, lapsed, add) |
| Settings | `wave5/05-settings.html` | Your Macs & recovery · Security keys · Account & agent (+ phrase, enrol sheets) |
| Attention | `wave5/06-attention.html` | Inbox · Resume in progress · Empty (+ rollback) |

Every state renders at 1280×860 with zero console errors and no horizontal
overflow (34 deep-linked states, re-verified after each fix pass).

## 2. How it was checked

1. **Render sweep.** Every `?state=` of every file, before and after each fix
   pass, with the same headless-Chromium script used for waves 1–4.
2. **Consistency grep.** Rail badge arguments, vocabulary slips ("Parties",
   "Reveal", "probe" or "profile" outside parentheses), Deferred / Proposed /
   Gated band counts per file, hex colour and font-size literals in per-file
   styles, and the who-footer.
3. **Independent review, pass 1** (`wave5/REVIEW-PASS-1.md`, Opus): coherence
   across files, the five rules in `DESIGN.md` §1, wire honesty against
   `BRIEF.md` §2 and `WAVE4-BRIEF.md` §1, every one of the fifteen auditor
   asks, visual defects, and newcomer legibility.
4. **Fix pass** across all six files and the shared system.
5. **Independent review, pass 2** (`wave5/REVIEW-PASS-2.md`, Opus): each
   pass-1 finding verified resolved or not, regressions, and a verdict.

## 3. Pass 1 — what was found and what was done

The reviewer found ten defects and four partially answered auditor asks.
Resolution, in the reviewer's severity order:

| # | Finding | Resolution |
| - | ------- | ---------- |
| 1 | Attention's rotate reminder said deploy-bot was removed while Teams still listed it | Reminder now concerns dana.okafor, whose Remove sheet promises exactly that card; deploy-bot stays on the roster with its "what it can read" panel |
| 2 | Two different visibility-band ranges and wordings (Items ≥ 1; Teams −16384…0) | One range (`clampVis`, −16384…16383) and one wording (`FX.copy.visibility`) in `shared.js`, used by both |
| 3 | Security keys printed a serial next to the alias, a join the enrolled list cannot carry | Enrolled row shows alias and server only; serial moved to the Inspect disclosure; Connected-now keeps its serial |
| 4 | A "Join a team" button above "FOKS has no join button" | Relabelled "How to get added" |
| 5 | Home said "2 servers" over three rows and "3 things" beside a badge of 4 | You tile counts accounts and servers; the Attention tile lists exactly the four needs-you items the badge counts, everywhere |
| 6 | Items and Teams silently drew Acme's lease fresh while the fixture said lapsed | The fixture is healthy by default; a screen that draws the lapsed world calls `setLease("lapsed")` and says so in its review note; Items gained a `lease` state for the refusal |
| 7 | Selected table rows lost their secondary text to dark-on-blue | `system.css` keeps `.sub`, `.faint`, `.muted`, `.tiny` legible on selected rows |
| 8 | "profile" escaped the parenthesis rule in Servers | Sheet lead, Forget row and toasts reworded; "Profile" survives only inside Inspect |
| 9 | Two different Reset sheets for one action, one auto-opening on load | One shared `openResetSheet` in `shared.js`, used by Servers and Attention; the rollback state shows the banner and inert page, a separate `reset` state opens the sheet |
| 10 | Pairing carried a blue primary on a feature the desktop lacks; step 5 was scoped to an account Settings could not show | Pairing's Start is plain and disabled beside the "not in the desktop yet" chip; Your Macs has an account switcher; step 5 reads "Set up your other Mac" for rae |

Partial asks: Priya's install link is now a step in the invite message
(placeholder address, labelled as such); Marcus's drop target is real in the
Document kind and the team store header carries the one-line "you can read
what's in here" sentence once; Marcus's one-box team creation asks one name
and derives the alias; Sol's "nothing routine looks like Add server" is met by
making Create team a plain button so each page has at most one primary.

Also fixed from my own grep: rail badges are defaults in `shared.js` so every
page agrees (Home 2 remaining, Attention 4); tall sheets no longer clip their
insets; the two hex literals in per-file styles became tokens.

## 4. Pass 2 — what was found and what was done

The second reviewer verified thirteen of the fourteen pass-1 items as
resolved in the rendered states (the fourteenth, the duplicated team card,
had been deferred) and found that one fix had introduced a defect. Its
verdict was "not ready to sign off" with four must-fix items and three
smaller ones; all seven were then applied and the set re-rendered.

| # | Finding | Resolution |
| - | ------- | ---------- |
| N1 | Opening the band range to its full extent let the Change role sheet "demote" a Member · 0 to Member · 3 | The demote stepper is bounded at the current band minus one (Admin and Owner step down to Member at 0 or below); the current role is drawn unselectable with "A higher role or band means remove and re-add, which rekeys" |
| N2 | Pages under a "nothing can be read or changed" banner kept server-touching controls live | New under Items' lease refusal, passphrase and create-account actions on the lapsed server page and on the Account row are disabled with a reason; local actions (compare, Forget, Reset) stay live. One correction on top of the reviewer's ask: Check again stays live, because a lapsed lease still permits a probe — the one operation FOKS allows in that state — and the banner says so |
| N3 | The Attention badge was a hard-coded 4 while each screen picks its lease world | The badge is computed: three needs-you items plus the lapsed check-in only when it is lapsed; every fresh screen shows 3, every lapsed screen 4, and Attention counts its live cards |
| N5 | Home and Teams drew the same team card two ways | One `teamCard()` in `shared.js`, used by both |
| N4 | Two roster counts in one file | The roster header leads with the same "6 people and teams" as the tab |
| N6 | A lapsed check-in was amber on Servers and red everywhere else | Red (crit) everywhere |
| N7 | dana.okafor's removal and re-addition were reconciled only inside a collapsed detail | One sentence in the card body of both cards; the fixture's generation for the re-added party advanced to 7 |

After this pass every state renders clean again (34 states), and the four
must-fix items are verified in the renders rather than only in the code.

## 5. Assessment against the brief

- **Hierarchy.** A newcomer sees four destinations and two utilities. An
  item's row answers what it is, where it lives (store under its server), and
  who can read it (a chip that expands to names); the detail answers what
  they can do to it and names the version guard on every write.
- **Setup path.** First run asks who is setting you up; the invited path ends
  on a Waiting screen with the exact sentence to send and, honestly, a
  Proposed band on Check now because team discovery does not exist in the
  agent yet. Home carries Get started until it is done.
- **Consumer finish.** Light sidebar, inset groups, chips, avatars, one
  primary per surface, red destructive actions alone at the bottom behind
  typed confirmation. It no longer reads as an operator console.
- **Honesty.** Twelve Deferred / Proposed / Gated bands across the six files
  mark every place the wire falls short: team-store writes and role choice,
  team discovery, probe acceptance, device pairing in the desktop, un-admitting
  a team, removing a software device, and the gated rollback distinction.

## 6. What remains open

- **One fixture, two worlds.** Home, Attention and the Servers list draw
  Acme's check-in lapsed (the troubled world); Items and Teams draw it fresh
  so Engineering is legible. Each file states which in its review note, and
  Items and Teams keep a lapsed state for the refusal. In the real app this
  tension does not exist; in the mock set it is a documented choice.
- **Negative bands print with a hyphen** ("Member · -2") because `roleText`
  formats the number plainly; cosmetic.
- **Nothing here has been user-tested.** The five persona audits ran against
  waves 1–2; the recommendations in `SUMMARY.md` include a second audit
  round against wave 5 once the ship-first patch is in the real app.

## 7. Wave 1b — variants of the Apple Passwords and Drive mocks

Seven files drawn after this sign-off, against wave 1 rather than wave 5.
They are recorded here because the product owner has chosen from them, and
the choice supersedes part of wave 5's Items work.

**The feedback.** The Drive mock (`wave1/05`) is visually clean and its
components are good, but semantically the app should contain passwords,
resources and files-in-a-password-manager rather than files in a drive. The
Apple Passwords mock (`wave1/04`) is too busy — despite being easier to
parse than 01–03 — but has the right semantic atoms. Wave 1b answers both
halves.

| File | What it keeps | What it changes |
| --- | --- | --- |
| `wave1b/01-vault-list.html` | 05's stage, window, sidebar with server captions, header with search and Sort, the New split button, hover row actions, the details panel, notice / conflict / flash | A kind filter (All · Passwords · Resources · Files · Links) replaces the breadcrumb and folders become path chips; rows are name, a Readable-by chip and version over a "vault · server" subtitle; the panel holds 04's Login, Info and Sharing insets and the Manage sheet |
| `wave1b/02-drive-with-groups.html` | 05 verbatim — sidebar, breadcrumb, folders as navigation, rows, tiles, toast, lease and conflict states | Only the words, the four kinds, avatar stacks, the Login inset, the Sharing roster and the Manage sheet, so it can be compared with 05 line by line and the semantic change judged on its own |
| `wave1b/03-passwords-calm.html` | Every atom of 04: All / Personal / Groups / Attention, Login + Details + Sharing, the Manage sheet, the same three columns and tokens | Ten named sources of busyness removed: one-line rows, a monochrome kind glyph, one value inset, About and Sharing as single rows that expand on click, one footnote after Show instead of one under every value, no repeated deferred notes |
| `wave1b/04-card-vault.html` | 05's tile grid, section labels, toolbar, segmented control, 300px details panel, sheet, download toast, lease notice, Attention cards | Tiles are grouped by vault and group with avatar stacks and a Manage button per section; 04's Login / Value / File / Link, Info and Sharing insets fill the panel; group-store writes are drawn live under one amber Deferred band instead of a disabled flow |
| `wave1b/05-resources.html` | 05's whole visual system, and 04's inset lists, hued initials, avatar stacks and Manage sheet | The sidebar is organised by what things are (Passwords, Resources, Files, Links) before vaults and groups; a resource type chip is derived from the path (`/agents` → API key, `DATABASE_URL` → Database, `*token*` → Token, `/ssh` → SSH key, `/env` → Env); a Use-it inset gives the CLI line |
| `wave1b/06-onboarding-in-shell.html` | `wave2/03`'s six steps and their copy, and 01's shell CSS verbatim | First run no longer replaces the window: it runs inside the same titlebar, sidebar and main area, ends on a group, and leaves the resumable Get started checklist in the sidebar |
| `wave1b/07-first-run-in-shell.html` | `wave4/06`'s seven steps, both paths, the Waiting screen, the published-host-id compare, the two PROPOSED bands | The steps become sidebar rows with the shell's dot colours, and "You're in" *is* the shell — Engineering appears under Shared groups and staging-token opens in the details panel |

**The decision.** The vault-list (01) is the shell: it is the cleanest
combination, 05's components carrying 04's objects with nothing added. The
sidebar and the item-page header come from the card vault (04), which puts
All items above the section labels and gives the header a location subtitle
("Household · 2 people · named group on foks.example.net") that the
vault-list's path bar does not. The New sheet comes from the vault-list,
because its Save-in list names vaults and groups with "shared with N people"
and carries exactly one amber Deferred band. The error dialogs and error
layouts come from the card vault — the conflict sheet, the lease notice,
the inactive-group notice, the already-exists refusal, the agent-lost state
— with the vault-list's words where the two differ only in words. The first
run is wave 1b's own (07), rebuilt on the shared shell. `wave6/SPEC.md`
records this as the build order.

**What the variants got right that wave 5 did not.**

- **Password / Resource.** Wave 5 offers seven kinds on the surface (Login,
  Key or token, Note, Document, SSH key, Link, Folder) over four node types.
  Wave 1b reads four, and the Password / Resource split is the one that
  earns its keep: a Resource is a thing a program reads, so the agent and
  machine-identity case is visible in the item list rather than only on the
  Teams roster.
- **The calmer detail.** 03 audited its own busyness and named ten causes;
  the result is a detail pane wave 5's Items screen does not match — one
  value inset, one footnote after Show, About and Sharing as rows that
  expand on click, the title line sharing its row with Edit.
- **Readable-by is a first-class column.** Both sets compute the chip, but
  wave 1b gives it one of three columns (name, Readable by, version) and
  words it in people — "5 people", "2 people", "only you", names on hover —
  where wave 5 spends five columns and words it "Readable by 6 ▾".
- **One window.** Wave 1b's first run runs inside the shell the newcomer
  will use, and the last step is the vault itself. Wave 5's `firstRun()`
  draws its own screen with no rail, so setup and the app do not look alike.
- **Folders are not places.** Making the path a chip removed the breadcrumb,
  the folder tree and the empty-folder state at once. Wave 5 keeps Folder as
  a kind in the New sheet.

**What wave 5 still does better.**

- **The Trust page.** Host identity, lease, history check, keys, federation,
  and the proves / does-not-prove lines. Wave 1b has "Servers & devices" as
  a sidebar footer row and nothing behind it.
- **The Attention inbox.** Severity order, consequence → mechanism → action
  with its source response, filters, Resume for interrupted operations, and
  an empty state that says what will appear. Wave 1b's Attention is a card
  list.
- **The Teams page.** People & teams with a Reads column, the "what this
  party can read" panel, Connect an agent, Invite someone, Federation with
  the honest Take-it-back answer, Store and Danger. Wave 1b has the Manage
  sheet and no team page.
- **Settings.** Your Macs & recovery, security keys, agent, and the reset
  blast radius. Wave 1b defers all of it to a footer row.

**Two caveats.**

- **File size.** Following `BRIEF.md`'s self-contained rule, each wave 1b
  file carries its own tokens, fixture and helpers: 52–101 KB each, 5,166
  lines across seven files, with the fixture duplicated seven times. Wave 5
  broke that rule for six files with `system.css` and `shared.js`, and wave
  6 does the same with `shell.css` and `shell.js`. Further variant work
  should start from the shared files, not from a copy.
- **"2 people can read production-token".** Engineering has five people, and
  every Engineering row's Readable-by chip says 5 people except
  production-token, whose read role is Admin: that chip says 2. The number
  is computed from read role × roster by `readers()` in each file and is
  never typed in, so it does not flatter — a row in a five-person group can
  honestly read 2 people, and the panel names them. Keeping that number
  computed is the point of the chip; a hard-coded member count would have
  hidden the one item on the screen that behaves differently.

## 8. Wave 6 — the chosen combination, implemented

Wave 6 is the product owner's choice out of wave 1b, built. `wave6/SPEC.md`
names each piece and its source: the vault-list shell, rows, details panel,
Manage sheet, footer panes and New sheet from `wave1b/01`; the sidebar, item
page header, error dialogs, error layouts and grid from `wave1b/04`; the
first run from `wave1b/07`, rebuilt on the shared shell; and a **Set up
again** row below Settings that re-enters it. Two files carried it when the
audits ran — `wave6/01-vault.html` with 23 states and `wave6/02-first-run.html`
with 17 states on each of two paths — over `wave6/shell.css` (tokens and
components) and `wave6/shell.js` (the corrected fixture, the kind rule, the
readers computation, and the sidebar and header builders every file uses).
Three more have since joined them on the same shell (below), so the wave is
five files and 89 states. It is the first set in this iteration that is one
application rather than a set of drawings: the files link to each other, and
the last step of first run is the vault itself.

**How it was checked.** Two Opus implementers built the two halves and each
self-reviewed once against `SPEC.md` and the rules in `BRIEF.md` §2 and
`WAVE4-BRIEF.md` §1. The product owner's reviewer then reviewed the whole set
and found three defects, all fixed: the Attention badge rendered at zero, the
lease notice carried a primary on an action the lease refuses, and the search
placeholder still said "Search all items" under a group header. A render
sweep followed: all 57 states at 1280×860, zero console errors, scrollWidth
1280.

**The round-two audits.** The five personas from wave 3 re-walked their own
goals against the two files, each a separate Opus session in character, with
their round-one report in hand so the new one could say what changed
(`wave6/audits/PROTOCOL.md`). Each reported a then-and-now table marking every
round-one frustration Fixed / Partly / Not with the state as evidence, a fresh
first-person transcript, a remaining-issues table with severity and a
needs-protocol column, anything wrong about FOKS on any screen, and one
sentence on whether they would use it. Wave 6 covered only Items and first run
when they ran, so each auditor read `wave5/` for Trust, the Attention inbox,
the Teams page and Settings; those three screens are wave 6 files now. The
five reports are verbatim under `wave6/audits/`; their 46 remaining issues are
merged into ten themes in `wave6/audits/ISSUES.md`.

| # | Theme | Blockers · majors | Plan phase |
| - | ----- | ----------------- | ---------- |
| T1 | Team writes and the read-role control | 1 · 3 | 3, 7.6 |
| T2 | Team discovery and the Waiting screen | 1 · 2 | 5, 7.2 |
| T3 | Machine identities and revocation | 2 · 2 | 4 |
| T4 | Federation surface | 1 · 1 | 4, 7.4 |
| T5 | Servers, devices and settings are too thin | 3 · 4 | 6 |
| T6 | Reader counting semantics | 0 · 2 | 1, 2 |
| T7 | Invite message on the owner's side | 0 · 2 | 4 |
| T8 | Vocabulary leaks and unproven claims | 0 · 1 | 0 |
| T9 | First-run polish | 0 · 0 | 5 |
| T10 | Landing identity | 0 · 0 | 5 |

Counts are persona-issue rows (8 rows marked blocker, 17 major, 21 minor);
`ISSUES.md` merges the blocker rows into six distinct blockers, B1–B6, because
three personas raise the missing recovery and second-Mac pages separately.
Phases are `IMPLEMENTATION-PLAN.md`'s.

**The verdicts.** Priya: yes, the day a Who-can-read control ships; the
roster, the roles and the readable-by answers already beat 1Password Teams,
but she cannot create the one secret the exercise was about. Marcus: yes for
reading, because it finally says where the PDF lives and who can open it; not
yet for the family, because he cannot add to Household and is sent twice to a
page that does not exist. Jun: yes as a personal vault, because it reads a
store better than anything else here; not for the job, because nothing tells a
program how to fetch a value and nothing takes a bot's access away. Ade: yes
as a password manager, and the compare-and-pin screen should be kept verbatim;
but he cannot add a second server, the Federation page is pointed at and
absent, and no history-fork state is drawn. Sol: yes; every question from
round one is answered on the screen where it arises, and the only thing in the
way is the honest amber band saying the group cannot be seen yet.

**The ports, and what they closed.** What the combination had lost was the
rest of wave 5, and four of the six blockers were exactly those absences.
Those three files now exist on the same shell, and each was checked state by
state against `ISSUES.md` rather than against its own commit message:

| File | States | What it closes |
| --- | --- | --- |
| `wave6/03-groups.html` | 14 — `groups` · `people` · `party` · `federation` · `store` · `danger` · `invite` · `add` · `demote` · `remove` · `admit` · `create` · `lease` · `inactive` | B3, B4, T3, T4 (bar un-admit), T6, T7. Every **Reads N of 4** and every readable-by list comes from `readersOf`; an admission that reports inactive is struck through and reads nothing; a promotion is named as remove and re-add — two operations, and it rekeys — never as one signed step |
| `wave6/04-servers.html` | 8 — `list` · `server` · `lapsed` · `rollback` · `reset` · `add` · `unprobed` · `check` | B5 and the servers half of T5: Add, Check, the full 66-hex ids behind Show full and Copy, the local out-of-band compare, Forget, and Reset behind typed confirmation naming the resumable it discards |
| `wave6/05-settings.html` | 7 — `macs` · `phrase` · `keys` · `enrol` · `account` · `agent` · `about`, plus `?state=macs&account=work` | B6 and the settings half of T5: Your Macs and recovery, the 17-word phrase shown once, Security keys with alias and server on the enrolled row and the serial only where the wire carries it, and **Set up again** |

`01-vault.html` grew to 26 states (`new-group`, `invite`, `party-remove`) and
`02-first-run.html` kept its 17 × 2 while taking the round-two copy fixes, so
T1's greyed control, T2's band placement, T8's vocabulary, T9's first-run
polish and T10's landing identity all landed there. `shell.js` gained
`admissionActive`, a `readersOf` that excludes an inactive admission, and
`peopleGroups`, which is T6 at its root.

**What remains open.** Three things, all of them protocol and none of them
drawable away. **Team discovery** (B2 / T2): a group someone else adds you to
cannot appear, so the Waiting screen's Check now can only answer "Not yet" —
left Proposed by decision, Phase 7.2. **Group-store writes with read/write
role choice** (B1 / T1): without them the New sheet's Who-can-read control can
only ever be greyed, which is what it now is, with the computed preview beside
it — Phase 7.6. **Un-admit** (T4): `RemoveTeamMember` names a username and an
admitted group has none, so Federation carries the honest sentence instead of
a control — Phase 7.4. One place the ports deliberately did not do what the
audit asked: the history-check failure is drawn as fact with no gated tag,
because `readiness/PROTOCOL-READINESS.md` §1 finds typed errors do exist. The
verdict on the wave is therefore that the design work is done except where the
protocol stops it: every blocker that was a missing screen is a screen now.
The rest of §6's open items still stand, except that the second audit round it
asked for has now run against wave 6 rather than wave 5, and still against
mocks rather than running software.
