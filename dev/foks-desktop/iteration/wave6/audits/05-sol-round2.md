# Audit 5, round two — Sol, first-time user with no context

**Persona:** Sol. sam.ortiz said "install this and I'll add you"; I still know
nothing about FOKS. **Goal:** first launch to "I am on Engineering and can see
the staging token", with nothing but the app.
**Walked:** `02-first-run.html` invited — boot, who, address, no-address,
checked, compare, error, account, existing, protect, phrase, waiting (Check now
pressed), added, checklist-invited; then own — who, address, account, protect,
create-group, done, checklist-own; then `01-vault.html` — show, join, attention.

## 1. Then and now

| # | Round-one frustration | Wave 6 | Evidence |
| - | --------------------- | ------ | -------- |
| 1 | Nothing says I need an address from sam; the placeholder looks like a real address to type | **Fixed** | `02/who`: "You need one thing from them: the **address of their server**." `02/address` is titled "What did sam send you?"; `02/no-address` hands me the sentence to send sam, with Copy. |
| 2 | Notifications said "No actions need attention" while I waited for an invitation that can never arrive | **Fixed** | `02/checklist-invited`: Attention badge 1, and its pane reads "Waiting for sam.ortiz to add sol — Engineering is not listed yet." |
| 3 | Parties opened on "Create named team" with `engineering` pre-filled — the obvious button made a second Engineering | **Fixed** | The invited path has no create step at all; `01/join` leads with "Nobody joins one on their own" before offering Create a group. |
| 4 | The Invite field looked required and nothing said whether sam's server needs one | **Fixed** | `02/account`: Invite under OPTIONAL, "Leave empty unless you were given one", "most servers don't need one… and then sam is who to ask." |
| 5 | After sam adds me, nothing tells me to press Refresh | **Partly** | `02/waiting` has Check now and says Engineering will appear under GROUPS — but the amber band above it says the operation that finds a group someone else added me to does not exist yet, and pressing it returns "Not yet, only Personal is listed." Today that is the only answer it can give. |

## 2. Fresh transcript

**boot.** "Preparing this Mac. Nothing to do yet." Three ticked lines, and
**agent** is defined in the same breath — "a private helper on this Mac".

**who.** The screen I asked for last time is here: "Someone invited me to their
group" / "I'm setting up on my own", and under the first, in order — you need
the address of their server; they add you by the username you pick in a minute;
you cannot add yourself, there is no link or code to wait for. Three of my four
questions, answered before I type anything. **Start here.**

**address.** "What did sam send you?" I have nothing yet: **They sent nothing?**

**no-address.** "Ask sam this", the exact sentence, a **Copy** button, and "Your
username is next — you choose it here and send it to them after." I paste it to
sam, get `foks.acme-corp.com`, and type it with no `:443` because the caption
gives the shape and says to type it exactly as given. **Check the server.**

**error** (I mistyped `.co`). "did not answer, so nothing was saved… Check the
address with sam — one missing letter is the usual cause." Unsticks me alone.

**checked.** "Found it. This Mac now recognises foks.acme-corp.com and will
refuse anything else that answers to that name. Nothing about you has been sent
yet." The amber band under it is about whether the app can say "new pin / same
as before / advanced", and today only **New pin**. It does not stop me;
`ProbeOutcome.acceptance` sits in brackets and I skip it. Below, "Compare with a
host id sam published" — an empty box. Sam published nothing, and nothing says
it is fine to leave it empty. I hesitate, then **Continue**.

**compare** (had sam sent one): green "Match. The id sam published is the one
this Mac pinned," plus "Compared on this Mac only."

**account.** Username `sol`, and under it, bold: **"This is what sam will type
to add you."** That one sentence is my whole second question. "This Mac's name"
is a separate field, so I never wonder whether an alias is also me. **Create my
account.**

**existing** (clicked out of curiosity): pair from a Mac I already use, or the
17-word phrase. The pairing card is tagged "not in the desktop yet" and yet
carries the only blue button on the screen — I would have pressed it.

**protect.** "Right now this Mac holds the only key to sol. Lose the Mac and the
account goes with it." I type a passphrase. Beside it the YubiKey card is three
warning blocks — cannot be split or resumed, factory management key, PUK — the
largest thing on the page, for the one option marked **Later** that I cannot
use; it made me think I was doing something dangerous. **Show my phrase**: 17
words, "Shown once. No copy button, on purpose," a checkbox. I write them down.

**waiting.** "Waiting for sam.ortiz to add sol." "Send sam this: *Add sol on
foks.acme-corp.com to Engineering*", with Copy. Then plain rows: Engineering
will appear under GROUPS in the sidebar; you'll see what your role lets you
read; quitting is fine. On the right, "Meanwhile: your own logins — put your own
logins in Personal now; nothing in it is visible to Engineering." That is
question four answered.

Then **Check now**. The amber band is *above* the button and phrased as a denial
— a group someone else added me to "cannot be found from here" — so I read it
as: the button below does not work. I pressed it: "Asked the server — nothing
new… Not yet, only Personal is listed." Now I cannot tell whether sam hasn't
added me or the app cannot see it, and nothing covers the second case. That is
where I message sam and stop.

**checklist-invited.** I quit and reopen. "Get started · 3 of 5", every step
where I left it, step 5 still "Waiting for sam.ortiz to add sol" with Copy the
sentence and Check now. Nothing was lost — exactly what I wanted.

**added.** "You're in Engineering. sam.ortiz added sol as a Member. This Mac
asked the server just now and Engineering was in the answer: its 4 items are
listed, and what your role can read is open." Engineering sits under GROUPS. I
click **staging-token**, press **Show**, and the panel says it read version 3
from foks.acme-corp.com and nothing is kept on this Mac. **Goal met.** The same
amber band repeats under the banner, which now reads as the app denying what
just happened.

## 3. Remaining issues

| # | Severity | Where | What | Ask | Needs protocol? |
| - | -------- | ----- | ---- | --- | --------------- |
| 1 | blocker | `02-first-run.html` · `waiting`, `checklist-invited` | The band says a group someone else added me to cannot be found from this Mac today, so the invited path never completes: Check now can only ever answer "only Personal is listed", and no fallback is offered. | "If the app can't find the group yet, say so in one line and tell me the one thing that does work — even if it's 'ask sam to do X'. Don't leave me pressing a button that can't succeed." | yes |
| 2 | major | `02-first-run.html` · `waiting` | The Proposed band sits above the Check now button and is phrased as a denial, so it reads as "this button is broken" rather than "this is being built". | "Put the yellow box under the button, and write it as what's coming, not as what I can't have." | no |
| 3 | major | `02-first-run.html` · `waiting` | Nothing says when to check, or whether reopening FOKS checks by itself, so I don't know whether to sit here or come back tomorrow. | "Tell me 'this checks each time you open FOKS' or 'you have to press this' — one of the two." | no |
| 4 | minor | `02-first-run.html` · `added` | The same Proposed band is repeated below a banner saying the check just worked. | "Once I'm in, stop telling me the thing that happened is impossible." | no |
| 5 | minor | `02-first-run.html` · `checked` | "Compare with a host id sam published" appears with an empty field and no line saying it is fine to have nothing to paste. | "Add 'Nothing to compare? That's normal — carry on.'" | no |
| 6 | minor | `02-first-run.html` · `protect` | The YubiKey card is the biggest, reddest thing on the screen, for the one option marked Later that I cannot use today. | "Fold the YubiKey warnings behind a 'What to know first' link. Don't make the option I can't take the scariest thing on my first day." | no |
| 7 | minor | `02-first-run.html` · `existing` | The only primary blue button on the screen, Start pairing, is on the card tagged "not in the desktop yet". | "Don't make the button that doesn't work the blue one." | no |
| 8 | minor | `01-vault.html` · `join` | The page lists my usernames per server but has no Copy button for the sentence to send, unlike `02/waiting`. | "Same Copy the sentence button here as in the first run." | no |
| 9 | minor | `01-vault.html` · `all` vs `02/added` | Following "Open your vault" lands me in a vault belonging to rae, with Work (Acme), Household and Homelab — none of which I have. | "The vault I land in should be the one the setup just built: Personal and Engineering, me." | no |

## 4. Anything wrong or missing about FOKS

- **Member counts.** The sidebar prints "7 people" per group and the header
  repeats it. BRIEF §2 gives a team summary no member count, so the number must
  come from a roster read per group per view — a second network call the screen
  never admits to, on a surface whose own rule is "nothing refreshes on its own".
- **The Added banner.** "This Mac asked the server just now and Engineering was
  in the answer" narrates exactly the operation the adjacent band says does not
  exist. Both are on screen at once; only the band is true today.
- **Invite copy.** "Most servers don't need one — a server that does will say so
  when you continue" is a claim neither BRIEF §2 nor WAVE4-BRIEF §1 supports.
  Better than silence, but the one sentence in the run I could not check.
- **Corrected since round one, and right now:** the passphrase no longer claims
  an idle lock; the existing-account screen offers a Mac, not a phone, and tags
  pairing "not in the desktop yet" per WAVE4 §1; the backup phrase is 17 words,
  shown once, no copy button, with a checkbox; roles are only Member / Admin /
  Owner and visibility reads "0 is the default; lower bands see less"; Reset
  hard state and raw JSON never appear on my path.

## 5. Would I use it

Yes — every question I had to message sam about last time is now answered on the
screen where it arises, and the only thing standing between me and Engineering
is that the app admits, in yellow, that it cannot yet see the group sam put me
in.
