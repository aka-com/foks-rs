# Round two — Priya, engineering lead

**Persona:** Priya, engineering lead at a 12-person startup; three years on 1Password Teams.
**Goal:** set the team up on `foks.acme-corp.com`, add two engineers with the right roles, put a
production token where only admins read it and a staging token everyone reads, and tell a new
hire how to get in.

Rendered: `01-vault.html` at `all`, `personal`, `password`, `show`, `group`, `manage`, `new`,
`new-file`, `grid`, `lease`, `exists`, `conflict`, `attention`, `join`, `servers`, `settings`,
plus clicked-through Engineering (list, production-token details, its Manage sheet, the New
sheet with Engineering chosen); `02-first-run.html` at `who`, `address`, `checked`, `account`,
`protect`, `waiting` (both paths), `added`, `create-group`, `done`, and both checklists. Wave 6
has no Federation, Teams page or full Settings, so I read `wave5/03-teams.html` and
`wave5/05-settings.html` for those.

## 1. Then and now

| Round-one frustration | Verdict | Evidence |
| --- | --- | --- |
| Team writes deferred, and only one grey sentence about it | **Partly** | `01-vault ?state=new` and New-from-Engineering: the whole flow is live, Engineering is selectable and pre-selected, the button says "Create in Engineering", and one amber band says "Deferred — plan §5 group B … Today, items in Engineering are listed and read, and new items are written in Personal." Honest and prominent. It still does not say what I asked for: *nothing* writes a team item today, not even a command. Wave 5's Store tab said exactly that; wave 6 dropped the sentence. |
| No way to make anyone an Admin | **Fixed** | The Manage sheet (`?state=manage`, and on Engineering) has Member / Admin / Owner radios, each with one sentence, and a visibility stepper reading "0 is the default; lower bands see less". Add is live and blue. This is the single biggest change. |
| No read-role control anywhere | **Not** | The New sheet has Save in, the kind's fields and Path. There is no "Who can read", no "Who can change", not even greyed out. My round-one ask #1, recorded verbatim in WAVE4-BRIEF §2, is simply absent from wave 6. |
| Invite: masked, required-looking, unexplained | **Fixed** | `02-first-run ?state=account`: it sits under a column headed OPTIONAL, placeholder "Leave empty unless you were given one", and the hint says invites are issued by the server's operator, most servers don't need one, and a server that does will say so. |
| Parties/Stores vocabulary, JSON everywhere, Reset in the same blue as Add server | **Fixed** | Sidebar is VAULTS / GROUPS / STATUS. Raw responses are behind "Inspect response". `Reset hard state` is nowhere on either screen. One leak survives (see #6). |

## 2. Fresh transcript

I launch. `02-first-run` asks **"Who is setting you up?"** — "Someone invited me to their group" or "I'm setting up on my own", each with three numbered lines. I'm the one running this, so **Set up ›**.

**A server.** One field, the address. I type `foks.acme-corp.com`, it comes back **"Found it."** — "This Mac now recognises foks.acme-corp.com and will refuse anything else that answers to that name. Nothing about you has been sent yet." An amber PROPOSED band admits "New pin / Same as before / Advanced" isn't a fact yet. Round one this was a JSON blob.

**Your account.** Username `priya.n`, "How others on foks.acme-corp.com add you to groups." Email and Invite are under **OPTIONAL**. Create my account. **Protect it** — passphrase, with a 17-word backup phrase behind it. **Create a group**: one box, Named vs Ad-hoc with a line each, "You become its Owner. People join when you add them by username from Manage." I name it Engineering. **Done**: "Engineering exists. You are its Owner and nobody else is in it yet. Add people from Manage, by username — they need an account on foks.acme-corp.com first: send them the address, ask for the username they picked, then add them as Member, Admin or Owner."

That paragraph *is* my new-hire instructions — but it's a paragraph on my screen, not something I can send. In 1Password I press Invite and it emails them; here I retype it into Slack. Wave 5 had the button I asked for: `wave5/03-teams ?state=invite`, a five-line pre-written message with install link, server address, "create your account with username firstname.lastname", "then tell rae your username", and **Copy message**. Wave 6 left it behind.

**Adding Sam and Dana.** Sidebar → Engineering → **Manage**. The roster lists everyone with their role, then ADD PEOPLE: Username, and radios — **Member** ("Opens items at or above their visibility band"), Visibility 0, **Admin** ("Changes items and adds or removes members. Cannot change other admins or owners"), **Owner**. I add `sam.ortiz` as Admin and `dana.okafor` as Member. Two minutes. Round one this was impossible.

Under the roster: "Roles can only be lowered from here; a promotion is done by removing and re-adding… Lowering and removing are deferred this release." The **Lower** buttons are greyed. I'd rather be told than click into a failure.

**The two tokens.** New → Password → the sheet. Save in: Personal, Work (Acme), **Engineering — shared with 6 people**, Household, Homelab (greyed, "reports inactive — resume its creation first"). Engineering is selected, the button says "Create in Engineering", and the amber band admits it lands later. Then I scroll for the control I actually came for — **who can read this** — and it isn't there. I can choose *where* the token goes but not *who in there reads it*. Production and staging are the same secret to this sheet. The whole point of my goal is one dropdown that doesn't exist, and unlike round one nothing on the sheet even acknowledges the gap.

What it *does* do beautifully is read it back. Engineering's list shows `production-token` with a **3 people** chip and `staging-token` with **6 people**. Click production-token: "3 people can read this — everyone in Engineering at **Admin** or above", then sam.ortiz (Owner), rae.chen and priya.n (Admin). That is a better answer to "who can see this" than 1Password's vault list ever gave me. It just has no matching control on the way in.

**The lapsed lease** (`?state=lease`, `?state=attention`) keeps round one's excellent copy, now with Engineering greyed in the sidebar under "server check-in lapsed". Nothing pretends to work.

## 3. Remaining issues

| # | Severity | Where | What | Ask | Needs protocol? |
| - | -------- | ----- | ---- | --- | --------------- |
| 1 | blocker | `01-vault.html` `?state=new`, and New with Engineering selected | The New sheet has no read-role or write-role control at all — not live, not greyed. "Only admins can read this" cannot be expressed anywhere in the product. | "Put **Who can read** and **Who can change** on the New sheet, right under Save in. If it isn't shipping, grey them out with 'comes with team writes' — don't leave them off the screen." | no |
| 2 | major | `01-vault.html` `?state=join`; `02-first-run.html` `?state=done` | Nothing produces a message I can send a new hire. Join shows *my* username per server; Done explains it in prose. Wave 5 `03-teams ?state=invite` has the exact button and copyable text. | "Bring back **Invite someone** from wave 5 — the written message with the install link, the server address, 'username firstname.lastname', 'tell me your username', and a Copy button." | no |
| 3 | major | `01-vault.html` `?state=manage` (Engineering) | Roster rows show role only. There is no "reads N of M" per person, though WAVE4-BRIEF §3 requires it and wave 5's people table has "3 of 4". Deferred rows also lose it. | "On each person put **what they can actually read** — 'reads 3 of 4'. I hire against that number." | no |
| 4 | major | `01-vault.html` `?state=new` amber band | The band names what works today but not that *nothing at all* writes a team item today — no CLI either. Wave 5's Store tab said so plainly. | "Add the sentence 'no command writes a team item today either', so I stop looking for a workaround." | no |
| 5 | major | wave 6 has no Federation surface (see `wave5/03-teams.html ?state=federation`) | Homelab's admission into Engineering is listed as a party and shows in Attention, but there is no page listing what was admitted, from which host, at which role, active or not, or how to take it back. | "One **Federation** tab per group, or say on the party row that I can't undo it from here." | no |
| 6 | minor | `01-vault.html` `?state=password` / `?state=show`, Personal Sharing text | "sharing anything means putting it in a **team**, whose members are **parties** with roles" — the two words the rest of the app replaced with group and people. | "Say group and people here too." | no |
| 7 | minor | `02-first-run.html` `?state=existing` and `?state=done`; `01-vault.html` `?state=settings`, `?state=servers` | First run points at "Settings › Devices › Set up another Mac" and, elsewhere, at "Servers & devices". Neither screen has a Devices section or that control; Servers & devices only lists two Macs. | "Make one of those pointers real, or drop the pointer." | no |
| 8 | minor | `01-vault.html` Manage sheet, role descriptions | "Admin … Cannot change other admins or owners" appears nowhere in BRIEF, GAPS or the plan; "removing and re-adding, which rekeys the group **in one signed step**" describes two operations as one. | "Only tell me limits you can prove, and don't call remove-then-add one step." | no |

## 4. Anything wrong or missing about FOKS

Checked against BRIEF §2 and WAVE4-BRIEF §1, the wire story is far better than round one. Item metadata is exactly path / kind / version / size / read role / write role — no modified time, no owner, no history. Every write names its guard ("Save writes version 10 only if the store still holds version 9"), and the conflict sheet refuses rather than offering an overwrite. Values stay masked until Show, which says which version it read and from where. Visibility now reads "0 is the default; lower bands see less" — round one's "0 is the widest band" error is gone. Federation roles are no longer claimed to accept Admin or Owner. The two PROPOSED bands are the two WAVE4-BRIEF sanctions, and both name what is missing below the agent.

Three things still don't hold up: the Admin limitation in #8 is asserted without a source; "rekeys the group in one signed step" over-states remove-then-add; and the deferred band in #4 leaves the impression that a command line could do today what the sheet can't — wave 5 denied that outright, and wave 6 stops one sentence short.

## 5. Would I use it

I would run my team on this the day a **Who can read** control ships on the New sheet — the roster, the roles and the readable-by answers are already better than 1Password Teams; today I can set up the team and get the right people in the right roles, but I cannot create the one secret the whole exercise was about.
