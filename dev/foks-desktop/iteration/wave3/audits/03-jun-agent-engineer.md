# Audit 3 — Jun, agent/platform engineer

**Persona:** Jun. Runs Claude Code and Codex with MCP servers on a CI box and a
laptop. Wants API keys kept in FOKS and handed to agents at run time, with a way
to see and revoke what an agent can reach. **Model:** Fable.
**Goal:** store an Anthropic API key and a database URL, make them readable by a
`deploy-bot` identity that an agent uses, understand exactly what the bot can
and cannot decrypt, then revoke it.

Screens rendered: app-surface `?step=items`, `?catalog=team`, `?step=party`,
`?modal=remove-party`; wave1/02 `access`, wave1/03 `new`, wave1/05 `details`;
wave2/01 `team`; wave2/02 `team`, `store`, `remove`, `add`; wave2/05
`parties-members`.

## Round two (wave 6)

Jun re-ran the deploy-bot goal against `wave6/01-vault.html` and
`wave6/02-first-run.html`; his report is
`../../wave6/audits/03-jun-round2.md`. None of his five round-one frustrations
are Fixed: three are Partly (Engineering is a live Save-in target under an
honest deferred band, but the sheet sets no read role and never says what read
role a created item gets; the Sharing block computes who can read an item, but
not what a party can read; visibility now reads correctly and consistently) and
two are Not (Manage has no Remove at all and lost round one's rekey sentence,
and nothing anywhere says how a program on another host obtains a value). His
verdict: he would use wave 6 as his own vault today, because it reads a store
better than anything else in this project, but not for the job, because telling
a program how to fetch a value and taking a bot's access away are the two things
it does not have.

## 1. Transcript — the current app

I open Items. The context line says `Profile: local    Account: personal`. The
`personal` store lists `/env/prod/DATABASE_URL`, `/latest-key`,
`/ssh/id_ed25519`. Selecting `DATABASE_URL` gives me Type `small-file`,
Version 3, Size 96, Read role `Owner`, Write role `Owner`, a `Reveal value`
button, a `Replacement secret value` field, `Replace` / `Remove`. Good: read
role and write role are on the same card as the version. The DB URL already
exists, so I use the `New account-store item` form under it for the key: Path
`/agents/anthropic-api-key`, Secret value, `Create file`. The form's grey line
says "new items default to Owner read and write roles". Fine for my own store.

Now the bot. I know FOKS well enough to know there is no per-item grant: the
bot has to be a party in a team whose store holds the item. Rail → Parties.
The first card, `Create a team store`, has Owner account alias / Local team
alias / FOKS team name / Pending team alias, and `Create named team`. I make
`deploy`. Second card, `Local team members`: Named team, Username, Member
visibility `0`, then `List roster`, `Add member`, `Resume add`, `Demote to
member`, `Remove member`, `Resume edit`. I type `deploy-bot`, leave visibility
at 0, `Add member`. Note what I could *not* choose: a role. The desktop only
adds Members with a visibility number, and the card's paragraph ("an ordered
authorization band that affects disclosure and key access") never says whether
0 is the widest band or the narrowest. `List roster` fills the JSON panel:
`source_role:{Member:{visibility:0}}, destination_role:{Member:{visibility:0}},
generation:4, locally_manageable:true`. I can read that, but "what does this
credential decrypt" is not a question a JSON blob answers.

Back to Items to put the two secrets into `deploy`'s store. I click the
`Engineering` store head (the mock's stand-in). The inspector says, twice,
"Team stores are list/read-only in this release." There is no create form for
a team store and no move from `personal`. This is where the goal dies: the
only place the bot could ever read from is the one place I cannot write to.

Pretending the items existed, I check the fixture's Engineering rows:
`/deploy/production-token` Read `Admin`, Write `Owner`; `/release/bundle.tar`
Read `Member (0)`, Write `Admin`. With the roster JSON from the other screen I
can work out that a Member(0) bot decrypts `bundle.tar` and not
`production-token`. I had to do that join in my head across two rail entries;
nothing on any screen says "deploy-bot can read: …".

Revocation: Parties → Username `deploy-bot` → `Remove member`. The sheet reads
"Remove deploy-bot from engineering? Removing this party rekeys the team in one
signed operation. It prevents future access, but cannot recall copies the party
already downloaded." That is the right sentence and it is the best sentence in
the app. What it lacks: which paths the bot could have downloaded, so I know
which upstream keys to rotate, and the generation the team moves to. If the
target is not locally manageable the card warns it "can still be refused", and
the failure I would get is one generic banner ("The last operation failed: …").

How does the agent get the value at run time? Nothing. No CLI command, no
socket path, no "install foks on the CI host", no MCP. The only "agent" the
desktop mentions is its own private local agent in the `FOKS agent unavailable`
overlay. Settings is thirteen YubiKey buttons.

## 2. The same goal against wave 1 / wave 2

**wave2/02 Teams hub got me furthest.** `Teams › Engineering › Members` is a
real table: Party (username over `party_id_hex`), Kind, Role here, Role at
source, Generation, Actions. The `deploy-bot` row: User, `Member · 0`, "same",
6, and `Change role` / `Remove` disabled with "managed where it lives"
underneath. The `Store` tab's "Who can read what" is the first place my
question is answered at all: `Admin — 1 item` with three avatars, `Member · 0
— 3 items` with six avatars including `DB`. The `Remove` sheet (I could only
open it on dana.okafor) is exactly the shape I want: red banner "Removing rekeys
the team … Keys or ciphertext retained from before removal can still open data
from those generations, so rotate every affected secret", then "Secrets
dana.okafor could read — rotate these afterwards: `/deploy/staging-token` v3
Member · 0". Two failures remain: the one party I care about has `Remove`
disabled, and "managed where it lives" never says where or how. The `Add
member` sheet offers Member (with a −/+ visibility stepper), Admin, Owner —
more than the app — but its hint "The band changes what this party can discover
in the roster and which keys and items it can open" still does not say which
direction opens more.

**wave1/02 Keychain** answers the inverse question well: select
`production-token`, `Access` tab → Read role `Admin`, Write role `Owner`, then
the roster with `Role here` / `Role at source` / `Party id`, deploy-bot marked
"managed elsewhere", `Change role…` disabled "deferred". Per-item "who can open
this" is what I would check before committing a secret.

**wave1/05 Drive** details panel: "Everything in a shared drive is visible to
every party the read role admits — here, Member · visibility 0 and above" — the
words "and above" plus wave1/04's "0 is the widest band" leave me unsure which
way the band runs.

**wave1/03 1Password** has an `API key` template in `+ New` (`/agents/`, "one
opaque value") which is the right shape, and the Vault picker greys Engineering
with "Shared with every party of Engineering (5 parties) under their roles —
DEFERRED". Honest, and useless for me.

**wave2/01 System Settings** team page lists deploy-bot as "User · generation 6
· not manageable here · 01c93d2f8b…8e57"; its `Agent` section is the only place
in the set that shows a socket path and "local protocol v2", but that is the
desktop's agent on this Mac, not my bot's.

**wave2/05 Pills** Members table adds a `Manage: local / elsewhere` pill; Add a
member is Username + Member visibility, same as the app.

Across all ten mocks, still failed: (a) writing the two secrets into a team
store; (b) revoking `deploy-bot` specifically; (c) any hint of how a CLI or MCP
process obtains a value; (d) which machines hold the bot's keys.

## 3. Ranked frustrations and things that worked

**Frustrations**

1. Team stores are read-only, so there is no store the bot can read that I can
   write to. Worse, when writes land, new items default to `Owner`/`Owner` and
   role controls are deferred — a Member(0) bot still reads nothing.
2. No single view says what `deploy-bot` can decrypt. Teams hub's Store tab
   comes closest, as avatars I have to count.
3. `deploy-bot` is "managed where it lives" / "not manageable here" in every
   mock, `Remove` disabled, and nothing says where or how to revoke it.
4. Zero run-time story: no CLI, no MCP, no "the bot needs its own account on
   its host". The desktop's "agent" is its own daemon.
5. Vocabulary never defined: visibility direction, `generation`, and three
   spellings of the same role (`Member (0)`, `Member · visibility 0`,
   `Member · 0`).

**Worked**

1. Remove-party copy everywhere: one signed rekey, cannot recall downloads;
   Teams hub adds the "rotate these afterwards" list.
2. Two roles per party, `party_id_hex` always present, a Kind column that
   admits a party can be a team (Teams hub, Keychain).
3. Read role and write role next to the version on every item, in every mock.
4. "Who can read what" (Teams hub Store) and per-item roster (Keychain Access,
   Drive details).
5. Honesty: `deferred` tags instead of fake share buttons; masked until
   revealed at an exact version; an API-key template.

## 4. Three UI changes I would ask for

1. "Put a 'What can deploy-bot read?' view on the party row and inside the
   Remove sheet: every path in this team whose read role its role-here admits,
   with version. Make it work for parties I can't manage from here — I still
   need to know what it reached before I rotate."
2. "Let me create an item in a team store and pick its read role from the
   roles the roster actually has, instead of silently defaulting to Owner. If
   that's deferred, show the disabled read-role control with the word
   `deferred` in the New item sheet so I stop looking for it."
3. "A 'Connect an agent' panel — on the party row or under Settings — with the
   exact `foks` command the bot runs to read this path, the account alias it
   needs on its own host, and one line saying this Mac's agent socket is not
   it. Text and a copy button would do."

## 5. Wrong or missing, checked against BRIEF.md and GAPS.md

- **wave2/01 team page under a lapsed lease** renders the roster with "Its
  roster below is what this Mac last saw". GAPS #11 forbids "last known"
  language and GAPS #20 says reads stop first. The roster is a read.
- **wave2/02 `Inspect response` under a lapsed lease** shows
  `{error:"lease_lapsed", …}`. GAPS #16: only `OperationFailed` with prose
  exists today; a structured code is §5 group E fiction, unlabelled.
- **Visibility direction.** wave1/04: "0 is the widest band"; wave1/05:
  "Member · visibility 0 and above". BRIEF says only "ordered band". Neither
  claim is sourced, and they may point opposite ways.
- **wave2/02 Remove sheet's readable-secrets list** treats every `Member` read
  role as readable by every Member regardless of band (`i.read.startsWith
  ("Member")`). Wrong as soon as two bands exist.
- **wave2/02 Add sheet offers Admin/Owner** from the desktop; the shipping app
  exposes Member visibility only (app-surface: "The desktop's add/demote path
  currently exposes Member visibility"). Not a protocol violation, but not
  labelled as beyond what ships.
- **`generation`** is shown in five mocks and defined in none; GAPS #12 gives
  the type only. Is it the team key generation the party joined at, or the
  party's own? I need to know for "old generations still open".
- **`deploy-bot` fixture:** `party_kind:"user"`, `locally_manageable:false`, no
  `scoped_host_id_hex`. GAPS #12 says locally_manageable "is literally whether
  the party is a local user". A same-server user drawn as unmanageable is
  either a fixture inconsistency or means "local" = an account this Mac holds,
  in which case I could manage nobody but myself. The brief should say which.
- **README:** the task pointed me at "Installing the CLI and connecting
  agents"; `/home/user/multitool/README.md` has no such section. It calls AKA
  "a credential broker for people, teams, and AI agents" and then says nothing
  about how an agent connects. The gap is upstream of the desktop.
- **app-surface Items inspector** shows `Type: small-file / symlink / file` —
  wire node types leaking where BRIEF names the kinds Secret/File/Folder/Link.
- **wave1/02 Access tab** on a personal item: "Only this account's devices —
  MacBook Pro (this Mac) and Travel Mac — hold its keys", drawn as known;
  device lists come from a per-account network call, not the item listing.
