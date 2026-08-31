# Wave 7 — four from-scratch takes on the Servers page

Every Servers mock so far (`wave2/04`, `wave2/04-identity`, `wave5/04`,
`wave6/04`) is assembled from the vault's vocabulary: status-striped cards,
then a server page of inset rows. These four are designed from the data the
page actually holds, each around a different structural idea, and each is
judged against `../wave6/04-servers.html` for what could be folded in cheaply.

The data, as every take reads it (BRIEF §2, WAVE4-BRIEF §1, readiness §1–§3):
the last `ProbeReport` (lookup name, canonical name, 66-hex host id, signed
history entries, signed tree version), the lease this Mac holds and its expiry
(`ListProfiles`), the typed history-check error, your account(s) on the server
(`ListAccounts`), your owner devices (`ListDevices`), the groups reached through
the account and your role in each (`ListTeams`, roster), the one resumable a
reset would discard (Homelab reports inactive), and the danger actions (reset,
forget). Adding a server writes a record and does not contact it; the first
check pins. The out-of-band compare is local. Probe acceptance is dropped
before the report and stays PROPOSED in every take.

**One extension every take makes, and flags:** the fixture has no times. Each
mock adds *when this Mac made its calls* — added, first check (the pin), last
check, account created — on this Mac's own clock, and uses the lease's own
`expires_at` for the check-in. No agent operation returns those times; a
desktop that records its own calls needs no protocol, but it is a client-side
record, not a wire fact, and each header comment says so. A lapsed check-in's
time *is* a wire fact (the lease's expiry) and is labelled as such.

## Take 1 — Ledger (`01-ledger.html`, on the wave 6 shell)

**Thesis.** A server, to this Mac, is nothing but what this Mac has recorded
about it, in order — added, checked and pinned, account created, groups
stored, checked again, check-in expired, refused. The present state is not a
separate thing; it is read off the newest rows, so "when was this learned" is
answered by construction.

**Optimises for** legibility of time and cause: every fact sits on the row
that learned it, and the two things people asked for (Ade: "did the check pin
a new id or match the old one"; Sol/Marcus: "what does lapsed mean") become
rows you can read top to bottom. It also makes the acceptance question
partly answerable *locally*: with the previous check's report kept, "same host
id as the pin · 9 → 12 entries" is a comparison between two rows the desktop
already has; only the agent's own verdict stays PROPOSED.

**Costs.** It draws a history, and BRIEF §2 says no activity log exists —
the ledger is honest only as the desktop's own journal of its own calls,
which the current desktop does not keep (it would be a client-side store of
past `ProbeReport`s and call times, no protocol). Before that journal exists,
every server has a one-row ledger. It is also long: the personal server is
seven rows, each with an inset, so the danger section is far down and the
"Now" panel at the top has to carry the summary.

**Serves best** Ade (self-hoster): the pin, the history growth, the expiry
and the refusal are all dated and comparable. Sol reads the "Now" panel and
never needs the rows.

**Fold-in against wave6/04: moderate.** The shell is shared, the sidebar,
header, sheets, `.inset/.srow` rows, the compare widget and the reset sheet
are the same code, so the cost is the page body (~250 lines of new render
functions) plus a client-side journal. Three pieces are trivial on their own:
(1) the *lease's own expiry time* on the lapsed row ("expired 30 Aug · 08:12 —
the lease's own time") — wave 6 shows "Lapsed" with no time although
`ListProfiles` carries it; (2) the Add sheet's "what the ledger will say" framing
(now: added, written here only → next: first check pins) as two rows of copy;
(3) the reset sheet's "Kept: … and the ledger" row is not needed without the
ledger. The row-to-row comparison is small (keep the previous report) and would
turn wave 6's "Last check against the pin: Not reported" into a local fact with
the band demoted to the agent's verdict only.

## Take 2 — Trust dashboard (`02-trust.html`, on the wave 6 shell)

**Thesis.** The only question a person brings to this page is "can this Mac
trust what it reads from that server right now?", so the page's spine is that
answer, per server, and every host fact is filed under the one of four
conditions it supports: the server is the one this Mac first met (the pin),
its history has only moved forward (the checkpoint), this Mac is still allowed
to talk to it (the check-in), it answered to its name. The failing condition
floats to the top.

**Optimises for** the newcomer's reading and the two failure modes' different
meanings: a No from the check-in is *permission that ran out, not distrust*
(identity and history still hold); a No from the history *is* distrust and
reset is its only remedy. Marcus, who "would never open" a page of merkle
epochs, gets Yes / No / Not yet and a sentence; the epochs are still there, one
inset down, under the condition they justify.

**Costs.** Four verdict cards in a row means each card is narrow (condition
titles are shortened on the cards). The regrouping splits what wave 6 kept
together: the host facts are no longer one block an operator can scan, and
"Signed history entries" lives two sections away from "Host id". The verdict
is computed from three facts; if a fourth failure appears (a typed error the
design has not met), the page has no slot for it without a fifth condition.

**Serves best** Marcus and Sol (the answer), and Priya (the reason the answer
is No, in one sentence she can relay).

**Fold-in against wave6/04: trivial for the spine, moderate for the body.**
The verdict hero (`verdict()` + the `.hero` block, ~40 lines) reads the same
three facts wave 6 already has and can sit above wave 6's page unchanged; the
three verdict cards (`vcard()`, ~25 lines) can replace the status-striped
cards one-for-one, same data, same primary (partner's Check now). Regrouping
wave 6's `factsGroup`/`trustGroup` into `condBlock` is a moderate rewrite
(~120 lines) that reuses every row; the sheets are identical; "Through this
Mac" is wave 6's "You on this server" + "Groups" merged.

## Take 3 — You on each server (`03-identity.html`, standalone chrome)

**Thesis.** Nobody has a server; they have an identity on one. The page is a
map of you: one lane per server, and in it your account there, the Macs that
can act as it, the vault it owns and the groups it reaches, as a tree. Server
facts are the lane's *footing* — a drawer on the right — and matter only as the
ground an identity stands on.

**Optimises for** the join story and the "which me is this" question: rae vs
rae.chen are two identities in two lanes, each with its own groups, and the
empty partner lane says exactly what is missing (footing, then an account).
Lapsed and mismatch become a band across one lane while the others stay
usable, which is the correct blast radius drawn as geometry.

**Costs.** It is not a Servers page; it is closer to wave 5's Home. Three
equal lanes fit at 1180 but a fourth server would not; the drawer overlays
the map rather than sitting beside it. Server facts are secondary by design,
so Ade's flow (check, compare, reset) is two clicks deeper than in wave 6, and
the danger actions live in a drawer that can be closed. Vault item counts in
the lanes are a metadata listing per store — one call each, empty offline.

**Serves best** Sol and Marcus; worst for Ade.

**Fold-in against wave6/04: rewrite.** Standalone chrome: the lanes, tree and
drawer have no counterpart in `shell.css`, and the sidebar's "Servers &
devices" row becomes "You & servers", which touches 01-vault's redirect pane
and 05-settings' pointers. What moves cheaply is the *drawer's contents*,
which are wave 6's server page rows almost verbatim, and one idea: wave 6's
"You on this server" inset could become the identity tree (account → Macs →
vault → groups with roles, ~40 lines) without changing the page's shape.

## Take 4 — Table & inspector (`04-inspector.html`, standalone chrome)

**Thesis.** A network admin wants every server on one screen with the facts as
columns, and the selected row opened in a right-hand inspector — a mail
client's account list. Every fact is a cell, every cell says when it was
learned, and ↑ ↓ ⌘R ⌘C ⌘N drive it. Adding a server is an editable row at the
top of the table, not a sheet.

**Optimises for** scanning and comparison across servers (which pin is
oldest, which check-in expires first), keyboard speed, and Ade's whole loop
without leaving the screen: select, ⌘R, compare, reset at the bottom of the
inspector.

**Costs.** At 1180 wide, three panes and six columns are tight: the inspector
is 316 px and its hints wrap hard; the table drops "Last check" and "You" as
columns and folds them into the Answer and Server cells. It reads as an admin
console, which wave 5 rejected for the product's shape; a first-time user gets
a grid of dashes. The editable add row has no room for the trust-policy and
PROPOSED expected-id copy, which moves to the inspector.

**Serves best** Ade and Jun; worst for Sol.

**Fold-in against wave6/04: moderate for the inspector, rewrite for the
table.** `shell.css` already has the three-pane geometry (`.app.with-details`,
`.details` at 300 px) and the header/row grid; wave 6's server page rows fit
the `.details` panel with a narrow-key variant of `.srow` (~30 lines of CSS).
The keyboard handler is ~20 lines and could ship in wave 6's list as a
convenience (↑ ↓ between cards, ⌘R on the selected one). The dense table
itself (columns, sort, editable row) is new and needs the denser type wave 6
does not have.

## Recommendation

Fold **take 2's spine** into `wave6/04-servers.html`: the verdict hero above
the server page and the three verdict cards in place of the status-striped
cards. It is the cheapest change with the largest audit payoff — Marcus and
Sol get an answer before the facts, and the lapsed state finally says it is
*not* distrust. Keep wave 6's Host facts / Trust grouping underneath rather
than regrouping by condition, so Ade's scan stays intact.

Take two trivial pieces from **take 1**: the lease's own expiry time on the
lapsed row, and the Add sheet's "now / next" framing. If the desktop gains a
client-side record of its previous `ProbeReport`, add the row-to-row
comparison so "Last check against the pin" becomes a local fact and the
PROPOSED band shrinks to the agent's own verdict.

Take **take 4's** keyboard handler as a convenience in the list.

Do not fold **take 3**. It is a better Home than a better Servers page; it
belongs to the wave 5 Home discussion (You · Attention · Your groups), where
its lane could be the "You" tile.

## Verification

Every state of every file rendered at 1280×860 with Chromium: zero console
errors, `scrollWidth === 1280`, and the server pages also rendered scrolled to
the bottom. Files: 01 40 KB, 02 38 KB, 03 51 KB, 04 52 KB.

## Simpler takes (05, 06)

Takes 01–04 were judged too complex: dense rows of facts, a ledger, a
four-condition breakdown, drawers, a keyboard-driven table, paragraphs of
explanation. Two more takes ask the opposite question — what does this page
look like if a person understands every server's situation in two seconds
and sees more only when they ask. Both keep every BRIEF §2 rule, both run on
the wave 6 shell, both accept the same eight `?state=` values, and both use
the same status model: one status per server computed from the last
`ProbeReport`, the lease's `expires_at` and the typed history-check error,
said in plain words — *Fine · checked 27 Aug*, *Can't be read · since 30 Aug*,
*Never checked · added 1 Sep*, *Untrusted / Don't trust this · since 2 Sep*.
No "pin", "checkpoint", "lease", "probe" or "canonical" on a first screen;
those words appear only inside the Details disclosure. Client-side call
times are flagged once in each header comment, as in takes 01–04.

### Take 5 — Status list (`05-status-list.html`)

**Thesis.** The list answers "is each server fine, and if not what do I do":
a row per server with the avatar, the name and who you are there, one
status chip with its date, and — only for a server that is not fine — a
second line of one sentence and one button. Fine servers say nothing more.
A server's page is five plain rows, one sentence each, one action, and a
single Details disclosure. The register is System Settings › Internet
Accounts.

**What was cut, and where it went.** From the list: the three-line status
prose, the "You: rae · personal and 2 groups" line, the "whoever runs a
server cannot read what is in it" paragraph, and the Your Macs section
(now a collapsed disclosure under the list). From the page: the You / Host
facts / Trust / Groups sections become five rows (Status, Trusted since,
Check-in, You, Groups); the full host id and Copy, the name the server
answered with, the signed-history numbers, the out-of-band compare, the
PROPOSED acceptance note, your Macs with ids and the raw last response all
move under Details. The Add sheet drops the trust-policy row and the
expected-host-id PROPOSED band (two fields and one sentence remain). The
Reset sheet drops from five rows and a paragraph to Discarded / Kept / Type
its name.

**Fact count.** List: 2 per fine server (who, status with date), 3 per
server that is not fine (plus one sentence and one button). Page: 5 rows +
1 action for a checked server with an account (Check again in the toolbar);
5 rows for the lapsed server; 2 rows + Check now before the first check;
4 rows + Create an account… just after it. Above the fold nothing else but
the collapsed Details link and the Danger box.

**Fold-in against wave6/04: moderate, and mostly deletion.** Shared as-is:
shell, `sidebar`, `pageHeader`, `openSheet`, `flash`, `review`,
`getState/setUrl`, `setLease`, the fixture extension, the compare widget,
`veil()` and the event table. New: `status()` (12 lines, replaces
`health()`), `listRow()` (10 lines, replaces `serverCard()`), `mainRows()`
(15 lines, replaces `youGroup`/`factsGroup`/`trustGroup`/`groupsGroup`
above the fold), and ~25 lines of layout CSS. Wave 6's existing rows fit
the Details inset almost verbatim, so the disclosure is a move, not a
rewrite. The sheets shrink rather than change shape.

### Take 6 — Verdict cards (`06-verdict-cards.html`)

**Thesis.** Each server is one card whose first line is the verdict in plain
words with its date, then at most two supporting sentences and one action;
the server page is the same card enlarged over four plain rows and a
Details disclosure. It is take 02's "answer before the facts" without the
four conditions, the explanation paragraph or the condition blocks — the
simplified descendant the feedback asked for.

**What was cut, and where it went.** From take 02's card: the four-condition
list and the six-line "why" paragraph (the verdict is now one or two
sentences); the "What a Yes rests on" prose and the Macs inset below the
grid (Macs are a collapsed disclosure). From take 02's page: the four
condition blocks with their insets — their facts go to the same Details
disclosure as take 05 — and the "Through this Mac" section becomes the You
and Groups rows. Everything else is identical to take 05, so the two are
directly comparable.

**Fact count.** List: 3 per card (who; verdict with date; one reason, which
the lapsed and untrusted cards say in two sentences) plus one action where
there is one to take. Page: the card (3) + 4 rows (Trusted since, Check-in,
You, Groups) + 1 action, i.e. under the six-row budget; 1 row before the
first check, 3 just after it.

**Fold-in against wave6/04: trivial for the list, moderate for the page.**
`card()` (12 lines) and 14 lines of card CSS replace `serverCard()`
one-for-one, on the same data; `verdict()` is 8 lines where take 02's
`conditions()` + `verdict()` were 25. The page fold-in is take 05's: the
hero sits above whatever rows are kept, and the Details move is the same.
If wave 6's Host facts / Trust grouping is kept underneath instead of the
four rows, the fold-in is the hero alone (~25 lines).

### Recommendation, revised

The earlier recommendation was to fold take 02's spine — its verdict hero
and three verdict cards — into `wave6/04-servers.html` and keep wave 6's
Host facts / Trust grouping underneath. The simple takes make that cheaper
and go further: take 06's card *is* that hero with the conditions removed
(the cards drop from ~80 lines to ~25 and lose nothing a newcomer reads),
and take 05 shows that wave 6's four fact sections read better as five
rows with the operator material one disclosure down than as a scrolling
page. So: fold take 06's card into the list in place of wave 6's striped
cards, fold take 05's five rows + Details into the page in place of wave
6's four sections (wave 6's rows survive verbatim inside Details, including
the compare widget and the PROPOSED note), and shorten both sheets as these
takes do. Keep the wave 6 chrome. Of the two, take 05 is the calmer list —
a fine server says nothing — and take 06 is the louder one when something
is wrong; the combination above takes the loud card and the calm page.
Takes 01 and 04 remain a source of two conveniences (the lease's own expiry
time, which both simple takes already show as "since 30 Aug", and the list
keyboard handler); take 03 stays a Home idea.

## Verification (05, 06)

Every state of both files rendered at 1280×860 with Chromium: zero console
errors, `scrollWidth === 1280`. Server pages were also rendered with Details
open and scrolled to the bottom (Danger stays last). Files: 05 24 KB, 06
24 KB; local `<style>` 42 and 46 lines.
