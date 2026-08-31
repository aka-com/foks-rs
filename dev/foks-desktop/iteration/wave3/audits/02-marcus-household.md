# Audit 02 — Marcus, household organiser

**Persona.** Marcus. Not technical. I keep the family's things in iCloud Drive and our
logins in Apple Passwords. I have never run a command in a terminal.
**Model.** Opus.
**Goal.** Make a "Household" group, put the guest Wi-Fi password and one PDF
(our insurance/emergency document) in it, share it with my partner Sam, and then
find and open that PDF later from the other Mac in the kitchen.
**Screens rendered.** App: Get started, Items (personal and team), Parties,
Settings, Notifications. Wave 1: Finder, Apple Passwords (group, manage, PDF,
reveal), Drive, 1Password. Wave 2: Teams hub (teams, create, add, store),
Onboarding (existing, phrase, team), System Settings, Servers & identity.

---

## Round two (wave 6)

Marcus re-walked the Household goal against `wave6/01-vault.html` and
`wave6/02-first-run.html`; his report is
`../../wave6/audits/02-marcus-round2.md`. Two of his five round-one
frustrations are Fixed (a real Choose file row, and locality stated word for
word on the file detail), three are Partly (the deferred band now arrives under
Save in before he types anything, but the group page itself still says nothing;
the rail and most of the word list are gone, but party, journaled, alias,
version guard and unlinks survive; the other-Mac question is answered in first
run and then points at Settings pages that do not exist), and none are Not.
His verdict: yes for the reading half, because it finally tells him where the
PDF lives, that it is not on this Mac and who else can open it, but not yet for
the family, because he cannot put anything into Household, has no sentence to
send Sam, and is sent twice to a page that does not exist.

## 1. The attempt against the app as built

A dark strip down the left with seven words: **Items, Notifications, Get started,
Stores, Parties, Servers, Settings**. **Parties** I read as birthday parties and
**Stores** as shops; neither is what they mean, and I only found out by clicking.

I clicked **Get started**. Four lines: "✓ Secure local state initialized",
"○ Add a FOKS server profile", "→ Probe the selected server before account
onboarding", "○ Create or resume an account store". I do not know what a
**profile** is or what **probe** means (it sounds medical). Nothing here mentions
a family folder, a group or sharing, so the checklist does not end anywhere near
my goal. I clicked through Add server and Probe anyway and got a grey box of
computer text containing `host_id_hex`, `host_chain_sequence` and `merkle_epoch`.
I have no idea whether that was good news.

**Making the Household group.** I eventually found it under **Parties** →
"Create a team store". The form wants four things: *Owner account alias*, *Local
team alias*, *FOKS team name*, *Pending team alias*. Three are the word "alias",
which I take to mean nickname, and I could not tell why one group needs three
names. I typed "household" into all of them and pressed **Create named team**,
guessing "named" was better than **ad-hoc**. Nothing said "Household is ready" —
the confirmation was more computer text at the bottom of the page.

**Putting the Wi-Fi password in.** On **Items** my personal list has a panel on
the right headed "New account-store item": a Path box, a Secret value box, and
three buttons — **Create file**, **Create folder**, **Create symlink**. "Symlink"
is not a word I know. When I clicked the group heading in the list instead, the
whole right-hand panel became one grey sentence, printed **twice**:

> Team stores are list/read-only in this release.
> Team stores are list/read-only in this release.

That is the end of my goal in this app. There is no way to put the guest Wi-Fi
password into Household. The sentence does not say *when* that changes or what I
should do meanwhile, and it calls a thing I created and own "read-only".

**Putting the PDF in.** There is no upload anywhere: no "+", no drag target, no
file picker. The nearest thing is **Create file**, which wants me to *type* the
contents into a box. You cannot type a PDF. Both halves of my goal are blocked,
and neither block is announced anywhere I would look first.

**Sharing with Sam.** Back on **Parties**, "Local team members" has three boxes
(*Named team*, *Username*, *Member visibility*) and six buttons in one row,
including Resume add, Demote to member and Resume edit. I would need Sam's exact
username on the same server, and nothing says Sam must have an account there
first or how Sam gets one. "Member visibility 0" is a number I cannot interpret;
the explanation calls it "an ordered authorization band".

**The other Mac.** This is where I got most lost. **Settings** has "Software
owner devices" with **Provision owner device** and the sentence "Provisioning
creates *another local owner credential*". I read "device" and thought "my
kitchen Mac" — but "local" seems to mean *this* machine, and the boxes want a
"Device serial" of 2. Nothing tells me to do anything on the other Mac. Below it,
"Owner recovery" offers a 17-word backup phrase and **Recover account** with
boxes for "Recovered account alias" and "Recovery device serial". So no screen
here says "here is how to use your other Mac", and the one that sounds like it is
about something else.

**Is the PDF on this Mac?** No screen says. Selecting a big team file shows
`Size 84399718` (raw digits, not "84 MB") and a blue **Reveal value** button —
the same button as for a password. Nothing says the file is fetched from the
server, that it is not stored here, or that the list itself is a live request
that shows nothing when the server cannot be reached. If our internet were down
I would think the family folder had been emptied.

---

## 2. The same goal against the wave 1 / wave 2 mocks

**What got me furthest: wave 2 / 03 "Onboarding" for the setup and the group,
and wave 1 / 04 "Apple Passwords" for living in it afterwards.**

*Onboarding, step 4 "Join or create a team"* is the first screen in the set
written for me: a plain paragraph saying sharing anything means putting it in a
team, one field labelled **Team name** pre-filled with "Household", and beside it
a card "**Ask to be added**" holding the exact sentence to send — *"Add rae on
foks.example.net as a Member"* — with a Copy button. That is what I would send my
partner.

*Onboarding, "Add this Mac to your account"* is the only screen anywhere that
answers my kitchen-Mac question in my own words: three tiles, "Another device
approves this one", "Recover with a backup phrase", "Use a YubiKey", with "Start
here, then finish on a Mac that already holds the account." (See §5 — I could not
find the matching button in the app.)

*Apple Passwords* is my daily app, and this mock is it. Household sits under
**Shared Groups** with faces and a **Manage** button, and the item page ends with
a sentence I understood: *"Sharing means this item lives in Household's store …
there is no per-item share — who can see it is who is in the group."* Uniquely in
the whole set, under the masked value: *"Showing reads version 4 from
foks.example.net — **the value isn't kept on this Mac**."* That is the sentence I
had been missing.

Where it still failed me: the header says "2 **parties**", the Manage sheet talks
about a "**visibility** band … 0 is the widest band" and "Adding is a **journaled**
operation", and the item's top-right reads "Group items are read-only in this
release" with a greyed **Edit** and an amber **deferred** tag — so I still cannot
add the Wi-Fi password. The PDF page has a **Download** button, good, but its only
explanation is "Read in chunks bound to version 7", which never uses the plain
words the secret row uses two inches away.

*Drive (wave 1 / 05)* is best for the PDF half: folders, a real **Download**
button, a "WHO CAN SEE THIS" panel listing people, and a greyed toolbar item
saying **"Uploads to shared drives are deferred"** — the truth up front rather
than after four clicks. *1Password (wave 1 / 03)* states the same blockage
plainly: its "New item" sheet lists Household with a **DEFERRED** tag.

*Teams hub (wave 2 / 02)* has the best group-creation sheet (Server, As account,
Named vs Ad-hoc each explained in a sentence) but still asks for two names,
"Alias on this Mac" and "FOKS-visible name"; its **Store** tab shows a card
reading "**WRITES FROM THIS APP — List and read only**". *Finder (wave 1 / 01)*
is familiar to use, but every "New…" entry is greyed and its info panel shows
"Pinned at chain 12 · epoch 4821" and a 66-character "Team id".

*System Settings (wave 2 / 01)* has the best backup sentence: "It can recover the
account on a Mac that has none of your keys." *Servers & identity (wave 2 / 04)*
is the one I would never open: "Merkle epoch 4,821", "Compatibility **lease**",
"Probing is a network round trip".

---

## 3. Ranked frustrations and wins

**Frustrations**

1. I cannot do the thing at all — putting the Wi-Fi password or the PDF into the
   family group is impossible everywhere, and the app only says so after I have
   made the group and clicked into it.
2. No way to add a file from my Mac. "Create file" means "type it into a box".
3. "Parties" as a menu word, and "2 parties" for me and my partner — we are
   people. Also alias, probe, lease, store, symlink, ad-hoc, visibility band,
   journaled, tombstone, compare-and-swap, Merkle epoch.
4. The other-Mac question has no answer in the app, and the button that sounds
   like the answer ("Provision owner device") means something else.
5. Nothing tells me the group's files are not on this Mac, or that an empty list
   may mean "could not reach the server" rather than "nothing is here".

**What worked**

1. Onboarding step 4's "Ask to be added" card with the exact sentence to copy.
2. Onboarding's "Add this Mac to your account" — three tiles, plain words.
3. Apple Passwords' "the value isn't kept on this Mac" line and its one-sentence
   explanation of what sharing means.
4. Drive's greyed "Uploads to shared drives are deferred" *before* I invested any
   effort, plus a real Download button.
5. Teams hub's "FOKS has no join button" explainer and its "List and read only"
   card. I would rather be told no than click a dead end.

---

## 4. Three changes I would ask for

1. **"Let me drag a file into the family folder — and if I can't yet, say so on
   the folder itself."** One line at the top of Household, not in a side panel:
   "You can read what's in here. Adding things comes in a later update; for now
   put new things in your own list." Then let me drag the PDF in from Finder the
   way I do with iCloud Drive.
2. **"Ask me for one name, and call it a group."** One box, "Group name:
   Household", one button — not owner alias, local alias, FOKS name and pending
   alias. And call the menu "Groups", not "Parties".
3. **"Put 'Set up my other Mac' in the app, and tell me where the file lives."**
   A button saying exactly that, which shows me a code to type on the kitchen Mac
   or asks for the 17 words. And one line under every document: "Stored in
   Household on foks.example.net — not on this Mac. Download to open."

---

## 5. Things I was told that turned out to be wrong or missing

- **"Get started" hides the real first steps.** It offers "Add a FOKS server
  profile" as if I can do it here. `GAPS.md` finding 17 says keystore init and
  profile-add are **CLI-only** and that Get started is meant to say so. It never
  does, so I clicked a step that cannot start there.
- **"Reveal value" on a document is misleading.** `BRIEF.md` §1 says a File is
  read in version-bound chunks; `GAPS.md` finding 24 says a sync pulls every
  chunk of every large file. The app gives an 84 MB file the same blue *Reveal
  value* button as an eight-character password — no size in human units, no
  progress, no way to save it to disk, and no "open the PDF" anywhere.
- **Nothing warns that the list is live.** `GAPS.md` finding 11 says listing is a
  network call and that offline means an *empty* Items screen, not a stale one.
  No app screen says this; the only failure wording is "The last operation
  failed: local agent I/O failed", which I would not connect to my Wi-Fi.
- **The same sentence twice.** Selecting a team item prints "Team stores are
  list/read-only in this release." twice in the inspector. It reads like the app
  stuttering.
- **Onboarding promised a pairing flow I could not find.** "Another device
  approves this one — Start pairing" is exactly what I want, but the app's only
  device action is *Provision owner device* ("another **local** owner
  credential"), and `ENGINEERING-PLAN.md` §5 group D lists only
  provision/resume/backup/recover. If it does not exist yet, the mock should
  label it "deferred" like the others do.
- **"PQ slot" in Settings is not what it says.** `GAPS.md` finding 22 records
  that both YubiKey slots hold P-256 keys and the post-quantum label is invented.
  The app's field still says "PQ slot".
- **The removal warning is written for someone else.** "This writes a FOKS
  tombstone using the exact version shown in the catalog" is the most frightening
  sentence in the app and I could not tell you what it does to my file.
