# Persona audit 4 — Ade, security-minded self-hoster / small-company admin

**Persona:** Ade — runs the FOKS server for a six-person shop; comfortable with
YubiKeys, PIV, offline backups and SSH host-key pinning; assumes every trust
claim is false until a screen proves it. **Model:** Opus.
**Goal:** add a second server, probe and pin it, admit a partner company's team
into our internal team by federation, enrol a YubiKey and a backup phrase, and
recover the account on a new Mac.

---

## Round two (wave 6)

Ade re-ran the operator goal against `wave6/01-vault.html` and
`wave6/02-first-run.html`; his report is
`../../wave6/audits/04-ade-round2.md`. Two of his five round-one frustrations
are Fixed (the thirteen identical blue buttons, by deletion, and the 17-word
phrase, now a shown-once write-it-down sheet), two are Partly (probe acceptance
and the out-of-band host-id compare both exist, but only inside the first-run
wizard, not on Servers & devices) and one is Not (Federation is pointed at from
the Manage sheet and was never built). His verdict: he would run wave 6 as his
password manager tomorrow and keep its compare-and-pin screen verbatim, but he
cannot run his server on it, because it cannot add a second server, sends him to
a Federation page it never built, points at a Settings with no key enrolment,
and draws no history-fork state at all.

## 1. The attempt against the application as built

**Adding the second server.** **Servers**: one card, two fields (Profile name,
FOKS probe address), four buttons in a row — *Add server*, *Probe selected
server*, *Remove selected server*, *Reset hard state* — all the same size,
weight and blue. The nuclear option sits two tab stops from the button I want
and nothing about its appearance says so. Adding `partner` /
`foks.partner.dev:443` was otherwise painless.

**Probing and pinning.** The card's subtitle is the one sentence in the whole
app that talks about trust: "Add a Go-compatible v0.1.9 server, then probe and
pin its authenticated host identity before creating an account." I clicked
**Probe selected server** — and immediately wanted to know *which* server is
selected. The only affordance is a single pill labelled `local` above the form;
with a second profile added I have no confidence about which one the destructive
buttons point at.

The probe result appears as raw JSON below: `lookup_name`, `canonical_name`,
`host_id_hex` (the full 66 characters), `host_chain_sequence`, `merkle_epoch`.
I appreciate the unvarnished payload, but it answers neither question I came
with. **Is this the first time this Mac has seen this host id, or does it match
what was pinned?** Nothing says. `ProbeOutcome.acceptance` (`Inserted |
Advanced | Unchanged`) exists in the client and is thrown away before it reaches
me — GAPS.md §26 names this and calls the fix a two-line change. **And can I
check the host id against the fingerprint the partner's admin read me over the
phone before it gets pinned?** No: no expected-host-id field, no confirm step,
no diff. The pin happens as a side effect of the probe and I learn what I pinned
afterwards, by reading JSON. So "probe and pin" is pure trust-on-first-use and
the app never uses the words. That is the single biggest failure for me; SSH has
had `StrictHostKeyChecking` and a fingerprint prompt since 1999.

**Federating the partner team.** **Parties** → *Federated team admission*: four
free-text boxes (Local named team, Remote profile, Remote team, Member
visibility). No dropdown of the profiles I have actually probed, no host id for
the remote team, no preview, no confirmation sheet. I clicked **Admit remote
team** and the JSON panel changed. Its helper line — "Both profiles must already
be probed and contain active teams. Desktop admission uses a member role; CLI
and agent clients can explicitly select admin or owner" — is accurate and doing
all the work by itself. Afterwards the only view of what I admitted is **List
remote bindings**, another JSON array, and there is no way at all to take an
admission back. Also here: **Remove member** is styled identically to **List
roster**; a one-character slip rekeys the team.

**YubiKey.** *YubiKey lifecycle* is **thirteen identical blue buttons in three
wrapped rows**. Row two reads: Resume management rotation, Recover management
key, Recover subkey, **Revoke YubiKey**, Change PIN, Change PUK. The action that
rotates my account keys and permanently retires the card sits between two
routine ones, same colour, no separation. It does open a confirmation sheet with
good copy ("The card will no longer open the account; copies it already
downloaded cannot be recalled"), and `gui.rs`'s `show_confirmation` focuses
**Cancel** — which I checked and credit. What the card never says is that
preparing a card's PIV slots cannot be resumed: "Resume enrollment" is a peer
button in the same row, reading as a promise that anything interrupted can be
picked up. It cannot — `ensure_slot_empty` refuses the retry and the card needs
a PIV reset.

**Recovering on a new Mac.** *Owner recovery* → **Enroll backup** returns the
17-word phrase **into the JSON response panel**, where it sits in a scrollable
debug pane until I navigate away. A full account key rendered as a `"phrase"`
string in a log-shaped box is not how you hand someone a paper backup.
*Recover account* with that phrase does work. What is absent entirely is pairing
this Mac from an existing device: `StartDevicePairing` / `AcceptDevicePairing`
exist in `foks-agent-proto` and are wrapped at
`crates/foks-desktop/src/lib.rs:1343`, but `gui.rs` never mentions pairing. So
the backup phrase is the only route onto a new Mac — and if I skipped it, that
Mac is CLI-only, with no warning at the moment I skip. **Notifications** said
"No actions need attention" throughout, lapsed lease and inactive federation in
the fixture notwithstanding.

## 2. The same goal against the wave 1 / wave 2 mocks

Wave 1 is all Items — Finder, Keychain, 1Password, Apple Passwords, Drive. None
of it is my job; `01-finder`'s greying of a whole server on a lapsed lease is the
one instinct there I'd keep. Wave 2 is where my goal lives, and
**`04-servers-identity` got me furthest.**

- `?state=add` is the first screen in the set honest about the trust model: a
  **Trust policy** row reading *"Pin on first probe. This Mac pins whatever host
  id the first probe returns and refuses the server if its signed history later
  goes backwards or forks against that pin"*, then a **PROPOSED** amber band
  saying an expected host id to compare against is not part of adding a profile
  today, field drawn disabled. It tells me it is TOFU *and* that the fix is not
  built. I trust the whole mock more for that band.
- `?state=unprobed` — "Nothing on this server can be trusted until it is probed
  from this Mac", every fact `—`. `?state=server` — "Host facts, **as of last
  probe**", "…not live… there is no operation that reads the pinned host without
  touching the network." `?state=lapsed` adds "Probing is still allowed", which
  matches what the capability gate really permits.
- `?state=rollback` — the page goes inert, the banner says the scope is the
  server and not one store, and that reset is the *only* remedy. `?state=reset`
  — a Discarded / Lost / Kept / Untouched list plus **type the server name to
  continue**, button disabled until you do: the treatment the as-built's lone
  "Reset hard state" button should have had.

**`01-system-settings` won the YubiKey question outright.** The thirteen buttons
become Enrolled keys / Connected now / Add / Everyday / Recovery / Danger, each
row with one explanatory line, and it gets three things right:

- *"The enrolled list cannot say which enrollments finished — a half-enrolled
  key looks the same as a complete one. Resume enrollment is what finds out, and
  it needs the card's PIN."* (GAPS §15 plus the PIN requirement from §22.)
- *"Preparing a card writes both keys in one step that cannot be split… the
  slots are no longer empty and the retry is refused. Getting that card back
  means resetting its PIV applet, which erases everything on it."* (§22, right
  down to the consequence.)
- *"Change PUK — the unlock code is yours to choose and yours to keep: nothing
  generates one for you and nothing will show it to you later."* (§22.)

Revoke sits alone in a red **DANGER** group, and the Reset hard state sheet
gates on an "I know why this server's history moved, and I am not doing this to
make a warning go away" checkbox.

**`02-teams-hub` won federation.** Its Admit sheet has a *dropdown* of servers
already probed on this Mac, the role locked to Member with the reason attached
("Admitting as Admin or Owner is a CLI operation" — matching the agent's actual
refusal), a visibility stepper saying what the band changes, and "Admission is
journaled and reports one state, active or not." The Members table shows party
id, kind, **Role here** and **Role at source** as separate columns, and
generation — the first place I could see that a member can be a team.

**`03-onboarding`** answered the new-Mac question with `?state=existing`: "Add
this Mac to your account", three routes (pair from another device, backup
phrase, YubiKey), the non-splittable-prepare warning repeated at the right
moment, and a phrase sheet with an "I have written these 17 words down" checkbox
and no copy button — though **Start pairing** is its primary action for an
operation the shipped GUI does not expose. **`05-pills-evolved`** is the cheap
fix and gets the structure right (reset behind a collapsed Danger disclosure,
revoke in a red menu column), but its Security-keys card says *"One YubiKey is
enrolled for personal"* — the exact overclaim `01` refuses to make.

**What still failed in every mock.** Nowhere can I verify a host id out of band;
`04` draws the field disabled and marked proposed, which is honest but still
leaves me without an answer. Nowhere shows probe acceptance. Host ids are
truncated to ten hex characters with no full value and no copy control, while
`02-teams-hub` puts "Copy id" next to the *team* id — the one I need least. And
no mock offers a way to un-admit a federated team.

## 3. Ranked frustrations and wins

**Frustrations**

1. No screen anywhere — app or mock — tells me whether a probe confirmed the
   host id I already had or pinned a new one. Pinning is invisible.
2. I cannot check a host id against an out-of-band value before it is pinned.
   TOFU is the actual policy and only one mock ever says the word.
3. Thirteen identical blue buttons with **Revoke YubiKey** in the middle of them,
   and **Remove member** identical to **List roster** on Parties.
4. Federation is four blank text boxes with no picker, no preview, no host id,
   and no way to reverse an admission once made.
5. The 17-word backup phrase is delivered into a JSON debug panel.

**What worked**

1. `04-servers-identity`'s "Pin on first probe" + PROPOSED expected-host-id band.
2. `01-system-settings`'s three YubiKey sentences — all check out against GAPS
   §15 and §22.
3. The reset sheets in `04` (type the server name) and `01` (acknowledgement
   checkbox), and their Discarded / Lost / Kept / Untouched lists.
4. `02-teams-hub`'s two-role Members table and Member-only admit sheet.
5. In the shipped app: confirmation sheets exist, their copy leads with
   consequence, and Cancel takes focus. I'd keep all of that.

## 4. Three changes I would ask for

1. **"After a probe, tell me in one line whether that host id is new, the same
   as before, or an advance — and let me paste the fingerprint the other admin
   gave me before you pin anything."**
2. **"Move every destructive button somewhere I cannot hit by accident, and
   spell out what I lose. In particular, tell me that Reset hard state throws
   away every half-finished operation I could still have resumed on that
   server."**
3. **"Give me one Federation page per team that lists what I admitted, from
   which host, at which role and whether it is active — and a way to take it
   back, or a plain sentence saying I cannot from this Mac and what I have to
   run instead."**

## 5. Things I was told that are wrong or missing

- **Reset hard state's blast radius is understated everywhere.** The app says it
  "discards the rollback checkpoint and hard-state database, including the
  authenticated host pin". That database is also what holds the finalizable
  mutation records every *Resume* button depends on (`account.rs:63`, `:192`,
  `:223`), and `reset_hard_state_locked` deletes it and its sidecars. Resetting
  therefore destroys every resumable signup, team creation, member edit,
  provision, recovery and enrolment on that profile. Nothing says this.
- **Rollback and lease-lapse are drawn as distinguishable failures, and today
  they are not.** GAPS §16 says every dispatch failure collapses to
  `OperationFailed` with a prose string, and that these screens must be
  "explicitly marked as requiring §5 group E". No mock carries that mark;
  `04-servers-identity`'s History-mismatch banner and `01-system-settings`'
  lease banner both read as shipping behaviour.
- **`01-system-settings` shows a serial and a server on an *enrolled* key row**,
  two lines above its own correct caption about that list. `ListYubiAccounts`
  returns aliases only; joining a connected card to an alias is a guess the wire
  cannot support. Separately, **`03-onboarding` shows a "Set" chip on
  Passphrase**, while `04-servers-identity` correctly says whether one is set is
  not reported.
- **`02-teams-hub` explains the greyed-out Remove on the federated party as
  "managed where it lives".** `locally_manageable` means only that the party is
  a local user (GAPS §12). Removing homelab from Engineering is an
  *Engineering-side* operation; the label sends me to the wrong server.
- **Device pairing is missing from the app and missing from GAPS.**
  `StartDevicePairing` / `AcceptDevicePairing` exist on the wire and are wrapped
  in `foks-desktop/src/lib.rs`, but no `gui.rs` screen exposes them and GAPS.md
  never records the omission — while `03-onboarding` draws "Start pairing" as
  the primary route onto a new Mac.
- **Get started never mentions the CLI-only steps** that GAPS §17 says exist;
  the shipped checklist shows four in-app rows and an "Initialize secure state"
  button, implying otherwise.
