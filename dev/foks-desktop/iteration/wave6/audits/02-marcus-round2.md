# Round two — 02 · Marcus, household organiser

**Persona.** Marcus. Not technical. iCloud Drive and Apple Passwords.
**Goal.** Make a "Household" group, put the guest Wi-Fi password and our emergency PDF in
it, share it with Sam, then find and open that PDF from the kitchen Mac.
**Screens rendered** (1280×860, no console errors). All 18 `01-vault.html` states and all
16 `02-first-run.html` states, both paths. Also `wave5/05-settings.html?state=macs`, for the
kitchen-Mac question wave 6 does not cover.

---

## 1. Then and now

**1. "I cannot do the thing, and only find out after making the group." — Partly fixed.**
The New sheet puts an amber band under **Save in** the moment Household is picked, before I
type anything: writing into a group's store comes later; today items in Household are listed
and read, and new items go in Personal. Right message, right moment, workaround named. But
the Household page itself (`?state=group`) still says nothing — my ask was one line on the
folder, not inside a sheet I have to open — and the band is signed "Deferred — plan §5
group B", a plan number that makes it read like a bug reference.

**2. "No way to add a file from my Mac." — Fixed on the surface.** `?state=new-file` has a
**File** row with a real **Choose file…** button and a Path under it. Still no drag from
Finder, and for Household it sits under the deferred band.

**3. "Parties, and the whole word list." — Mostly fixed, two leaks.** The rail is gone; the
sidebar reads All items / VAULTS / GROUPS / Attention and the footer says **Join or create a
group**. *Probe* and *lease* are demoted into brackets — "the last check (a probe)", "the
check-in (compatibility lease)" — exactly right, and visibility is explained where it is
used. Symlink, tombstone and compare-and-swap are gone. But a Personal item's Sharing says *"sharing anything means putting it in a
**team**, whose members are **parties** with roles"* — the word I complained about,
undefined, in the sentence explaining sharing, calling it a team when everything else says
group. Manage adds *"A **party** that is not a user of this server…"*. **Journaled** is now
in five places and never explained; **alias**, **version guard** and **unlinks** survive in
Settings, the Remove sheet and the agent-lost notice.

**4. "The other-Mac question has no answer." — Partly, then it breaks.** The first run's
**Add this Mac to your account** (`02-first-run?state=existing`) is the best screen here:
*Pair from a Mac you already use* and *Recover with your 17-word backup phrase*, each with a
plain Requires list. But it only appears during a first run on the new Mac, and it says *"On
the Mac that already has your account, open **Settings › Devices › Set up another Mac**."*
No Devices page and no such button exists. `?state=servers` lists "MacBook Pro · this Mac"
and "Travel Mac" with 14 characters of hex, no explanation, and only **Add a server…**
beneath; `?state=settings` has Accounts, Security keys and Start over — no phrase, no
recovery, no other Mac. The "Household exists" screen even says "Set up another Mac from
this one lives under Servers & devices". It does not.

**5. "Nothing tells me the files are not on this Mac." — Fixed, word for word.**
`?state=file`: **2.8 MB · version 7**, a **Download** button, and *"Stored in Household on
foks.example.net — not on this Mac. Download reads version 7 in bounded chunks."* Show on a
password says it reads version 4 and nothing is kept here. The lapsed check-in screen says
items *"are not listed rather than merely uneditable"*, so an empty list is explained. My
biggest ask, and I got it.

---

## 2. Fresh transcript

**Who is setting you up?** Two cards, one paragraph, no jargon. I pick *I'm setting up on my
own*: server address, a check I did not have to understand, username, this Mac's name.
**Protect it** shows a passphrase, a YubiKey and the 17 words side by side; the phrase sheet
says "Shown once. No copy button, on purpose" and makes me tick a box. I would actually
write them down.

**Step 5 of 6 — Create a group.** One box: **Group name: Household**. My round-one ask,
granted. Named vs Ad-hoc each get a sentence. Then a paragraph ending "Creating a group is
journaled" — a word I do not know. **Create group** → **Household exists**: I am Owner, add
people from Manage by username, they need an account on foks.example.net first, send them
the address, ask for the username. The whole story in four lines.

**Open your vault.** Household sits under GROUPS with a face. **+ New** → Save in →
Household, and the amber band appears at once: not yet, put it in Personal for now.
Disappointing, but honest before I typed. **New file** the same, with **Choose file…**. The
fixture already holds both, so I see the destination: guest-password with **Show** and a
**Copy** on the network name; emergency.pdf with **Download**, 2.8 MB, and the "not on this
Mac" line. That is the app I wanted.

**Sharing with Sam.** **Manage** on the Household header: one **Username** box hinted "their
username on foks.example.net", three roles with a sentence each. What I cannot get here is
Sam started — no "here is the sentence to send", the way the invited path's waiting screen
has *"Add sol on foks.acme-corp.com to Engineering"* with **Copy the sentence**. **Join or
create a group** lists my server and username as two bare lines with nothing to copy.

**The kitchen Mac.** Here I stop. Nothing in the vault offers to set up another Mac, and the
two pointers to it point at a page that is not there. Wave 5's **Your Macs & recovery** is
exactly right — "Add another Mac", *Set it up from this Mac* (greyed, "not in the desktop
yet") and *Use the backup phrase on the other Mac* — but wave 6 leaves it out. So the only
door back to my account from the kitchen is a small blue link, *"I already have an account
on this server"*, under the sign-up form. Miss it and I make a second account, and Household
is not there.

---

## 3. Remaining issues

| # | Severity | Where | What | Ask | Needs protocol? |
| - | -------- | ----- | ---- | --- | --------------- |
| 1 | blocker | `01-vault?state=servers`, `?state=settings` | No "set up another Mac" anywhere, and no backup-phrase or recovery row. Servers & devices lists "Travel Mac" plus hex with no explanation and only **Add a server…**. | "Put wave 5's **Your Macs & recovery** page in. One place that lists my Macs in words, offers 'Add another Mac' both ways, and says whether my 17 words are set." | no |
| 2 | blocker | `02-first-run?state=existing`; `?state=done` "Household exists" | Both send me to **Settings › Devices › Set up another Mac** / "under Servers & devices" — neither exists. Directions to a room the app does not have. | "Don't send me to a page you haven't built. Build it, or say 'not in the app yet — use your 17 words on the other Mac'." | no |
| 3 | major | `01-vault?state=group`, every group page | The one place I look — the Household folder — never says I cannot add to it; the truth is only inside the New sheet. | "One line at the top of Household: 'You can read what's in here. Adding comes in a later update; for now put new things in Personal.'" | no |
| 4 | major | `01-vault` Manage sheet; `?state=join` | No sentence to send Sam and no Copy button. The invited path has exactly this; the owner's side does not. | "Give me a **Copy the invitation** button: 'Install FOKS, go to foks.example.net, make a username, then tell me what it is.'" | no |
| 5 | major | `01-vault` Personal item Sharing; Manage hint | "…putting it in a **team**, whose members are **parties** with roles"; "A **party** that is not a user of this server". Undefined, and "team" contradicts "group" everywhere else. | "Say group, and say people. If you must say party once, say what it means in the same breath." | no |
| 6 | minor | Both files, ~13 places | **journaled** is never explained; so are **version guard**, **unlinks**, **alias**, **ad-hoc**, and **epoch / chain / host id** on Servers. | "Say 'safe to stop and pick up again' instead of journaled, and keep the code words in a Details box." | no |
| 7 | minor | `01-vault?state=new`, `new-file` | The band is signed "Deferred — plan §5 group B". | "Drop the plan number, or put it after a dash where I can ignore it." | no |
| 8 | minor | `02-first-run?state=account`, `?state=who` | "I already have an account on this server" is a small blue link under a sign-up form; neither card on the first question says "I already use FOKS on another Mac". | "Make that a third card on the first question." | no |
| 9 | minor | `01-vault?state=file` | **Download** never says where it lands or that a copy then sits on this Mac. | "Say 'Saves a copy to your Downloads folder.'" | no |

---

## 4. Anything wrong or missing about FOKS

Against BRIEF §2 and WAVE4-BRIEF §1 the facts hold up better than anything I have been shown
so far. Items carry only path, kind, version, size, read role and write role — no dates, no
owner names, no sync ticks I would have believed and been wrong about. Visibility uses the
corrected wording, not the old "0 is the widest". The add sheet offers Member, Admin and
Owner and says a promotion means removing and re-adding. Refusals are refusals: "There is no
'create anyway'", "There is no 'save anyway'". The 17 words are shown once with no copy
button. "Check now" carries its Proposed band saying a group someone else added me to cannot
be found from here — the most useful warning on the invited path.

Two things are off. **Device pairing**: WAVE4-BRIEF §1 says it is wire-backed and should be
drawn; wave 6 draws it once, on a first-run screen I only reach on a fresh Mac, and both
mentions route me to a Settings page wave 6 does not contain. **Vocabulary**: WAVE4-BRIEF §3
allows "parties" once, in a roster header, defined; wave 6 uses it twice in body copy
undefined and mixes "team" into a product that otherwise says group. Smaller: the
**Servers & devices** row opens two different pages depending which file I am in — the
first-run one carries the pairing sentence, the vault one does not.

---

## 5. Would I use it?

Yes for the reading half — it finally tells me where my PDF lives, that it is not on this
Mac, and who else can open it — but not yet for the family, because I still cannot put the
Wi-Fi password or the PDF into Household, I have no sentence to send Sam, and the app twice
sends me to a page that does not exist when I ask about the kitchen Mac.
