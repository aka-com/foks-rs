# Persona audit 4, round two — Ade, security-minded self-hoster

**Under audit:** `wave6/01-vault.html`, `wave6/02-first-run.html`.
**Goal, unchanged:** add a second server, check and pin it, admit a partner
team by federation, enrol a YubiKey and a backup phrase, recover on a new Mac.

---

## 1. Then and now

| # | Round-one frustration | Verdict | Evidence |
| - | --- | --- | --- |
| 1 | No screen says whether a probe confirmed the host id or pinned a new one | **Partly** | `02-first-run ?state=checked` carries it — "**New pin** … A later check would say **Same as before**, or **Advanced**" — under a PROPOSED band naming `ProbeOutcome.acceptance`. `01-vault ?state=servers` has nothing of the kind. |
| 2 | I cannot compare a host id against an out-of-band value | **Partly** | `?state=compare`: a 66-hex paste field, Match / prefix-only / Mismatch, and the sentence I wanted — "Compared on this Mac only … the result does not change the pin — a mismatch is for you to act on." It exists once, in the wizard. |
| 3 | Thirteen identical blue buttons with **Revoke YubiKey** among them; **Remove member** identical to **List roster** | **Fixed, by deletion** | `?state=settings` is three headings and a link; `?state=manage` has no Remove, no Revoke, **Lower** disabled and deferred. The one red control in the set is **Remove** on a personal item, alone, behind a sheet naming the exact version. |
| 4 | Federation is four blank boxes, no picker, no host id, no way to reverse | **Not** | `?state=manage` shows the admitted group with both roles and says "change it from **Engineering's Federation page**". There is no such page. One Attention card is the whole surface. |
| 5 | The 17-word phrase is delivered into a JSON debug panel | **Fixed** | `?state=phrase`: seventeen numbered words, "Shown once. No copy button, on purpose", a written-it-down checkbox gating **Done**. |

---

## 2. A fresh transcript

**Servers & devices** is subtitled "As of the last check — nothing here
refreshes on its own", which is correct and which the shipped app never said.
Three rows: `foks.example.net` (host 9f31c2aa07, chain 12, epoch 4821, fresh
6 d), `foks.acme-corp.com` (correctly **lapsed**, no expiry), and
`foks.partner.dev` — "never checked". That last row is my second server and
there is nothing I can do to it: no **Check** on any row, and **Add a server…**
only flashes "Adding a server checks it (a probe) and pins what comes back" — a
sentence about something the page does not do. Host ids are ten hex characters,
no full value, no copy. A heading reading **This Mac** lists two Macs.

So I go the long way: **Set up again** → `02-first-run?state=who`. Step 2 is the
screen I asked for last time — "Found it. This Mac now recognises
`foks.example.net` and will refuse anything else that answers to that name.
Nothing about you has been sent yet", a **Pinned on this Mac** chip, **Details**
with the five `ProbeReport` facts and the host id in full, "Inspect response" as
an escape hatch rather than the surface, and the amber band giving me
new/same/advanced while saying plainly it is not a fact yet. I paste the
sixty-six characters our partner's admin read me: "✓ **Match.**" A ten-character
prefix gets "**Prefix matches** — a short form is not a verification; paste the
whole id." Right answer to the right question. But Settings promised "each step
says what it finds and what it would change", and it doesn't: step 2 hard-codes
"New pin — the first time this Mac has seen this server".

**Federation.** Engineering's Manage sheet is the best roster in any wave: party
rows with `group` tags, two roles on the admitted team ("Member · visibility 0"
here, "Owner where it lives"), and a visibility stepper reading "0 is the
default; lower bands see less". Then it sends me to a Federation page that does
not exist. I cannot admit the partner team, or see a remote host id, destination
role or active flag. **Attention** says "Homelab's admission into Engineering is
not active … so Homelab's members read nothing in Engineering through it" —
while the details panel for `/deploy/staging-token` says "**6 people** can read
this", counting Homelab as one of them.

**YubiKey and recovery.** `?state=protect` has the three hard facts right:
prepare cannot be split or resumed and needs a PIV reset; the card must still
hold its factory management key; the PUK is yours to choose and keep. Then
**Enrol a YubiKey…** says it is done "from Settings, not here" — and Settings
has no enrolment, only "primary key · YubiKey 20993145 · connected", a join
`ListYubiAccounts` cannot make. `?state=existing` honestly marks pairing
"wire-backed · not in the desktop yet", then makes **Start pairing** the blue
primary over **Recover**, which works.

**Absent entirely:** any rollback/fork state, reset, Trust group or per-server
page. `wave5/04-servers.html` still has all of them — `?state=server`,
`?state=rollback` (inert, gated tag) and `?state=reset` (Discarded / Lost /
**Also discarded: every operation you could still have resumed** / Kept /
Untouched, behind typing the server name) — and `wave5/05-settings.html
?state=keys` carries the enrolment story. Wave 6's two panes claim almost
nothing false, so they are honest in the narrow sense; but they are a read-out,
not an operator surface, and the roster's own copy proves them incomplete.

---

## 3. Remaining issues

| # | Severity | Where | What | Ask | Needs protocol? |
| - | --- | --- | --- | --- | --- |
| 1 | blocker | `01-vault ?state=servers` | A second server cannot be added, checked, compared or pinned: **Add a server…** is a toast, `foks.partner.dev` sits at "never checked" with no control, no row has a Check, no host id is full or copyable, and step 2's acceptance and compare are absent. | "Put the check, the full host id and the paste-the-published-fingerprint box on every server row, and let me add a server from the page called Servers." | no |
| 2 | blocker | `01-vault ?state=manage` (Engineering) | The roster names "Engineering's Federation page" and there is none: no admit sheet, no admissions list, no remote host id, no destination role, no active flag, and no sentence saying I cannot take one back. | "Give me the Federation page you point at: what I admitted, from which host, at which role, active or not — and how to take it back, or the sentence that I cannot from here." | no |
| 3 | major | `01-vault` Sharing panel, `?state=group` and Engineering | "6 people can read this" counts an admitted *group* as one person, and counts it as a reader while Attention says its admission is inactive and its members read nothing. | "Don't count a group as a person, and mark an inactive admission in the roster the way Attention marks it." | no |
| 4 | major | `01-vault ?state=settings` | "primary key · YubiKey 20993145 · connected" joins an alias to a connected card; `ListYubiAccounts` returns aliases only and merges finished with half-enrolled. | "Alias and server on that row, serial on a connected-card row, and tell me the list cannot say which enrolments finished." | no |
| 5 | major | `02-first-run ?state=protect` → `01-vault ?state=settings` | **Enrol a YubiKey…** says enrolment happens in Settings; Settings has no enrolment, recovery, passphrase or revoke. A dead end on one of my two protection routes. | "Carry wave 5's Security keys section, or say enrolment is not in this build and name what to run instead." | no |
| 6 | major | whole set | No rollback/fork state and no reset anywhere, though BRIEF §2 makes a fork block the whole server and outrank everything, and WAVE4-BRIEF §1 says reset also discards every resumable operation. | "Draw the history-check failure with its gated tag, and the reset sheet listing discarded / lost / kept / untouched — resumables included — behind typing the server name." | no (gated tag is the marker; telling it from a lease refusal is plan §5 group E) |
| 7 | major | `02-first-run ?state=existing` | **Start pairing** is the blue primary on the route the desktop does not call, over **Recover**, which works; it points at "Settings › Devices › Set up another Mac", which this set lacks. | "Make the route that works the primary one, and don't send me to a menu you haven't built." | no |
| 8 | minor | `01-vault ?state=settings`, "Start over" | "Each step says what it finds and what it would change" is untrue: step 0 re-narrates preparing this Mac, step 2 hard-codes "New pin — the first time this Mac has seen this server". | "If Set up again is how I add a second server, have it say what it found; otherwise drop the promise." | no |
| 9 | minor | `01-vault ?state=servers` | Heading **This Mac** lists two devices, one "Travel Mac"; host ids are ten hex with no full value or copy, while a team id gets 8+4 in Manage. | "Call it Your Macs, and let me see and copy a whole host id." | no |
| 10 | minor | `01-vault ?state=attention` | "Re-run the admission" is the only federation control in the product, and nothing says what re-running does to the roster or whether it rekeys. | "One line on what re-running an admission costs." | no |

---

## 4. Anything wrong or missing about FOKS

Checked against BRIEF §2 and WAVE4-BRIEF §1.

- **Readable-by over-counts a federated party twice** (issue 3): a team counted
  as a person contradicts GAPS §12; counting an inactive admission as a reader
  contradicts `FederatedMembershipSummary.active`, which this build's own
  Attention card reads correctly. The one place wave 6 states a fact the wire
  denies.
- **The Settings key row asserts an alias-to-serial join** GAPS §15 says cannot
  be made, and drops the "a half-enrolled key looks the same as a complete one"
  sentence wave 5 got right.
- **The `locally_manageable` wording is right and points nowhere** — it is the
  sentence WAVE4-BRIEF §1 asked for, on a page nobody built; deploy-bot's "a
  user of foks.acme-corp.com like any other" is likewise correct.
- **Missing, not wrong:** the rollback banner and its gated tag; reset and its
  corrected blast radius; the admit sheet and its Member-only reason — the agent
  refuses Admin and Owner as federation roles, and wave 6 never says so because
  it never offers the choice. And "a YubiKey set up earlier works too: plug it
  in and its PIN is what continues" (`?state=existing`) matches GAPS §22 on the
  PIN, but nothing in the set shows that route.
- **Right, and worth keeping:** the lease copy is the correct way round — reads
  stop with writes, other servers unaffected, a probe still permitted and not
  renewing it. Agent-lost is right that no write was left half-applied.
  Team-store writes sit under one amber deferred band; promotion by
  remove-and-re-add "rekeys the group in one signed step"; every write names its
  version guard; and the compare is scrupulously described as local and
  pin-neutral.

---

## 5. Would I use it?

I would run wave 6 as my password manager tomorrow and keep its compare-and-pin
screen verbatim, but I cannot run my server on it — it cannot add my second
server, points at a Federation page it never built and a Settings with no key
enrolment, and draws no history-fork state at all: a good vault with the
operator half missing.
