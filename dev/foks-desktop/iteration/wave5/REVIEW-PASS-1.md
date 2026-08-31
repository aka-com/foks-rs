# Wave 5 — independent review, pass 1

Read against `DESIGN.md` §1, `../BRIEF.md` §2, `../WAVE4-BRIEF.md` §1–§3, and the
five wave 3 audits §4. All thirty-one states render with zero console errors and
`scrollWidth` 1280. Findings are ranked most severe first.

---

**1. `06-attention.html` ?state=all says deploy-bot was removed from Engineering;
`03-teams.html` ?state=people shows it still on the roster.** The Rotate card
reads "deploy-bot was removed from Engineering and the team rekeyed… the list was
computed when deploy-bot was removed". Teams lists deploy-bot as party 6 of 6,
Member · 0, with a live *Remove…* button, and Items' chips say "Readable by 6 of
6". One of the two is describing a world the other does not have, and it is the
count DESIGN §1 asks the reviewer to check. Fix: make the reminder card about
`dana.okafor` (whose removal sheet in 03 already promises exactly this reminder —
"Rotate 3 items dana.okafor could read"), or add a seventh, already-removed party
to the fixture and compute the card from it.

**2. The visibility band runs in opposite directions in two files, and band ≥ 1 is
a dead end.** `03-teams.html` (?state=add, demote, admit) clamps every stepper to
`[-0x4000, 0]` — no party can ever be set above 0. `02-items.html` ?state=new
clamps the item read-role stepper to `≥ 1` and offers "Members at band 1 — a band
above 0 admits only Members raised to it". Nothing in the application can raise a
member to band 1, so that read role is readable by nobody, and WAVE4-BRIEF §1
names only `Member(-0x4000) < Member(0) < Admin < Owner`, i.e. bands above 0 are
invented. Fix: give the New sheet the same `[-0x4000, 0]` clamp and relabel the
option "Members at band N or above (N ≤ 0)", so both steppers move on one axis.

**3. `05-settings.html` ?state=keys prints a fact the wire does not carry, and
then denies it twice on the same screen.** The enrolled-key row reads "primary key
· Connected · serial 20993145 · foks.example.net · enrolled from this Mac". Its
own footnote says the list "cannot say which plugged-in card is which alias", and
the Connected-now row below says "Which alias it belongs to, if any, is not
carried by either list." This is precisely the defect Ade logged against
`wave3/01`. Fix: the enrolled row shows the alias only; move serial and server
into an *Inspect* disclosure, and drop the Connected chip from it (the
Connected-now section is where a plugged-in card belongs).

**4. `03-teams.html` ?state=teams puts a button labelled "Join a team" 250 px
above the sentence "FOKS has no join button and no invite links."** The button
only scrolls to that card, but a newcomer reads the label, not the handler. It
also breaks DESIGN §1's vocabulary contract. Fix: relabel it "Not seeing a team?"
or delete it — the card beneath already carries the whole story.

**5. Home's tiles disagree with the rail on the same screen.**
`01-start.html` ?state=home: the "You" tile says "2 servers" over three server
rows (two accounts plus the never-checked `foks.partner.dev`); the Attention tile
says "3 things — most serious first" while the rail badge beside it says 4, and
the tile's three rows are `FX.notifications` (which include the lease — an item
`06-attention.html` classifies as *waiting*, explicitly not counted by the badge).
Fix: label the tile "2 accounts on 3 servers", and build the Attention tile from
the same needs-you set the badge counts (homelab · dana · admission · protect),
with the lease shown as a banner rather than a counted row.

**6. The set's default views hide the fixture's headline safety state.**
`FX.servers[acme].lease.state` is `lapsed`, yet `02-items.html` (?state=file,
?state=new) and every state of `03-teams.html` silently set the lease fresh so
Engineering can be listed — so most default screens show a readable team on a
server the shared fixture says is refusing every operation. Each file discloses
this in its review note, which is invisible to anyone deep-linking. Fix: keep the
override, but give 02 the persistent review-strip chip 03 already has
(`lease on acme: fresh`).

**7. Selected rows lose their secondary text.** `02-items.html` ?state=login and
?state=file: the selected row's `.sub` (the item's parent path — `/logins`,
`/release`) renders dark grey on the strong blue selection and is unreadable.
`03-teams.html` ?state=people: the selected deploy-bot row's "Role at source"
`.faint` em-dash renders as an illegible smudge. 03 has
`.table tr.on td .sub{color:inherit}` and 02 does not; neither covers `.faint`.
Fix: add `.table tr.on td .sub, .table tr.on td .faint, .table tr.on td .tiny
{color:inherit;opacity:.85}` to `system.css` so it holds everywhere.

**8. "profile" escapes into user-facing copy in `04-servers.html`.** DESIGN §1
puts *profile* in the parenthesis column behind *server*. The Add-server sheet
leads with "Adding writes a profile on this Mac", the Forget row says "retains the
durable profile directory", and the confirmation toast says "Profile added". Fix:
"Adding writes this server's record on this Mac"; "the durable directory for this
server (its profile)"; "Server added".

**9. One destructive action, two different sheets.** `04-servers.html` ?state=rollback
and `06-attention.html` ?state=rollback both open Reset. 06's names the operation
it discards ("Setting up Homelab") and itemises kept/lost in five dotted rows;
04's is a paragraph, and its confirm field is placeheld "Type it exactly" rather
than the server name it is asking for. 04 also auto-opens the destructive sheet on
page load, which is the opposite of Ade's "somewhere I cannot hit it by accident".
Fix: lift 06's sheet into `shared.js` and call it from both; make the placeholder
the server name; on 04 have the rollback state land on the banner with Reset
in the Danger group, not on an open sheet.

**10. `06-attention.html` ?state=empty promises six sources "and from nowhere
else", but the inbox carries a seventh.** The "Protect your account" card's source
is "kept on this Mac · Get started, step *Protect it*, skipped", which is not one
of the six listed. Fix: add a seventh row — "Steps you skipped in Get started ·
kept on this Mac" — or fold Protect into the Reminders row and say so.

**11. Pairing is drawn with a live blue primary on a row that says it does not
exist.** `05-settings.html` ?state=macs: "Set it up from this Mac" carries the
chip *not in the desktop yet* and a `btn primary` **Start…**. WAVE4-BRIEF §1 says
to draw pairing but say the desktop does not expose it; a filled blue button is
the strongest possible claim that it does. Fix: make Start… a plain `btn` and let
the sheet's Deferred band carry the primary. Related: `01-start.html`'s Get
started step 5 is "Set up your other Mac **for rae.chen**", but Settings › Your
Macs is scoped to `account personal` with no account switcher, so following that
step lands on the wrong account's device list.

**12. The same object is drawn two ways.** `01-start.html`'s team card (avatar
stack, role chip, amber tint when inactive) and `03-teams.html`'s (status chip
top-right, role inline in a muted sentence, no tint) share a fixture and a
stylesheet but not a design. Fix: promote one into `shared.js` as `teamCard()`.

**13. The New sheet's summary line — the one line that says what will be written —
is truncated.** `02-items.html` ?state=new footer: "Secret at
/deploy/preview-token · read Admin · write Owner · created only if nothing is th…".
`.sheet .foot .left` is `nowrap` + ellipsis. Fix: let the footer wrap to two
lines, or move "created only if nothing is there yet" above the buttons.

**14. Two "N of M" chips on adjacent screens count different things.** Items shows
"Readable by 6 of 6" (parties); Teams shows "Reads 3 of 4" (items). Both read as
the same idiom. WAVE4-BRIEF §3 and DESIGN §2 specify "Readable by N ▾" for the
item chip. Fix: restore "Readable by 6 ▾" on the item chip and keep "Reads N of M"
only in the roster column, where M is stated in the header ("Reads · of 4 items").

**Smaller things.** The Items list sorts on the full path but shows leaf names, so
the default order looks random and no header is sortable. `Work (Acme)` shows a
blank count where every other store shows a number — empty is indistinguishable
from unknown. The Rotate card says "3 **secrets**" over a list containing
`bundle.tar` and `README.md`; 03's remove sheet promises "3 **items**".
`02-items.html` ?state=file shows two blue primaries at once (New, Download).
"alias" labels both the account alias (04, "Alias on this Mac") and a device alias
(05, "alias laptop" under a header reading "account personal"). `generation 6`
appears on every party row and is defined nowhere — Jun asked and still has no
answer. And `deploy-bot` was changed from the BRIEF fixture's
`locally_manageable:false` to `true`, quietly deleting the case Jun's first ask
was written about.

**Auditor asks — where each is answered.** Priya 1 → 02 ?state=new (Who can read /
Who can change, live under the Deferred band that says no command writes a team
item today). Priya 2 → the rail everywhere + 03 ?state=add (Member/Admin/Owner) and
?state=demote ("no promote-in-place on the wire"). Priya 3 → 03 ?state=invite —
**partial**: the message carries server address, `firstname.lastname` and "then
tell rae your username", and marks the signup invite optional, but there is no
install link. Marcus 1 → 02 ?state=file, Deferred band on the team store —
**partial**: it names where to put things meanwhile, but there is no drop target
and no line on the folder itself. Marcus 2 → "Parties" is gone everywhere;
03 ?state=create is still three fields, not one box, and the word is "team", not
"Groups" — **partial, by DESIGN's choice**. Marcus 3 → 05 ?state=macs + 02's
locality line. Jun 1 → 03 ?state=people panel and ?state=remove (works for the
unmanageable `homelab` party). Jun 2 → 02 ?state=new. Jun 3 → 03 party panel
"Connect an agent" + 05 Account › Agent › Socket. Ade 1 → 04 ?state=server Trust.
Ade 2 → 04 Danger + 06 reset sheet. Ade 3 → 03 ?state=federation. Sol 1 → 01
?state=who. Sol 2 → 01 ?state=waiting + 06 ?state=all waiting card. Sol 3 →
**partial**: raw JSON is behind Inspect and Reset is behind a typed confirmation,
but "Create team" is still the blue primary on Teams, the same blue as "Add
server…" on 04 — the half of the ask about colour is unanswered.

---

## What is right

The spine holds. Four destinations and two utilities, the same rail with the same
badges and the same two-line who-footer on all six files; Stores and Parties are
gone as destinations and nothing mourns them. The safety states outrank their
pages — 02's lapsed store refuses instead of greying, 03's team page shows an
empty roster with a reason, 04's rollback goes inert beneath a banner carrying
its "gated · plan §5 group E" tag. Every write names a version guard, every
server fact says "as of the last check", every Show says what it reads and from
where, every File says where it lives and how many chunks it takes, and
"who can read this" is computed from `readersOf` in four places rather than
written as prose. The Deferred and Proposed bands are used sparingly and each
names what works today. 03's party panel and remove sheet are the strongest
screens in the set: they answer "what did this thing reach?" with a per-item
ledger and two named levers, and they say plainly what cannot be done from this
Mac and why. 01's first question, and the waiting page behind it, are the clearest
statement of the join story anyone has drawn in five waves.
