# Round two — Jun, agent/platform engineer, against wave 6

**Goal:** store an Anthropic API key and a database URL, make them readable by
a `deploy-bot` identity an agent uses, understand exactly what the bot can and
cannot read, and revoke it.

Screens rendered (1280×860, no console errors): 01-vault `all`, `personal`,
`resource`, `resource`+Show, `new-resource`, `new-resource`→Engineering,
`group`, `manage`, `attention`, `lease`, `settings`, `servers`, `join`, plus
Engineering's item page, `staging-token` and `production-token` details
scrolled to the roster, and Engineering's Manage sheet; 02-first-run
`checklist-own`, `added`.

## 1. Then and now

**1. Team stores are read-only, so there is no store the bot can read that I
can write to; new items default Owner/Owner. — Partly.** *Save in* now offers
Engineering as a live target, the primary button reads **Create in
Engineering**, and one amber band carries the caveat: "Deferred — plan §5 group
B: writing into a group's store comes in a later release… Today, items in
Engineering are listed and read, and new items are written in Personal." Exactly
the honesty I asked for. Still missing is the second half of my ask: the sheet
has **no read-role control, disabled or otherwise, and never says what read
role the created item gets**, so I can aim the write at the right store and
still not know whether `deploy-bot` at Member · visibility 0 would read it.

**2. No single view says what `deploy-bot` can decrypt. — Partly.** The details
panel's SHARING section lists every party with its role chip and marks those the
read role excludes: on `/deploy/production-token`, "3 people can read this —
everyone in Engineering at **Admin** or above", with `dana.okafor`, `homelab`
and `deploy-bot` each carrying "· cannot read this". Computed, not typed. The
inverse — what *this party* can read — still does not exist.

**3. `deploy-bot` is unmanageable and nothing says how to revoke it. — Not
fixed; the revocation half went backwards.** The fixture correction is right:
`locally_manageable` is true and the roster note reads "a service account — a
user of foks.acme-corp.com like any other, so its role can be changed from
here". But **Manage has no Remove control at all** — only a disabled "Lower"
per row. Round one the app had `Remove member` and the best sentence in the
product ("Removing this party rekeys the team in one signed operation… cannot
recall copies the party already downloaded"). Wave 6 has neither. Its only
"rekeys" sits in the Manage hint about *promotion*, and stops before the part
that matters.

**4. Zero run-time story. — Not fixed.** No `foks` command, no CLI, no "the bot
needs its own account on its own host", no socket, no MCP, in either file.
Settings is now three short sections instead of thirteen YubiKey buttons, and
none of them is about a program.

**5. Vocabulary. — Partly.** Visibility reads correctly and consistently: "an
ordered band inside Member", "0 is the default; lower bands see less", one
spelling everywhere. `generation` and `party_id_hex` were not defined but
**deleted** — neither appears on any screen.

## 2. Fresh transcript

`All items`. Both my secrets are there: `anthropic-api-key`, chip `agents`,
**Readable by: only you**, v4; `DATABASE_URL` under `env/prod`, same. A column
saying "only you" beside rows saying "6 people" is the first time I have read a
vault at a glance. The key: Kind `Resource · a Secret` (the node type admitted,
not hidden), read Owner, write Owner, Show — "reads version 4 … nothing is kept
on this Mac." Sharing: "an account's store belongs to that account alone…
sharing anything means putting it in a team." Both are in the wrong place for a
bot, and the app says so.

New → Resource. **Save in** lists Personal, Work, Engineering (6 people),
Household (2 people), Homelab greyed "reports inactive — resume its creation
first". I pick Engineering; the amber band appears, the button becomes "Create
in Engineering". Then I look for the read role. There is none. Nothing says
whether this lands at Owner/Owner like a personal item — in which case
`deploy-bot` reads nothing — or at Member · 0.

Engineering, four items. `staging-token`: read `Member · visibility 0`, write
`Admin`, `deploy-bot` unmarked in the roster — it reads this.
`production-token`: read `Admin`, `deploy-bot` marked "· cannot read this".
Four items and six parties still means doing the join by hand.

Then the count stops being trustworthy. `staging-token` says "6 people can read
this", and one of the six is `homelab`, an admitted group whose admission
Attention reports "inactive, **so Homelab's members read nothing in Engineering
through it**". The chip counts a party the app elsewhere says reads nothing —
trust it and I rotate one thing too many, or leave a live path open. And
"6 people" is the wrong noun: one of the six is a team of unknown size.

Manage: six rows, roles right, `deploy-bot` correctly described as a service
account manageable from here, Add people live with Member / Admin / Owner and a
visibility stepper — a genuine fix, matching the wire. Every row's only action
is a greyed **Lower**. No Remove. The hint says "Lowering and removing are
deferred this release", and that is the whole treatment of revocation: no
consequence, no rekey, no "cannot recall copies already downloaded", no rotate
list. The fourth part of my goal is on no screen at all.

How does the agent on the CI box get the value? Nothing. `Join or create a
group` does say "an admin of the group adds you, by your username on that
server. There are no invite links" — which is how `deploy-bot` gets in — but
nothing says the bot needs its own account and device there, and no command
appears anywhere.

**Where wave 6 does not cover it.** wave5/03-teams.html has both missing
pieces: a party panel headed "What deploy-bot can read · 2 of 4", each row ✓/✗
with its reason; and a "Connect an agent" block with `foks kv get
/deploy/staging-token --team engineering` and the paragraph saying a machine
reads *as itself*, that this Mac's agent socket is not how an agent reads
values, that a machine's role is its scope, and that revoking is Remove — plus
a Remove sheet listing "Everything deploy-bot could read — rotate these
afterwards". wave1b/05-resources.html has the same CLI panel per item.

## 3. Remaining issues

| # | Severity | Where | What | Ask | Needs protocol? |
| - | - | - | - | - | - |
| 1 | blocker | `01-vault.html`, Manage sheet (Engineering) | No Remove on any party row, and no sentence that removing rekeys the group and cannot recall what was already downloaded. Revocation is a clause in a grey hint. | "Put Remove back on the party row, with the rekey sentence and the paths that party could read so I know what to rotate. If it must be disabled this release, disable it — don't delete it." | no |
| 2 | blocker | `01-vault.html`, no state | Nothing says how a program on another host obtains a value: no `foks` command, no "the bot needs its own account and device on that server", no line saying this Mac's agent is not it. | "One panel with the command the bot runs, the account it needs on its own host, and a line saying this Mac's agent socket is not it." | no |
| 3 | major | `01-vault.html`, Engineering items (`staging-token`, `bundle.tar`, `README.md`) | Readable-by counts the `homelab` admission as a reader although Attention says that admission is inactive and its members read nothing through it. | "If an admission is inactive, don't count it as a reader — strike it through with 'admission inactive, reads nothing here'." | no |
| 4 | major | `01-vault.html`, New sheet with a group chosen | No read-role control and no statement of what read role a created item gets, so I cannot tell whether the bot will read what I just wrote. | "Show the read role even if I can't change it yet, and say which parties it admits before I press Create." | no |
| 5 | major | `01-vault.html`, Manage sheet and item details | No party-centric view: "what can deploy-bot read" is assembled by opening every item in turn. | "Let me click a party and see every path it can and cannot read, with the reason on each line." | no |
| 6 | minor | `01-vault.html`, Manage sheet and Sharing panel | `party_id_hex` appears nowhere, though BRIEF §2 says it is always present. It is my only stable handle on a service account. | "Show the party id, shortened, with a copy button." | no |
| 7 | minor | `01-vault.html`, Manage sheet | `generation` appears nowhere. After a rekey I need it to reason about what an old downloaded key still opens. | "Put the generation on the party row and say what it means." | no |
| 8 | minor | `01-vault.html`, sidebar, group header, Sharing panel | "6 people" counts a `named-team` party as a person; one of the six is another team of unknown size. | "Count them separately: '5 people · 1 group'." | no |
| 9 | minor | `01-vault.html`, Manage sheet, `homelab` row | The note says "change it from Engineering's Federation page"; wave 6 has no Federation page. | "Link somewhere that exists, or say it can't be done from this window yet." | no |
| 10 | minor | `01-vault.html`, details → Inspect response | `read_role` is emitted as the display string `"Member · visibility 0"`, not the wire shape. | "Show the role the way the wire sends it." | no |

## 4. Wrong or missing about FOKS

- **Inactive admission counted as a reader** (issue 3). By the build's own
  fixture (`federation[0].active === false`) and its own Attention card the
  `homelab` party reads nothing in Engineering; `readersOf()` admits it on role
  alone — the one place wave 6 states two incompatible things about one fact.
- **"N people" for a mixed roster.** BRIEF §2 says `party_kind` is
  user | named-team; four surfaces call a named-team party a person.
- **`party_id_hex` is drawn nowhere**, against BRIEF §2's "always"; likewise
  `generation`, which "old generations still open" depends on.
- **A Federation page is referenced and does not exist** in this set, and
  **Inspect response** shows a UI string where the wire has
  `{Member:{visibility:0}}`.
- **02-first-run `added`** says "This Mac asked the server just now and
  Engineering was in the answer", in the past tense, above a correctly labelled
  Proposed band saying no such discovery operation exists.
- Checked and **correct**: visibility direction and wording match WAVE4-BRIEF
  §1; Add offers Member/Admin/Owner while Lower is deferred; `locally_manageable`
  is used in its true sense; lease copy stops reads with writes; every write
  names its version guard; team-store writes sit under one amber band.

## 5. Would I use it?

I would use wave 6 as my own vault today — it reads a store better than
anything else in this project — but not for the job I came with, because the
two things an agent platform turns on, telling a program how to fetch a value
and taking a bot's access away, are the two things it does not have.
